//! Every user-facing conversion option, presets, and the device table.

use crate::pack::Depth;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub width: u16,
    pub height: u16,
    pub xtc: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Spread {
    Split,
    Rotate,
    Both,
    Thirds,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dither {
    FloydSteinberg,
    Atkinson,
    SierraLite,
    Ordered,
    None,
}

/// None = untouched; Light = KCC autocontrast (min..max stretch); higher levels clip more.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Contrast {
    None,
    Light,
    Medium,
    Strong,
    Maximum,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Overview {
    None,
    Portrait,
    Landscape,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub width: u16,
    pub height: u16,
    pub depth: Depth,
    pub rtl: bool,
    pub title: Option<String>,
    pub author: String,
    pub landscape: bool,
    pub flip_landscape_cw: bool,
    pub first_page_cover: bool,
    pub keep_cover_portrait: bool,
    pub cover_fill: bool,
    pub spread: Spread,
    pub rotate_right: bool,
    pub overview: Overview,
    pub stretch: bool,
    pub auto_crop: bool,
    pub crop_power: f32,
    pub preserve_margin_pct: f32,
    pub margin_crop_h_pct: f32,
    pub margin_crop_v_pct: f32,
    pub inter_panel_crop: bool,
    /// None = auto (KCC: 1.0, no correction)
    pub gamma: Option<f32>,
    pub contrast: Contrast,
    pub extreme_black: bool,
    pub invert: bool,
    pub dither: Dither,
    /// 0.0..=1.0
    pub dither_strength: f32,
}

impl Default for Settings {
    fn default() -> Self {
        preset_x4converter_look()
    }
}

pub fn devices() -> Vec<Device> {
    #[derive(Deserialize)]
    struct Table {
        devices: Vec<Device>,
    }
    toml::from_str::<Table>(include_str!("../devices.toml")).expect("devices.toml is valid").devices
}

/// The owner's KCC → x4converter setup, the default. Tuned against the reference files.
pub fn preset_x4converter_look() -> Settings {
    Settings {
        width: 480,
        height: 800,
        depth: Depth::Two,
        rtl: true,
        title: None,
        author: String::new(),
        landscape: false,
        flip_landscape_cw: false,
        first_page_cover: true,
        keep_cover_portrait: true,
        cover_fill: false,
        spread: Spread::Both,
        rotate_right: true,
        overview: Overview::None,
        stretch: true,
        auto_crop: true,
        crop_power: 1.0,
        preserve_margin_pct: 0.0,
        margin_crop_h_pct: 0.0,
        margin_crop_v_pct: 0.0,
        inter_panel_crop: false,
        gamma: None,
        contrast: Contrast::Light,
        extreme_black: false,
        invert: false,
        dither: Dither::None,
        dither_strength: 1.0,
    }
}

pub fn load_preset(path: &Path) -> anyhow::Result<Settings> {
    let text = std::fs::read_to_string(path)?;
    toml::from_str(&text).map_err(|e| anyhow::anyhow!("preset {}: {e}", path.display()))
}

pub fn save_preset(s: &Settings, path: &Path) -> anyhow::Result<()> {
    Ok(std::fs::write(path, toml::to_string_pretty(s)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_contains_xteink() {
        let d = devices();
        let get = |id: &str| d.iter().find(|x| x.id == id).cloned().unwrap();
        assert_eq!((get("x4").width, get("x4").height, get("x4").xtc), (480, 800, true));
        assert_eq!((get("x3").width, get("x3").height), (528, 792));
        assert!(d.len() > 30);
        assert!(d.iter().all(|x| x.width > 0 && x.height > 0));
    }

    #[test]
    fn default_is_owner_x4_setup() {
        let s = Settings::default();
        assert_eq!((s.width, s.height, s.depth, s.rtl), (480, 800, Depth::Two, true));
        assert_eq!((s.spread, s.rotate_right, s.auto_crop, s.stretch), (Spread::Both, true, true, true));
        assert_eq!((s.gamma, s.contrast), (None, Contrast::Light));
        // compare harness: x4converter's 2-bit grays are flat, undithered
        assert_eq!(s.dither, Dither::None);
    }

    #[test]
    fn preset_round_trips_through_toml() {
        let p = std::env::temp_dir().join(format!("xtc-preset-{}.toml", std::process::id()));
        let mut s = Settings::default();
        s.dither = Dither::Atkinson;
        s.gamma = Some(1.8);
        s.title = Some("Centuria".into());
        save_preset(&s, &p).unwrap();
        assert_eq!(load_preset(&p).unwrap(), s);
    }

    #[test]
    fn partial_preset_fills_defaults() {
        let p = std::env::temp_dir().join(format!("xtc-partial-{}.toml", std::process::id()));
        std::fs::write(&p, "depth = \"One\"\n").unwrap();
        let s = load_preset(&p).unwrap();
        assert_eq!(s.depth, Depth::One);
        assert_eq!(s.spread, Settings::default().spread);
    }

    #[test]
    fn bad_preset_is_error() {
        let p = std::env::temp_dir().join(format!("xtc-bad-{}.toml", std::process::id()));
        std::fs::write(&p, "dither = \"Sparkles\"\n").unwrap();
        assert!(load_preset(&p).is_err());
    }
}
