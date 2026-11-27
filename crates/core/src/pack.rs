//! XTG (1-bit) and XTH (2-bit) page images, as read by CrossPoint `lib/Xtc/Xtc/XtcTypes.h`.

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Depth {
    One,
    Two,
}

/// Quantized page: one byte per pixel, row-major.
/// Depth::One: 0 = black, 1 = white. Depth::Two: 0 = black, 1 = dark, 2 = light, 3 = white.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Levels {
    pub width: u16,
    pub height: u16,
    pub depth: Depth,
    pub px: Vec<u8>,
}

pub const HEADER: usize = 22;

/// Levels → 8-bit gray for display (preview, dump, compare).
pub fn to_gray(l: &Levels) -> image::GrayImage {
    let lut: &[u8] = match l.depth {
        Depth::One => &[0, 255],
        Depth::Two => &[0, 85, 170, 255],
    };
    image::GrayImage::from_fn(l.width as u32, l.height as u32, |x, y| {
        image::Luma([lut[l.px[(y * l.width as u32 + x) as usize] as usize]])
    })
}

/// Level (0 = black .. 3 = white) <-> XTH code (0 white, 1 dark, 2 light, 3 black). Self-inverse.
const XTH_CODE: [u8; 4] = [3, 1, 2, 0];

fn data_size(w: u16, h: u16, d: Depth) -> usize {
    let (w, h) = (w as usize, h as usize);
    match d {
        Depth::One => w.div_ceil(8) * h,
        Depth::Two => 2 * w * h.div_ceil(8),
    }
}

pub fn page_size(w: u16, h: u16, d: Depth) -> usize {
    HEADER + data_size(w, h, d)
}

pub fn pack(l: &Levels) -> Vec<u8> {
    let (w, h) = (l.width as usize, l.height as usize);
    assert_eq!(l.px.len(), w * h, "pixel count must equal width*height");
    let size = data_size(l.width, l.height, l.depth);
    let mut out = Vec::with_capacity(HEADER + size);
    out.extend_from_slice(match l.depth {
        Depth::One => b"XTG\0",
        Depth::Two => b"XTH\0",
    });
    out.extend_from_slice(&l.width.to_le_bytes());
    out.extend_from_slice(&l.height.to_le_bytes());
    out.extend_from_slice(&[0, 0]); // colorMode, compression
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 8]); // md5 (optional, unread by CrossPoint)
    let mut data = vec![0u8; size];
    match l.depth {
        Depth::One => {
            let row = w.div_ceil(8);
            for y in 0..h {
                for x in 0..w {
                    if l.px[y * w + x] != 0 {
                        data[y * row + x / 8] |= 0x80 >> (x % 8);
                    }
                }
            }
        }
        Depth::Two => {
            let col = h.div_ceil(8);
            let plane = w * col;
            for y in 0..h {
                for x in 0..w {
                    let code = XTH_CODE[l.px[y * w + x] as usize];
                    let i = (w - 1 - x) * col + y / 8;
                    let bit = 0x80 >> (y % 8);
                    if code & 2 != 0 {
                        data[i] |= bit;
                    }
                    if code & 1 != 0 {
                        data[plane + i] |= bit;
                    }
                }
            }
        }
    }
    out.extend_from_slice(&data);
    out
}

pub fn unpack(b: &[u8]) -> anyhow::Result<Levels> {
    anyhow::ensure!(b.len() >= HEADER, "page too short");
    let depth = match &b[0..4] {
        b"XTG\0" => Depth::One,
        b"XTH\0" => Depth::Two,
        m => anyhow::bail!("bad page magic {m:?}"),
    };
    let width = u16::from_le_bytes([b[4], b[5]]);
    let height = u16::from_le_bytes([b[6], b[7]]);
    let size = u32::from_le_bytes(b[10..14].try_into()?) as usize;
    anyhow::ensure!(size == data_size(width, height, depth), "page dataSize {size} does not match {width}x{height}");
    anyhow::ensure!(b.len() >= HEADER + size, "page data truncated");
    let d = &b[HEADER..HEADER + size];
    let (w, h) = (width as usize, height as usize);
    let mut px = vec![0u8; w * h];
    match depth {
        Depth::One => {
            let row = w.div_ceil(8);
            for y in 0..h {
                for x in 0..w {
                    px[y * w + x] = (d[y * row + x / 8] >> (7 - x % 8)) & 1;
                }
            }
        }
        Depth::Two => {
            let col = h.div_ceil(8);
            let plane = w * col;
            for y in 0..h {
                for x in 0..w {
                    let i = (w - 1 - x) * col + y / 8;
                    let s = 7 - y % 8;
                    let code = ((d[i] >> s) & 1) << 1 | (d[plane + i] >> s) & 1;
                    px[y * w + x] = XTH_CODE[code as usize];
                }
            }
        }
    }
    Ok(Levels { width, height, depth, px })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xtg_size_480x800() {
        assert_eq!(page_size(480, 800, Depth::One), 22 + 48_000);
    }

    #[test]
    fn xth_size_480x800() {
        assert_eq!(page_size(480, 800, Depth::Two), 22 + 96_000);
    }

    #[test]
    fn xth_odd_height_pads_columns() {
        assert_eq!(page_size(4, 9, Depth::Two), 22 + 2 * 4 * 2);
    }

    #[test]
    fn xtg_header_and_bits() {
        let l = Levels { width: 8, height: 1, depth: Depth::One, px: vec![0, 1, 1, 1, 1, 1, 1, 1] };
        let b = pack(&l);
        assert_eq!(&b[0..4], b"XTG\0");
        assert_eq!(u16::from_le_bytes([b[4], b[5]]), 8);
        assert_eq!(u16::from_le_bytes([b[6], b[7]]), 1);
        assert_eq!(u32::from_le_bytes(b[10..14].try_into().unwrap()), 1);
        assert_eq!(&b[14..22], &[0u8; 8]);
        assert_eq!(b[22], 0b0111_1111);
    }

    #[test]
    fn xth_top_right_pixel_is_first_byte_msb() {
        // 2x8 all white except top-right black (level 0 -> code 3 -> bit set in both planes)
        let mut px = vec![3u8; 16];
        px[1] = 0;
        let b = pack(&Levels { width: 2, height: 8, depth: Depth::Two, px });
        assert_eq!(&b[0..4], b"XTH\0");
        assert_eq!(b[22], 0b1000_0000);
        assert_eq!(b[22 + 1], 0);
        assert_eq!(b[22 + 2], 0b1000_0000);
    }

    #[test]
    fn xth_gray_codes() {
        // 1x4 column, levels black, dark, light, white -> codes 3,1,2,0
        // plane1 (high bit): 1,0,1,0 ; plane2 (low bit): 1,1,0,0
        let b = pack(&Levels { width: 1, height: 4, depth: Depth::Two, px: vec![0, 1, 2, 3] });
        assert_eq!(b[22], 0b1010_0000);
        assert_eq!(b[23], 0b1100_0000);
    }

    fn pseudo_random(n: usize, modulo: u8) -> Vec<u8> {
        let mut x: u32 = 12345;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                ((x >> 16) % modulo as u32) as u8
            })
            .collect()
    }

    #[test]
    fn round_trip_both_depths() {
        for (d, m) in [(Depth::One, 2), (Depth::Two, 4)] {
            for (w, h) in [(13u16, 9u16), (480, 800)] {
                let l = Levels { width: w, height: h, depth: d, px: pseudo_random(w as usize * h as usize, m) };
                assert_eq!(unpack(&pack(&l)).unwrap(), l);
            }
        }
    }

    #[test]
    fn unpack_rejects_bad_magic_and_size() {
        let mut b = pack(&Levels { width: 8, height: 8, depth: Depth::One, px: vec![1; 64] });
        let mut bad = b.clone();
        bad[0..4].copy_from_slice(b"XXX\0");
        assert!(unpack(&bad).is_err());
        b[10] = 99; // dataSize mismatch
        assert!(unpack(&b).is_err());
        assert!(unpack(&[0u8; 10]).is_err());
    }
}
