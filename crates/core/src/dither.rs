//! Quantize a screen-sized gray page to 2 or 4 levels, with optional dithering.

use crate::pack::{Depth, Levels};
use crate::settings::Dither;
use image::GrayImage;

/// Error-diffusion kernels: (dx, dy, weight); dx is mirrored on right-to-left rows.
const FLOYD_STEINBERG: &[(i32, i32, f32)] = &[(1, 0, 7.0 / 16.0), (-1, 1, 3.0 / 16.0), (0, 1, 5.0 / 16.0), (1, 1, 1.0 / 16.0)];
// Atkinson spreads only 6/8 of the error on purpose (keeps highlights and shadows clean)
const ATKINSON: &[(i32, i32, f32)] = &[(1, 0, 0.125), (2, 0, 0.125), (-1, 1, 0.125), (0, 1, 0.125), (1, 1, 0.125), (0, 2, 0.125)];
const SIERRA_LITE: &[(i32, i32, f32)] = &[(1, 0, 0.5), (-1, 1, 0.25), (0, 1, 0.25)];

/// 8x8 Bayer threshold index 0..64.
fn bayer(x: u32, y: u32) -> u32 {
    (0..3).fold(0, |v, i| {
        let (xb, yb) = ((x >> i) & 1, (y >> i) & 1);
        v * 4 + 2 * (xb ^ yb) + yb
    })
}

pub fn quantize(img: &GrayImage, depth: Depth, algo: Dither, strength: f32) -> Levels {
    let (w, h) = img.dimensions();
    let top = if depth == Depth::One { 1.0 } else { 3.0 };
    let step = 255.0 / top;
    let nearest = |v: f32| (v / step).round().clamp(0.0, top);
    let kernel = match algo {
        Dither::FloydSteinberg => FLOYD_STEINBERG,
        Dither::Atkinson => ATKINSON,
        Dither::SierraLite => SIERRA_LITE,
        Dither::Ordered | Dither::None => &[][..],
    };
    let mut px = vec![0u8; (w * h) as usize];
    if kernel.is_empty() {
        for (x, y, p) in img.enumerate_pixels() {
            let mut v = p[0] as f32;
            if algo == Dither::Ordered {
                v += ((bayer(x, y) as f32 + 0.5) / 64.0 - 0.5) * step * strength;
            }
            px[(y * w + x) as usize] = nearest(v) as u8;
        }
    } else {
        let mut buf: Vec<f32> = img.pixels().map(|p| p[0] as f32).collect();
        for y in 0..h as i32 {
            let ltr = y % 2 == 0; // serpentine scan avoids directional streaks
            for i in 0..w as i32 {
                let x = if ltr { i } else { w as i32 - 1 - i };
                let idx = (y as u32 * w + x as u32) as usize;
                let q = nearest(buf[idx]);
                px[idx] = q as u8;
                let err = (buf[idx] - q * step) * strength;
                for &(dx, dy, k) in kernel {
                    let (nx, ny) = (x + if ltr { dx } else { -dx }, y + dy);
                    if nx >= 0 && nx < w as i32 && ny < h as i32 {
                        buf[(ny as u32 * w + nx as u32) as usize] += err * k;
                    }
                }
            }
        }
    }
    Levels { width: w as u16, height: h as u16, depth, px }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    const ALL: [Dither; 5] = [Dither::FloydSteinberg, Dither::Atkinson, Dither::SierraLite, Dither::Ordered, Dither::None];

    fn flat(v: u8) -> GrayImage {
        GrayImage::from_pixel(64, 64, Luma([v]))
    }

    fn white_fraction(l: &Levels) -> f64 {
        l.px.iter().filter(|&&p| p == 1).count() as f64 / l.px.len() as f64
    }

    #[test]
    fn none_maps_to_nearest() {
        let img = GrayImage::from_pixel(1, 1, Luma([100]));
        assert_eq!(quantize(&img, Depth::Two, Dither::None, 1.0).px, [1]);
        assert_eq!(quantize(&img, Depth::One, Dither::None, 1.0).px, [0]);
    }

    #[test]
    fn pure_black_white_unchanged() {
        for a in ALL {
            for d in [Depth::One, Depth::Two] {
                let top = if d == Depth::One { 1 } else { 3 };
                assert!(quantize(&flat(0), d, a, 1.0).px.iter().all(|&p| p == 0), "{a:?}");
                assert!(quantize(&flat(255), d, a, 1.0).px.iter().all(|&p| p == top), "{a:?}");
            }
        }
    }

    #[test]
    fn fs_mid_gray_is_half_white() {
        let f = white_fraction(&quantize(&flat(128), Depth::One, Dither::FloydSteinberg, 1.0));
        assert!((0.45..=0.55).contains(&f), "{f}");
    }

    #[test]
    fn diffusion_algorithms_mid_gray_within_range() {
        for a in [Dither::Atkinson, Dither::SierraLite, Dither::Ordered] {
            let f = white_fraction(&quantize(&flat(128), Depth::One, a, 1.0));
            assert!((0.40..=0.60).contains(&f), "{a:?} {f}");
        }
    }

    #[test]
    fn strength_zero_equals_none() {
        let img = GrayImage::from_fn(32, 32, |x, y| Luma([((x * 8 + y * 3) % 256) as u8]));
        for a in ALL {
            assert_eq!(quantize(&img, Depth::Two, a, 0.0), quantize(&img, Depth::Two, Dither::None, 1.0), "{a:?}");
        }
    }

    #[test]
    fn two_bit_uses_only_0_to_3_and_keeps_size() {
        let img = GrayImage::from_fn(50, 30, |x, _| Luma([(x * 5) as u8]));
        let l = quantize(&img, Depth::Two, Dither::FloydSteinberg, 1.0);
        assert_eq!((l.width, l.height, l.px.len()), (50, 30, 1500));
        assert!(l.px.iter().all(|&p| p <= 3));
        assert!((0..=3).all(|v| l.px.contains(&v)));
    }
}
