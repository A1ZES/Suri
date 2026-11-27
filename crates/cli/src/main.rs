use clap::{Parser, Subcommand};
use std::path::PathBuf;
use xtc_core::container::{read_page, read_xtc};
use xtc_core::convert::{convert, default_output};
use xtc_core::pack::{to_gray, Depth};
use xtc_core::settings::{devices, load_preset, Settings};

#[derive(Parser)]
#[command(name = "xtc", about = "Manga to XTC/XTCH converter for Xteink e-readers", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show bit depth, page count, title, author and chapters of an XTC/XTCH file
    Info { file: PathBuf },
    /// Convert a series folder, chapter folder or CBZ to XTC/XTCH
    Convert {
        input: PathBuf,
        /// Output file (default: next to the input, named after it)
        #[arg(long)]
        out: Option<PathBuf>,
        /// Device id from the device list (x4, x3, sticky, ...)
        #[arg(long)]
        device: Option<String>,
        /// Preset TOML file (default: built-in x4converter look)
        #[arg(long)]
        preset: Option<PathBuf>,
        /// Bit depth: 1 (XTC) or 2 (XTCH)
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..=2))]
        depth: Option<u8>,
    },
    /// Put the same pages of two XTC files side by side and score their difference
    Compare {
        a: PathBuf,
        b: PathBuf,
        /// 1-based page numbers, comma separated
        #[arg(long, value_delimiter = ',', required = true)]
        pages: Vec<u16>,
        #[arg(long, default_value = ".")]
        out: PathBuf,
    },
    /// Render one page of an XTC/XTCH file to PNG
    Dump {
        file: PathBuf,
        /// 1-based page number
        #[arg(long)]
        page: u16,
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Info { file } => {
            let f = read_xtc(&file)?;
            let bits = if f.depth == Depth::One { 1 } else { 2 };
            println!("{bits}-bit, {} pages, \"{}\", author \"{}\"", f.page_count, f.title, f.author);
            for c in &f.chapters {
                println!("  {:>5}-{:<5} {}", c.start, c.end, c.name);
            }
        }
        Cmd::Convert { input, out, device, preset, depth } => {
            let has_preset = preset.is_some();
            let mut s = match preset {
                Some(p) => load_preset(&p)?,
                None => Settings::default(),
            };
            if let Some(id) = device {
                let all = devices();
                let d = all.iter().find(|d| d.id.eq_ignore_ascii_case(&id)).ok_or_else(|| {
                    let ids: Vec<_> = all.iter().filter(|d| d.xtc).map(|d| d.id.as_str()).collect();
                    anyhow::anyhow!("unknown device '{id}'; XTC devices: {}", ids.join(", "))
                })?;
                anyhow::ensure!(d.xtc, "{} does not support XTC output yet", d.name);
                (s.width, s.height) = (d.width, d.height);
            }
            if let Some(d) = depth {
                s.depth = if d == 1 { Depth::One } else { Depth::Two };
            }
            // the built-in preset is tuned for 2-bit; 1-bit without dithering would posterize badly
            if s.depth == Depth::One && !has_preset {
                s.dither = xtc_core::settings::Dither::Atkinson;
            }
            let out = out.unwrap_or_else(|| default_output(&input, s.depth));
            let t = std::time::Instant::now();
            let r = convert(&input, &out, &s, &|i, n| eprint!("\rimage {i}/{n}"))?;
            eprintln!();
            for (src, err) in &r.skipped {
                eprintln!("skipped {src}: {err}");
            }
            println!("wrote {} ({} pages) in {:.1}s", out.display(), r.pages_written, t.elapsed().as_secs_f32());
        }
        Cmd::Compare { a, b, pages, out } => {
            std::fs::create_dir_all(&out)?;
            for n in pages {
                anyhow::ensure!(n >= 1, "pages are numbered from 1");
                let (pa, pb) = (to_gray(&read_page(&a, n - 1)?), to_gray(&read_page(&b, n - 1)?));
                let (w, h) = (pa.width() + 8 + pb.width(), pa.height().max(pb.height()));
                let mut canvas = image::GrayImage::from_pixel(w, h, image::Luma([128]));
                image::imageops::replace(&mut canvas, &pa, 0, 0);
                image::imageops::replace(&mut canvas, &pb, pa.width() as i64 + 8, 0);
                let path = out.join(format!("page-{n}.png"));
                canvas.save(&path)?;
                let diff = if pa.dimensions() == pb.dimensions() {
                    let sum: u64 = pa.pixels().zip(pb.pixels()).map(|(x, y)| x[0].abs_diff(y[0]) as u64).sum();
                    format!("{:.1}", sum as f64 / pa.len() as f64)
                } else {
                    "n/a (sizes differ)".into()
                };
                println!("page {n}: mean difference {diff} -> {}", path.display());
            }
        }
        Cmd::Dump { file, page, out } => {
            anyhow::ensure!(page >= 1, "pages are numbered from 1");
            let out = out.unwrap_or_else(|| PathBuf::from(format!("page-{page}.png")));
            to_gray(&read_page(&file, page - 1)?).save(&out)?;
            println!("wrote {}", out.display());
        }
    }
    Ok(())
}
