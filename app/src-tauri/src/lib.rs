//! Tauri commands: thin wrappers over xtc-core.

use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use xtc_core::convert::{convert as run_convert, default_output, render};
use xtc_core::input::discover;
use xtc_core::pack::to_gray;
use xtc_core::settings::{self, Device, Settings};

/// Errors cross the IPC boundary as plain strings.
type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
struct ScanInfo {
    title: String,
    chapters: usize,
    images: usize,
    skipped: usize,
}

#[derive(Serialize, Clone)]
struct Progress {
    done: usize,
    total: usize,
}

#[derive(Serialize)]
struct ConvertResult {
    out: String,
    pages: u32,
    skipped: Vec<String>,
}

#[tauri::command]
fn devices() -> Vec<Device> {
    settings::devices().into_iter().filter(|d| d.xtc).collect()
}

#[tauri::command]
fn default_settings() -> Settings {
    Settings::default()
}

#[tauri::command]
async fn scan(path: PathBuf) -> Res<ScanInfo> {
    tauri::async_runtime::spawn_blocking(move || {
        let s = discover(&path).map_err(err)?;
        Ok(ScanInfo {
            title: s.title,
            chapters: s.chapters.len(),
            images: s.chapters.iter().map(|c| c.pages.len()).sum(),
            skipped: s.skipped.len(),
        })
    })
    .await
    .map_err(err)?
}

/// PNG bytes of every output page made from source image `index` (clamped to the series).
#[tauri::command]
async fn preview(path: PathBuf, index: usize, settings: Settings) -> Res<Vec<Vec<u8>>> {
    tauri::async_runtime::spawn_blocking(move || {
        // shortcut: rescans the folder per preview; cache the source list if large series feel slow
        let series = discover(&path).map_err(err)?;
        let sources: Vec<_> = series.chapters.iter().flat_map(|c| &c.pages).collect();
        let i = index.min(sources.len() - 1);
        let pages = render(sources[i], i == 0, &settings).map_err(err)?;
        pages
            .iter()
            .map(|l| {
                let mut png = std::io::Cursor::new(vec![]);
                to_gray(l).write_to(&mut png, image::ImageFormat::Png).map_err(err)?;
                Ok(png.into_inner())
            })
            .collect()
    })
    .await
    .map_err(err)?
}

#[tauri::command]
async fn convert(app: AppHandle, path: PathBuf, settings: Settings, out_dir: Option<PathBuf>) -> Res<ConvertResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let next_to_source = default_output(&path, settings.depth);
        let out = match out_dir {
            Some(dir) => dir.join(next_to_source.file_name().unwrap_or_default()),
            None => next_to_source,
        };
        let report = run_convert(&path, &out, &settings, &|done, total| {
            let _ = app.emit("progress", Progress { done, total });
        })
        .map_err(err)?;
        Ok(ConvertResult {
            out: out.display().to_string(),
            pages: report.pages_written,
            skipped: report.skipped.into_iter().map(|(src, e)| format!("{src}: {e}")).collect(),
        })
    })
    .await
    .map_err(err)?
}

#[tauri::command]
fn load_preset(path: PathBuf) -> Res<Settings> {
    settings::load_preset(&path).map_err(err)
}

#[tauri::command]
fn save_preset(path: PathBuf, settings: Settings) -> Res<()> {
    settings::save_preset(&settings, &path).map_err(err)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![devices, default_settings, scan, preview, convert, load_preset, save_preset])
        .run(tauri::generate_context!())
        .expect("error while running the app");
}
