//! Turns one source page into 1..n screen-sized pages: cover rules, spreads, orientation, resize.

use crate::settings::{Overview, Settings, Spread};
use image::imageops::{self, FilterType};
use image::{GrayImage, Luma};

#[derive(Clone, Copy)]
enum Fit {
    Stretch,
    Pad,
    Fill,
}

fn fit(img: &GrayImage, tw: u32, th: u32, mode: Fit) -> GrayImage {
    let (w, h) = img.dimensions();
    let f = FilterType::Lanczos3;
    match mode {
        Fit::Stretch => imageops::resize(img, tw, th, f),
        Fit::Pad | Fit::Fill => {
            let (sx, sy) = (tw as f64 / w as f64, th as f64 / h as f64);
            let k = if matches!(mode, Fit::Pad) { sx.min(sy) } else { sx.max(sy) };
            let (nw, nh) = (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1));
            let r = imageops::resize(img, nw, nh, f);
            let mut canvas = GrayImage::from_pixel(tw, th, Luma([255]));
            imageops::replace(&mut canvas, &r, (tw as i64 - nw as i64) / 2, (th as i64 - nh as i64) / 2);
            canvas
        }
    }
}

fn rotate(img: &GrayImage, clockwise: bool) -> GrayImage {
    if clockwise { imageops::rotate90(img) } else { imageops::rotate270(img) }
}

/// Vertical slices `(x, width)` in reading order.
fn slices(w: u32, spread: Spread, rtl: bool) -> Vec<(u32, u32)> {
    let mut v = match spread {
        Spread::Thirds => vec![(0, w / 2), (w / 4, w / 2), (w / 2, w - w / 2)],
        _ => vec![(0, w / 2), (w / 2, w - w / 2)],
    };
    if rtl {
        v.reverse();
    }
    v
}

pub fn layout(img: GrayImage, is_cover: bool, s: &Settings) -> Vec<GrayImage> {
    // landscape: lay out on a sideways screen, then store rotated back to the device's portrait buffer
    let (tw, th) = if s.landscape { (s.height as u32, s.width as u32) } else { (s.width as u32, s.height as u32) };
    let mode = if s.stretch { Fit::Stretch } else { Fit::Pad };
    let wide = img.width() > img.height();
    let mut pages = vec![];
    if is_cover && s.first_page_cover {
        let src = if wide && !s.keep_cover_portrait { rotate(&img, s.rotate_right) } else { img };
        pages.push(fit(&src, tw, th, if s.cover_fill { Fit::Fill } else { mode }));
    } else if !wide || s.spread == Spread::None {
        pages.push(fit(&img, tw, th, mode));
    } else {
        let rotated = || fit(&rotate(&img, s.rotate_right), tw, th, Fit::Pad);
        if s.spread == Spread::Rotate {
            pages.push(rotated());
        } else {
            match s.overview {
                Overview::None => {}
                Overview::Portrait => pages.push(fit(&img, tw, th, Fit::Pad)),
                Overview::Landscape => pages.push(rotated()),
            }
            if s.spread == Spread::Both {
                pages.push(rotated());
            }
            for (x, w) in slices(img.width(), s.spread, s.rtl) {
                pages.push(fit(&imageops::crop_imm(&img, x, 0, w, img.height()).to_image(), tw, th, mode));
            }
        }
    }
    if s.landscape {
        pages = pages.iter().map(|p| rotate(p, s.flip_landscape_cw)).collect();
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wide page: left half black, right half white.
    fn spread(w: u32, h: u32) -> GrayImage {
        GrayImage::from_fn(w, h, |x, _| Luma([if x < w / 2 { 0 } else { 255 }]))
    }

    fn mean(img: &GrayImage) -> f64 {
        img.pixels().map(|p| p[0] as f64).sum::<f64>() / img.len() as f64
    }

    fn s(spread: Spread, rtl: bool) -> Settings {
        let mut s = Settings::default();
        s.spread = spread;
        s.rtl = rtl;
        s
    }

    #[test]
    fn split_rtl_right_half_first() {
        let p = layout(spread(200, 100), false, &s(Spread::Split, true));
        assert_eq!(p.len(), 2);
        assert!(mean(&p[0]) > 250.0 && mean(&p[1]) < 5.0);
    }

    #[test]
    fn split_ltr_left_first() {
        let p = layout(spread(200, 100), false, &s(Spread::Split, false));
        assert!(mean(&p[0]) < 5.0 && mean(&p[1]) > 250.0);
    }

    #[test]
    fn rotate_gives_one_page() {
        let p = layout(spread(200, 100), false, &s(Spread::Rotate, true));
        assert_eq!(p.len(), 1);
        // rotated right (clockwise): the left (black) half ends up on top
        // rotated 100x200 fits as 400x800 with 40px white side padding; sample inside the content
        let top = imageops::crop_imm(&p[0], 60, 0, 360, 380).to_image();
        assert!(mean(&top) < 5.0);
    }

    #[test]
    fn both_gives_rotated_then_halves() {
        let p = layout(spread(200, 100), false, &s(Spread::Both, true));
        assert_eq!(p.len(), 3);
        assert!(mean(&p[1]) > 250.0 && mean(&p[2]) < 5.0);
    }

    #[test]
    fn thirds_gives_three() {
        assert_eq!(layout(spread(200, 100), false, &s(Spread::Thirds, true)).len(), 3);
    }

    #[test]
    fn none_keeps_spread_whole() {
        assert_eq!(layout(spread(200, 100), false, &s(Spread::None, true)).len(), 1);
    }

    #[test]
    fn overview_adds_full_page_before_splits() {
        let mut st = s(Spread::Split, true);
        st.overview = Overview::Portrait;
        assert_eq!(layout(spread(200, 100), false, &st).len(), 3);
    }

    #[test]
    fn cover_not_split() {
        assert_eq!(layout(spread(200, 100), true, &s(Spread::Split, true)).len(), 1);
    }

    #[test]
    fn portrait_page_one_output() {
        assert_eq!(layout(GrayImage::from_pixel(100, 160, Luma([128])), false, &Settings::default()).len(), 1);
    }

    #[test]
    fn output_is_exact_device_size() {
        for (w, h) in [(480, 800), (528, 792)] {
            let mut st = s(Spread::Both, true);
            st.width = w;
            st.height = h;
            for page in [spread(300, 200), GrayImage::from_pixel(97, 151, Luma([0]))] {
                for out in layout(page, false, &st) {
                    assert_eq!(out.dimensions(), (w as u32, h as u32));
                }
            }
        }
    }

    #[test]
    fn landscape_output_is_still_device_size() {
        let mut st = Settings::default();
        st.landscape = true;
        for out in layout(spread(300, 200), false, &st) {
            assert_eq!(out.dimensions(), (480, 800));
        }
    }

    #[test]
    fn fit_pads_white() {
        let mut st = Settings::default();
        st.stretch = false;
        let out = &layout(GrayImage::from_pixel(100, 100, Luma([0])), false, &st)[0];
        assert_eq!(out.get_pixel(0, 0)[0], 255);
        assert_eq!(out.get_pixel(240, 400)[0], 0);
    }

    #[test]
    fn cover_fill_has_no_padding() {
        let mut st = Settings::default();
        st.stretch = false;
        st.cover_fill = true;
        let out = &layout(GrayImage::from_pixel(100, 100, Luma([0])), true, &st)[0];
        assert_eq!(out.get_pixel(0, 0)[0], 0);
    }
}
