//! Finds chapters and pages in a series folder, a chapter folder, or CBZ files.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    File(PathBuf),
    Zip { archive: PathBuf, entry: String },
}

#[derive(Debug)]
pub struct InChapter {
    pub name: String,
    pub pages: Vec<Source>,
}

#[derive(Debug)]
pub struct Series {
    pub title: String,
    pub chapters: Vec<InChapter>,
    /// Chapters that could not be read: (path, error)
    pub skipped: Vec<(String, String)>,
}

fn has_ext(name: &str, exts: &[&str]) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn is_image(name: &str) -> bool {
    has_ext(name, &["jpg", "jpeg", "png", "webp"])
}

/// Hidden files (.DS_Store, ._x.jpg) and macOS resource folders are never content.
fn is_junk(name: &str) -> bool {
    name.starts_with('.') || name == "__MACOSX"
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn stem(p: &Path) -> String {
    p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn sorted_entries(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| !is_junk(&file_name(p)))
        .collect();
    v.sort_by(|a, b| natural_cmp(&file_name(a), &file_name(b)));
    Ok(v)
}

fn dir_chapter(dir: &Path) -> anyhow::Result<InChapter> {
    let pages = sorted_entries(dir)?
        .into_iter()
        .filter(|p| p.is_file() && is_image(&file_name(p)))
        .map(Source::File)
        .collect();
    Ok(InChapter { name: file_name(dir), pages })
}

fn cbz_chapter(archive: &Path) -> anyhow::Result<InChapter> {
    let z = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
    let mut entries: Vec<String> = z
        .file_names()
        .filter(|n| is_image(n) && !n.split('/').any(is_junk))
        .map(String::from)
        .collect();
    entries.sort_by(|a, b| natural_cmp(a, b));
    let pages = entries.into_iter().map(|entry| Source::Zip { archive: archive.to_path_buf(), entry }).collect();
    Ok(InChapter { name: stem(archive), pages })
}

pub fn discover(path: &Path) -> anyhow::Result<Series> {
    let mut skipped = vec![];
    let (title, mut chapters) = if path.is_file() {
        anyhow::ensure!(has_ext(&file_name(path), &["cbz", "zip"]), "unsupported input file {}", path.display());
        (stem(path), vec![cbz_chapter(path)?])
    } else {
        // loose images form a chapter named after the folder; subfolders and CBZs are one chapter each
        let mut chapters = vec![dir_chapter(path)?];
        for p in sorted_entries(path)? {
            let ch = if p.is_dir() {
                dir_chapter(&p)
            } else if has_ext(&file_name(&p), &["cbz", "zip"]) {
                cbz_chapter(&p)
            } else {
                continue;
            };
            // one unreadable chapter must not block the rest of the series
            match ch {
                Ok(c) => chapters.push(c),
                Err(e) => skipped.push((p.display().to_string(), e.to_string())),
            }
        }
        (file_name(path), chapters)
    };
    chapters.retain(|c| !c.pages.is_empty());
    anyhow::ensure!(!chapters.is_empty(), "no images found in {}", path.display());
    Ok(Series { title, chapters, skipped })
}

pub fn read_bytes(src: &Source) -> anyhow::Result<Vec<u8>> {
    match src {
        Source::File(p) => Ok(std::fs::read(p)?),
        Source::Zip { archive, entry } => {
            // shortcut: reopens the archive per page; fine for local CBZs, cache the handle if profiling says so
            let mut z = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
            let mut f = z.by_name(entry)?;
            // the size field is untrusted; don't let a corrupt entry reserve gigabytes up front
            let mut b = Vec::with_capacity(f.size().min(64 << 20) as usize);
            std::io::Read::read_to_end(&mut f, &mut b)?;
            Ok(b)
        }
    }
}

/// "Chapter 2" < "Chapter 10": digit runs compare as numbers, the rest case-insensitively.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    fn runs(s: &str) -> Vec<&str> {
        let mut v = vec![];
        let mut start = 0;
        let bytes = s.as_bytes();
        for i in 1..=bytes.len() {
            if i == bytes.len() || bytes[i].is_ascii_digit() != bytes[i - 1].is_ascii_digit() {
                v.push(&s[start..i]);
                start = i;
            }
        }
        v
    }
    let (ra, rb) = (runs(a), runs(b));
    for (x, y) in ra.iter().zip(&rb) {
        let ord = if x.as_bytes()[0].is_ascii_digit() && y.as_bytes()[0].is_ascii_digit() {
            let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
            x.len().cmp(&y.len()).then_with(|| x.cmp(y))
        } else {
            x.to_lowercase().cmp(&y.to_lowercase())
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    ra.len().cmp(&rb.len()).then_with(|| a.cmp(b))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xtc-input-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    pub fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = std::io::Cursor::new(vec![]);
        image::GrayImage::from_pixel(w, h, image::Luma([200])).write_to(&mut b, image::ImageFormat::Png).unwrap();
        b.into_inner()
    }

    pub fn images(dir: &Path, names: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        for n in names {
            std::fs::write(dir.join(n), png(4, 6)).unwrap();
        }
    }

    pub fn cbz(path: &Path, names: &[&str]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for n in names {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(&png(4, 6)).unwrap();
        }
        z.finish().unwrap();
    }

    fn names(s: &Series) -> Vec<&str> {
        s.chapters.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["Chapter 10", "Chapter 2", "Chapter 1 - 100 Slaves"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["Chapter 1 - 100 Slaves", "Chapter 2", "Chapter 10"]);
        let mut v = vec!["10.jpg", "2.jpg", "01.jpg"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["01.jpg", "2.jpg", "10.jpg"]);
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
    }

    #[test]
    fn series_of_chapter_dirs() {
        let d = tmp_dir("series");
        images(&d.join("Chapter 10"), &["1.jpg", "2.png", "3.webp"]);
        images(&d.join("Chapter 2"), &["10.jpg", "2.jpg", "1.jpg"]);
        let s = discover(&d).unwrap();
        assert_eq!(s.title, d.file_name().unwrap().to_str().unwrap());
        assert_eq!(names(&s), ["Chapter 2", "Chapter 10"]);
        assert_eq!(s.chapters[0].pages, vec![
            Source::File(d.join("Chapter 2").join("1.jpg")),
            Source::File(d.join("Chapter 2").join("2.jpg")),
            Source::File(d.join("Chapter 2").join("10.jpg")),
        ]);
        assert_eq!(s.chapters.iter().map(|c| c.pages.len()).sum::<usize>(), 6);
    }

    #[test]
    fn single_chapter_dir() {
        let d = tmp_dir("single").join("Chapter 1 - 100 Slaves");
        images(&d, &["01.jpg", "02.JPG"]);
        let s = discover(&d).unwrap();
        assert_eq!(names(&s), ["Chapter 1 - 100 Slaves"]);
        assert_eq!(s.chapters[0].pages.len(), 2);
    }

    #[test]
    fn folder_of_cbz_is_series() {
        let d = tmp_dir("cbzs");
        cbz(&d.join("Ch 2.cbz"), &["b.png", "a.png"]);
        cbz(&d.join("Ch 1.cbz"), &["x.png"]);
        let s = discover(&d).unwrap();
        assert_eq!(names(&s), ["Ch 1", "Ch 2"]);
        assert_eq!(s.chapters[1].pages[0], Source::Zip { archive: d.join("Ch 2.cbz"), entry: "a.png".into() });
        assert_eq!(read_bytes(&s.chapters[1].pages[0]).unwrap(), png(4, 6));
    }

    #[test]
    fn single_cbz() {
        let d = tmp_dir("onecbz");
        let f = d.join("Vol 1.cbz");
        cbz(&f, &["1.png", "2.png"]);
        let s = discover(&f).unwrap();
        assert_eq!((s.title.as_str(), names(&s)), ("Vol 1", vec!["Vol 1"]));
        assert_eq!(s.chapters[0].pages.len(), 2);
    }

    #[test]
    fn junk_files_ignored() {
        let d = tmp_dir("junk");
        images(&d.join("Ch 1"), &["1.jpg"]);
        for j in ["Thumbs.db", ".DS_Store", "ComicInfo.xml", "._1.jpg"] {
            std::fs::write(d.join("Ch 1").join(j), b"junk").unwrap();
        }
        std::fs::create_dir_all(d.join(".hidden")).unwrap();
        images(&d.join("__MACOSX"), &["x.jpg"]);
        cbz(&d.join("Ch 2.cbz"), &["1.png", "__MACOSX/._1.png", "ComicInfo.xml"]);
        let s = discover(&d).unwrap();
        assert_eq!(names(&s), ["Ch 1", "Ch 2"]);
        assert_eq!(s.chapters[0].pages.len(), 1);
        assert_eq!(s.chapters[1].pages.len(), 1);
    }

    #[test]
    fn empty_input_errors() {
        let d = tmp_dir("empty");
        std::fs::create_dir_all(d.join("Ch 1")).unwrap();
        let e = discover(&d).unwrap_err().to_string();
        assert!(e.contains("no images found"), "{e}");
    }
}
