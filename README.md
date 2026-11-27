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

Converting manga for an Xteink reader used to take three tools and a lot of retries. With Suri, you drop in a folder, pick your device, and press **Convert**.

## Features

- **Live preview:** see each page exactly as the e-reader will show it.
- **Fast and offline:** a 2,300-page series converts in about 40 seconds, with no upload.
- **Whole series:** a folder of chapters becomes one book with a chapter list.
- **Made for e-ink:** auto-crop, contrast, gamma and five dithering methods.
- **Spreads:** split, rotate or both, in manga or comic reading order.
- **Fully customizable:** change any option and save it as a preset.

## Supported devices

| Device | Resolution | Output |
|---|---|---|
| Xteink X4 / X4 Pro | 480 × 800 | XTC, XTCH |
| Xteink X3 | 528 × 792 | XTC, XTCH |
| reTerminal Sticky | 480 × 800 | XTC, XTCH |

Kindle, Kobo and reMarkable screen sizes are already in [`devices.toml`](crates/core/devices.toml) for future CBZ/EPUB output. Missing a device? Add one line to that file and open a pull request.

**Input:** folders of JPG, PNG or WebP images, and CBZ/ZIP archives. A folder of chapter folders (or of CBZ files) becomes one book with chapters.

## Install

Prebuilt installers for Windows, macOS and Linux are coming soon.

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

## License

[MIT](LICENSE). Free to use, modify and share.

Suri is an independent project, not affiliated with Xteink.
