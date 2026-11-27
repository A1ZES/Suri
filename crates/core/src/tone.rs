//! Decode to grayscale, crop margins, adjust tone.

use crate::settings::{Contrast, Settings};
use image::{GrayImage, Luma};

/// Any supported image → 8-bit gray; transparent pixels become white.
pub fn decode_gray(bytes: &[u8]) -> anyhow::Result<GrayImage> {
    let la = image::load_from_memory(bytes)?.to_luma_alpha8();
    Ok(GrayImage::from_fn(la.width(), la.height(), |x, y| {
        let [v, a] = la.get_pixel(x, y).0;
        let (v, a) = (v as u32, a as u32);
        Luma([((v * a + 255 * (255 - a) + 127) / 255) as u8])
    }))
}

/// Manual margin crop (% per side), then auto-crop to content.
// shortcut: `inter_panel_crop` is not implemented yet (needs KCC-style panel detection); hide it in the UI until it is
pub fn crop(img: &GrayImage, s: &Settings) -> GrayImage {
    let (w, h) = img.dimensions();
    // ≥ 50% per side would overlap; keep at least 10% of the page
    let mx = (w as f32 * s.margin_crop_h_pct.clamp(0.0, 45.0) / 100.0) as u32;
    let my = (h as f32 * s.margin_crop_v_pct.clamp(0.0, 45.0) / 100.0) as u32;
    let (mut x0, mut y0, mut x1, mut y1) = (mx, my, w.saturating_sub(mx).max(mx + 1), h.saturating_sub(my).max(my + 1));
    if s.auto_crop {
        if let Some(b) = content_box(img, (x0, y0, x1, y1), s.crop_power) {
            (x0, y0, x1, y1) = grow(b, s.preserve_margin_pct, (x0, y0, x1, y1), (w, h));
        }
    }
    image::imageops::crop_imm(img, x0, y0, x1 - x0, y1 - y0).to_image()
}

type Rect = (u32, u32, u32, u32); // x0, y0, x1 (exclusive), y1 (exclusive)

/// Bounding box of dark content; rows/cols with fewer than max(2, 0.5%) dark pixels are speckle.
fn content_box(img: &GrayImage, (x0, y0, x1, y1): Rect, power: f32) -> Option<Rect> {
    let t = (255.0 - 16.0 * power).clamp(1.0, 254.0) as u8;
    let (w, h) = (x1 - x0, y1 - y0);
    let mut rows = vec![0u32; h as usize];
    let mut cols = vec![0u32; w as usize];
    for y in y0..y1 {
        for x in x0..x1 {
            if img.get_pixel(x, y)[0] < t {
                rows[(y - y0) as usize] += 1;
                cols[(x - x0) as usize] += 1;
            }
        }
    }
    let span = |v: &[u32], min: u32| {
        let first = v.iter().position(|&c| c >= min)?;
        let last = v.iter().rposition(|&c| c >= min)?;
        Some((first as u32, last as u32 + 1))
    };
    let (cx0, cx1) = span(&cols, (h / 200).max(2))?;
    let (cy0, cy1) = span(&rows, (w / 200).max(2))?;
    Some((x0 + cx0, y0 + cy0, x0 + cx1, y0 + cy1))
}

/// Add the preserved margin, and never shrink a side below half the page (keeps sparse pages readable).
fn grow((x0, y0, x1, y1): Rect, margin_pct: f32, limit: Rect, (w, h): (u32, u32)) -> Rect {
    let axis = |a: u32, b: u32, lo: u32, hi: u32, full: u32| {
        let m = (full as f32 * margin_pct / 100.0) as u32;
        let (mut a, mut b) = (a.saturating_sub(m).max(lo), (b + m).min(hi));
        let min = (full / 2).min(hi - lo);
        if b - a < min {
            let extra = min - (b - a);
            a = a.saturating_sub(extra / 2).max(lo);
            b = (a + min).min(hi);
            a = b - min;
        }
        (a, b)
    };
    let (x0, x1) = axis(x0, x1, limit.0, limit.2, w);
    let (y0, y1) = axis(y0, y1, limit.1, limit.3, h);
    (x0, y0, x1, y1)
}

/// Extreme black → gamma → contrast stretch → invert.
pub fn tone(img: &mut GrayImage, s: &Settings) {
    let mut lut: [u8; 256] = std::array::from_fn(|v| v as u8);
    if s.extreme_black {
        lut.iter_mut().filter(|v| **v <= 64).for_each(|v| *v = 0);
    }
    let g = s.gamma.unwrap_or(1.0); // auto = KCC default 1.0
    if g != 1.0 {
        for v in lut.iter_mut() {
            *v = (255.0 * (*v as f32 / 255.0).powf(g)).round() as u8;
        }
    }
    let clip = match s.contrast {
        Contrast::None => None,
        Contrast::Light => Some(0.0),
        Contrast::Medium => Some(0.5),
        Contrast::Strong => Some(1.0),
        Contrast::Maximum => Some(2.0),
    };
    if let Some(pct) = clip {
        let mut hist = [0u64; 256];
        img.pixels().for_each(|p| hist[lut[p[0] as usize] as usize] += 1);
        let (min, max) = (hist.iter().position(|&c| c > 0), hist.iter().rposition(|&c| c > 0));
        // KCC: a range under 159 is probably intentional, leave it
        if let (Some(min), Some(max)) = (min, max) {
            if max - min >= 159 {
                let cut = (img.len() as f64 * pct / 100.0) as u64;
                let lo = percentile(&hist, cut, false).max(min);
                let hi = percentile(&hist, cut, true).min(max);
                if hi > lo {
                    for v in lut.iter_mut() {
                        let c = (*v as i32).clamp(lo as i32, hi as i32) - lo as i32;
                        *v = ((c * 255 + (hi - lo) as i32 / 2) / (hi - lo) as i32) as u8;
                    }
                }
            }
        }
    }
    if s.invert {
        lut.iter_mut().for_each(|v| *v = 255 - *v);
    }
    img.pixels_mut().for_each(|p| p[0] = lut[p[0] as usize]);
}

/// First value whose cumulative count (from the dark end, or the bright end if `from_top`) exceeds `cut`.
fn percentile(hist: &[u64; 256], cut: u64, from_top: bool) -> usize {
    let mut acc = 0;
    for i in 0..256 {
        let v = if from_top { 255 - i } else { i };
        acc += hist[v];
        if acc > cut {
            return v;
        }
    }
    if from_top { 0 } else { 255 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, ImageFormat, Rgba};
    use std::io::Cursor;

    fn encode<P: image::PixelWithColorType>(img: &ImageBuffer<P, Vec<P::Subpixel>>) -> Vec<u8>
    where
        [P::Subpixel]: image::EncodableLayout,
    {
        let mut b = Cursor::new(vec![]);
        img.write_to(&mut b, ImageFormat::Png).unwrap();
        b.into_inner()
    }

    fn base() -> Settings {
        let mut s = Settings::default();
        s.auto_crop = false;
        s.contrast = Contrast::None;
        s
    }

    fn page_with_square(w: u32, h: u32, sq: u32) -> GrayImage {
        let (x0, y0) = ((w - sq) / 2, (h - sq) / 2);
        GrayImage::from_fn(w, h, |x, y| {
            Luma([if (x0..x0 + sq).contains(&x) && (y0..y0 + sq).contains(&y) { 0 } else { 255 }])
        })
    }

    fn has_black(img: &GrayImage) -> usize {
        img.pixels().filter(|p| p[0] < 128).count()
    }

    #[test]
    fn decodes_rgba_png_over_white() {
        let img = ImageBuffer::from_fn(2, 1, |x, _| if x == 0 { Rgba([0u8, 0, 0, 0]) } else { Rgba([0, 0, 0, 255]) });
        let g = decode_gray(&encode(&img)).unwrap();
        assert_eq!((g.get_pixel(0, 0)[0], g.get_pixel(1, 0)[0]), (255, 0));
    }

    #[test]
    fn decodes_16bit_png() {
        let img: ImageBuffer<Luma<u16>, Vec<u16>> = ImageBuffer::from_pixel(3, 3, Luma([65535]));
        assert_eq!(decode_gray(&encode(&img)).unwrap().get_pixel(1, 1)[0], 255);
    }

    #[test]
    fn decode_garbage_is_error() {
        assert!(decode_gray(b"definitely not an image").is_err());
    }

    #[test]
    fn autocrop_removes_white_border() {
        let mut s = base();
        s.auto_crop = true;
        let c = crop(&page_with_square(100, 100, 20), &s);
        assert!(c.width() <= 60 && c.height() <= 60, "{}x{}", c.width(), c.height());
        assert!(c.width() >= 50 && c.height() >= 50);
        assert_eq!(has_black(&c), 400);
    }

    #[test]
    fn autocrop_ignores_speckle() {
        let mut s = base();
        s.auto_crop = true;
        let mut img = page_with_square(200, 200, 120);
        img.put_pixel(1, 1, Luma([0]));
        let c = crop(&img, &s);
        assert_eq!((c.width(), c.height()), (120, 120));
    }

    #[test]
    fn autocrop_blank_page_unchanged() {
        let mut s = base();
        s.auto_crop = true;
        let c = crop(&GrayImage::from_pixel(50, 80, Luma([255])), &s);
        assert_eq!((c.width(), c.height()), (50, 80));
    }

    #[test]
    fn manual_crop_pct() {
        let mut s = base();
        s.margin_crop_h_pct = 10.0;
        s.margin_crop_v_pct = 10.0;
        let c = crop(&GrayImage::from_pixel(100, 100, Luma([255])), &s);
        assert_eq!((c.width(), c.height()), (80, 80));
    }

    #[test]
    fn huge_margin_crop_is_clamped() {
        let mut s = base();
        s.auto_crop = true;
        s.margin_crop_h_pct = 100.0;
        s.margin_crop_v_pct = 60.0;
        let c = crop(&GrayImage::from_pixel(100, 150, Luma([255])), &s);
        assert_eq!((c.width(), c.height()), (10, 16)); // clamped to 45% per side (150*0.45 floors to 67)
    }

    #[test]
    fn invert_flips() {
        let mut s = base();
        s.invert = true;
        let mut img = GrayImage::from_pixel(1, 1, Luma([10]));
        tone(&mut img, &s);
        assert_eq!(img.get_pixel(0, 0)[0], 245);
    }

    #[test]
    fn light_contrast_stretches_min_to_max() {
        let mut s = base();
        s.contrast = Contrast::Light;
        let mut img = GrayImage::from_fn(3, 1, |x, _| Luma([[20, 120, 220][x as usize]]));
        tone(&mut img, &s);
        assert_eq!((img.get_pixel(0, 0)[0], img.get_pixel(2, 0)[0]), (0, 255));
    }

    #[test]
    fn contrast_skips_intentionally_flat_pages() {
        // KCC: range < 159 is probably intentional, leave it
        let mut s = base();
        s.contrast = Contrast::Maximum;
        let mut img = GrayImage::from_fn(2, 1, |x, _| Luma([[100, 200][x as usize]]));
        tone(&mut img, &s);
        assert_eq!((img.get_pixel(0, 0)[0], img.get_pixel(1, 0)[0]), (100, 200));
    }

    #[test]
    fn gamma_darkens_midtones() {
        let mut s = base();
        s.gamma = Some(2.0);
        let mut img = GrayImage::from_pixel(1, 1, Luma([128]));
        tone(&mut img, &s);
        assert_eq!(img.get_pixel(0, 0)[0], 64);
    }

    #[test]
    fn extreme_black_clamps_dark_grays() {
        let mut s = base();
        s.extreme_black = true;
        let mut img = GrayImage::from_fn(2, 1, |x, _| Luma([[60, 200][x as usize]]));
        tone(&mut img, &s);
        assert_eq!((img.get_pixel(0, 0)[0], img.get_pixel(1, 0)[0]), (0, 200));
    }
}
