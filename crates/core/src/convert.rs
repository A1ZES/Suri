//! Series folder → XTC/XTCH file.

use crate::container::{Chapter, Meta, XtcWriter};
use crate::dither::quantize;
use crate::input::{discover, read_bytes, Source};
use crate::layout::layout;
use crate::pack::{pack, page_size, Depth, Levels};
use crate::settings::Settings;
use crate::tone::{crop, decode_gray, tone};
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Report {
    pub pages_written: u32,
    /// (source, error)
    pub skipped: Vec<(String, String)>,
}

fn describe(src: &Source) -> String {
    match src {
        Source::File(p) => p.display().to_string(),
        Source::Zip { archive, entry } => format!("{}:{entry}", archive.display()),
    }
}

/// One source image → its output pages, exactly as the e-reader will show them.
pub fn render(src: &Source, is_cover: bool, s: &Settings) -> anyhow::Result<Vec<Levels>> {
    let mut img = crop(&decode_gray(&read_bytes(src)?)?, s);
    tone(&mut img, s);
    Ok(layout(img, is_cover, s)
        .iter()
        .map(|p| quantize(p, s.depth, s.dither, s.dither_strength.clamp(0.0, 1.0)))
        .collect())
}

fn process(src: &Source, is_cover: bool, s: &Settings) -> anyhow::Result<Vec<Vec<u8>>> {
    Ok(render(src, is_cover, s)?.iter().map(pack).collect())
}

/// `<parent>/<folder name or file stem>.xtc|.xtch` (no `with_extension`: "Vol. 2" must stay intact).
pub fn default_output(input: &Path, depth: Depth) -> PathBuf {
    let ext = if depth == Depth::One { "xtc" } else { "xtch" };
    let name = if input.is_file() { input.file_stem() } else { input.file_name() };
    let name = name.map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into());
    input.parent().unwrap_or(Path::new(".")).join(format!("{name}.{ext}"))
}

fn sibling(out: &Path, suffix: &str) -> PathBuf {
    let mut p = out.as_os_str().to_owned();
    p.push(suffix);
    PathBuf::from(p)
}

/// Converts in two passes so memory stays bounded by one chapter: pages go to a temp file
/// (page count is only known after layout), then header + tables + pages go to `<out>.part`,
/// which is renamed to `out` only on success.
pub fn convert(input: &Path, out: &Path, s: &Settings, progress: &(dyn Fn(usize, usize) + Sync)) -> anyhow::Result<Report> {
    let series = discover(input)?;
    let (pages_tmp, part) = (sibling(out, ".pages"), sibling(out, ".part"));
    let result = (|| {
        let total: usize = series.chapters.iter().map(|c| c.pages.len()).sum();
        let done = AtomicUsize::new(0);
        let mut tmp = BufWriter::new(File::create(&pages_tmp)?);
        let (mut written, mut skipped, mut chapters) = (0u32, series.skipped.clone(), vec![]);
        for (ci, ch) in series.chapters.iter().enumerate() {
            let results: Vec<_> = ch
                .pages
                .par_iter()
                .enumerate()
                .map(|(pi, src)| {
                    // a panic in one page (decoder bug, odd input) must not kill the series
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process(src, ci == 0 && pi == 0, s)))
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("crashed while processing this image")));
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                    r
                })
                .collect();
            let start = written + 1;
            for (src, r) in ch.pages.iter().zip(results) {
                match r {
                    Ok(pages) => {
                        for p in pages {
                            tmp.write_all(&p)?;
                            written += 1;
                        }
                    }
                    Err(e) => skipped.push((describe(src), e.to_string())),
                }
            }
            if written >= start {
                anyhow::ensure!(written <= u16::MAX as u32, "more than 65535 pages; split the series into volumes");
                chapters.push(Chapter { name: ch.name.clone(), start: start as u16, end: written as u16 });
            }
        }
        drop(tmp);
        anyhow::ensure!(written > 0, "no pages could be converted");
        let meta = Meta { title: s.title.clone().unwrap_or_else(|| series.title.clone()), author: s.author.clone() };
        let mut w = XtcWriter::new(BufWriter::new(File::create(&part)?), s.depth, s.width, s.height, written, &meta, &chapters)?;
        let mut pages = BufReader::new(File::open(&pages_tmp)?);
        let mut buf = vec![0u8; page_size(s.width, s.height, s.depth)];
        for _ in 0..written {
            pages.read_exact(&mut buf)?;
            w.push_packed(&buf)?;
        }
        w.finish()?.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        std::fs::rename(&part, out)?;
        Ok(Report { pages_written: written, skipped })
    })();
    let _ = std::fs::remove_file(&pages_tmp);
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::read_xtc;
    use crate::input::tests::tmp_dir;
    use image::{GrayImage, Luma};

    fn img(path: &Path, w: u32, h: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        GrayImage::from_fn(w, h, |x, _| Luma([if x % 7 == 0 { 0 } else { 255 }])).save(path).unwrap();
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        let d = tmp_dir(name).join("Series");
        for n in ["1.png", "2.png", "3.png"] {
            img(&d.join("Chapter 1").join(n), 40, 60);
        }
        img(&d.join("Chapter 2").join("1.png"), 40, 60);
        img(&d.join("Chapter 2").join("2.png"), 120, 60); // wide spread
        img(&d.join("Chapter 2").join("3.png"), 40, 60);
        d
    }

    #[test]
    fn converts_synthetic_series() {
        let d = fixture("conv");
        let out = d.parent().unwrap().join("Series.xtch");
        let r = convert(&d, &out, &Settings::default(), &|_, _| {}).unwrap();
        // default spread = Both: the wide page becomes rotated + 2 halves
        assert_eq!((r.pages_written, r.skipped.len()), (8, 0));
        let f = read_xtc(&out).unwrap();
        assert_eq!((f.page_count, f.title.as_str()), (8, "Series"));
        let ch: Vec<_> = f.chapters.iter().map(|c| (c.name.as_str(), c.start, c.end)).collect();
        assert_eq!(ch, [("Chapter 1", 1, 3), ("Chapter 2", 4, 8)]);
        let p = crate::container::read_page(&out, 7).unwrap();
        assert_eq!((p.width, p.height), (480, 800));
        assert!(!out.with_extension("xtch.part").exists());
    }

    #[test]
    fn corrupt_image_is_skipped() {
        let d = fixture("corrupt");
        std::fs::write(d.join("Chapter 1").join("2.png"), b"not a png").unwrap();
        let out = d.parent().unwrap().join("Series.xtch");
        let r = convert(&d, &out, &Settings::default(), &|_, _| {}).unwrap();
        assert_eq!(r.skipped.len(), 1);
        assert!(r.skipped[0].0.contains("2.png"));
        assert_eq!(read_xtc(&out).unwrap().page_count, 7);
    }

    #[test]
    fn failed_run_leaves_no_output() {
        let d = fixture("fail");
        let out = d.parent().unwrap().join("missing-dir").join("Series.xtch");
        assert!(convert(&d, &out, &Settings::default(), &|_, _| {}).is_err());
        assert!(!out.exists());
    }

    #[test]
    fn panic_in_a_page_is_contained_and_cleaned_up() {
        let d = fixture("panic");
        let out = d.parent().unwrap().join("Series.xtch");
        let mut s = Settings::default();
        s.margin_crop_h_pct = 100.0; // used to panic inside crop
        let r = std::panic::catch_unwind(|| convert(&d, &out, &s, &|_, _| {}));
        assert!(r.is_ok(), "convert must not panic");
        assert!(!sibling(&out, ".pages").exists() && !sibling(&out, ".part").exists());
    }

    #[test]
    fn corrupt_cbz_chapter_is_skipped() {
        let d = fixture("badcbz");
        std::fs::write(d.join("Chapter 3.cbz"), b"not a zip").unwrap();
        let out = d.parent().unwrap().join("Series.xtch");
        let r = convert(&d, &out, &Settings::default(), &|_, _| {}).unwrap();
        assert_eq!(r.pages_written, 8);
        assert!(r.skipped.iter().any(|(src, _)| src.contains("Chapter 3.cbz")));
    }

    #[test]
    fn default_output_keeps_dots_in_folder_names() {
        let p = Path::new(r"C:\Mangas\Vol. 2");
        assert_eq!(default_output(p, crate::pack::Depth::Two), Path::new(r"C:\Mangas\Vol. 2.xtch"));
    }

    #[test]
    fn default_output_for_cbz_uses_stem() {
        let d = tmp_dir("defout");
        let f = d.join("Vol 1.cbz");
        std::fs::write(&f, b"").unwrap();
        assert_eq!(default_output(&f, crate::pack::Depth::One), d.join("Vol 1.xtc"));
    }

    #[test]
    fn render_returns_screen_sized_levels() {
        let d = fixture("render");
        let src = Source::File(d.join("Chapter 2").join("2.png")); // wide
        let pages = render(&src, false, &Settings::default()).unwrap();
        assert_eq!(pages.len(), 3);
        assert!(pages.iter().all(|p| (p.width, p.height) == (480, 800)));
    }

    #[test]
    fn progress_reaches_total() {
        let d = fixture("progress");
        let out = d.parent().unwrap().join("Series.xtc");
        let last = std::sync::Mutex::new((0, 0));
        convert(&d, &out, &Settings::default(), &|i, n| *last.lock().unwrap() = (i, n)).unwrap();
        assert_eq!(*last.lock().unwrap(), (6, 6));
    }
}
