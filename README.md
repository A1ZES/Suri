# Suri (摺り)

**Press your manga and comics into e-reader-ready XTC files.**

*Suri* is the step in Japanese woodblock printing where paper is pressed onto the inked block. Suri does the same for your manga. It takes a folder of pages and turns it into a file that looks sharp on a small e-ink screen.

> **Status: alpha.** Suri is very new and changing fast. Today it outputs XTC/XTCH for Xteink readers only; more formats and devices are on the way.

<!-- TODO: screenshot of the app (docs/screenshot.png) -->

## About this project

I started Suri for two reasons: I wanted to learn Rust, and I wanted to build something useful for the e-ink community. Suri brings those two together.

The idea is simple: one free, open-source app where you can do everything, instead of jumping between a thousand different tools. Everyone can download it, use it, and change it.

Right now Suri focuses on XTC, the format used by Xteink readers. That's only the start. More output formats and devices will come as the project grows.

## Why

Getting manga onto an Xteink X3 or X4 used to take three tools: download the chapters, convert them in KCC with a custom screen size, export an EPUB, upload that EPUB to a browser converter (which chokes on big files), copy the result to the device, and start over if it looks wrong.

Suri does all of that in one step, on your own computer:

1. Drop in a manga folder or CBZ.
2. Pick your device.
3. Press **Convert**.

## Features

- **One window, no command line.** Queue several series, see a live preview of every page exactly as the e-reader will show it, and convert.
- **Fast and offline.** Pages are processed in parallel in native Rust. A 2,300-page series takes about 40 seconds, with no upload and no file-size limit.
- **Whole series at once.** Point it at a folder of chapters (for example, from a manga downloader) and you get one file with a proper chapter list.
- **Built for e-ink.** Automatic margin cropping, contrast, gamma, and a choice of dithering methods (Floyd–Steinberg, Atkinson, Sierra Lite, ordered, or none).
- **Double-page spreads your way.** Split them, rotate them, do both, or cut overlapping thirds, in manga (right-to-left) or comic (left-to-right) order.
- **1-bit XTC or 2-bit XTCH output.** The default preset matches the look of the popular x4converter output.
- **Presets.** Every option can be changed; save your setup and share it.

## Supported devices

| Device | Resolution | Output |
|---|---|---|
| Xteink X4 / X4 Pro | 480 × 800 | XTC, XTCH |
| Xteink X3 | 528 × 792 | XTC, XTCH |
| reTerminal Sticky | 480 × 800 | XTC, XTCH |

Kindle, Kobo and reMarkable screen sizes are already in [`devices.toml`](crates/core/devices.toml) for future CBZ/EPUB output. Missing a device? Add one line to that file and open a pull request.

**Input:** folders of JPG, PNG or WebP images, and CBZ/ZIP archives. A folder of chapter folders (or of CBZ files) becomes one book with chapters.

## Install

Prebuilt installers for Windows, macOS and Linux are coming soon on the [Releases](../../releases) page.

### Build from source

You need [Rust](https://rustup.rs) and [Node.js](https://nodejs.org) 20+. On Windows, the Rust installer also sets up the Visual Studio C++ build tools.

```sh
git clone https://github.com/A1ZES/suri.git
cd suri/app
npm install
npm run tauri dev      # run the app
npm run tauri build    # or build an installer
```

## Command line

Everything the app does is also available as a command-line tool, for batch jobs and scripts:

```sh
cargo run --release -p suri-cli -- convert "Mangas/Centuria"              # → Mangas/Centuria.xtch
cargo run --release -p suri-cli -- convert "Vol 1.cbz" --device x3 --depth 1
cargo run --release -p suri-cli -- convert "Mangas/Centuria" --preset my-preset.toml
cargo run --release -p suri-cli -- info Centuria.xtch                     # pages, chapters
cargo run --release -p suri-cli -- dump Centuria.xtch --page 3            # page → PNG
```

## How it works

```
folder / CBZ ─▶ decode ─▶ crop margins ─▶ tone (gamma, contrast)
            ─▶ layout (cover, spreads, resize to the screen)
            ─▶ quantize + dither (1-bit or 2-bit) ─▶ XTC / XTCH file
```

| Path | What it is |
|---|---|
| [`crates/core`](crates/core) | The engine: the XTC/XTCH format, the image pipeline and the device list |
| [`crates/cli`](crates/cli) | The `suri` command-line tool |
| [`app`](app) | The desktop app (Tauri + TypeScript), a thin layer over the engine |

Run the tests with `cargo test --workspace`.

## Credits

Suri stands on the work of these projects:

- [Kindle Comic Converter (KCC)](https://github.com/ciromattia/kcc): the image-preparation ideas and the e-reader device list (ISC license).
- [x4converter](https://x4converter.rho.sh/): the reference for XTC output quality.
- [xtcjs](https://xtcjs.app/) and cbz2xtc: inspiration for the conversion options.
- [CrossPoint Reader](https://github.com/crosspoint-reader/crosspoint-reader): its XTC reader is the format reference Suri writes against.

Suri is not affiliated with Xteink or any of the projects above.

## License

[MIT](LICENSE). Free to use, modify and share.
