//! XTC (1-bit) / XTCH (2-bit) container, layout per CrossPoint `lib/Xtc/Xtc/XtcTypes.h`.

use crate::pack::{pack, page_size, unpack, Depth, Levels};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub struct Chapter {
    pub name: String,
    pub start: u16,
    pub end: u16,
}

pub struct Meta {
    pub title: String,
    pub author: String,
}

pub struct XtcWriter<W: Write + Seek> {
    out: W,
    depth: Depth,
    width: u16,
    height: u16,
    expected: u32,
    pushed: u32,
}

const TITLE_AT: usize = 0x38;
const CHAPTERS_AT: usize = 0x138;
const CHAPTER_LEN: usize = 96;
const ENTRY_LEN: usize = 16;

/// NUL-terminated field of `len` bytes; text cut to `len - 1` bytes on a char boundary.
fn fixed_str(s: &str, len: usize) -> Vec<u8> {
    let mut end = s.len().min(len - 1);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut v = s.as_bytes()[..end].to_vec();
    v.resize(len, 0);
    v
}

fn read_str(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

impl<W: Write + Seek> XtcWriter<W> {
    /// Writes header, metadata, chapters and the page table up front (page size is fixed).
    pub fn new(mut out: W, depth: Depth, w: u16, h: u16, page_count: u32, meta: &Meta, chapters: &[Chapter]) -> anyhow::Result<Self> {
        anyhow::ensure!(page_count <= u16::MAX as u32, "{page_count} pages exceeds the XTC limit of 65535");
        anyhow::ensure!(w > 0 && h > 0, "page size {w}x{h} is invalid");
        let size = page_size(w, h, depth);
        let table = CHAPTERS_AT + CHAPTER_LEN * chapters.len();
        let data = table + ENTRY_LEN * page_count as usize;
        let mut b = Vec::with_capacity(data);
        b.extend_from_slice(match depth {
            Depth::One => b"XTC\0",
            Depth::Two => b"XTCH",
        });
        b.extend_from_slice(&[1, 0]); // version 1.0
        b.extend_from_slice(&(page_count as u16).to_le_bytes());
        // readDirection 0 as in the reference files (page order already encodes RTL)
        b.extend_from_slice(&[0, 1, 0, !chapters.is_empty() as u8]);
        b.extend_from_slice(&1u32.to_le_bytes()); // currentPage
        for off in [TITLE_AT, table, data, 0] {
            b.extend_from_slice(&(off as u64).to_le_bytes());
        }
        b.extend_from_slice(&(CHAPTERS_AT as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // padding must be 0: CrossPoint reads 0x30 as u64
        b.extend(fixed_str(&meta.title, 128));
        b.extend(fixed_str(&meta.author, 64));
        b.resize(CHAPTERS_AT, 0); // 64 reserved bytes after author (unread by CrossPoint)
        for c in chapters {
            b.extend(fixed_str(&c.name, 80));
            b.extend_from_slice(&c.start.to_le_bytes());
            b.extend_from_slice(&c.end.to_le_bytes());
            b.extend_from_slice(&[0; 12]);
        }
        for i in 0..page_count as usize {
            b.extend_from_slice(&((data + i * size) as u64).to_le_bytes());
            b.extend_from_slice(&(size as u32).to_le_bytes());
            b.extend_from_slice(&w.to_le_bytes());
            b.extend_from_slice(&h.to_le_bytes());
        }
        debug_assert_eq!(b.len(), data);
        out.write_all(&b)?;
        Ok(Self { out, depth, width: w, height: h, expected: page_count, pushed: 0 })
    }

    pub fn push(&mut self, page: &Levels) -> anyhow::Result<()> {
        anyhow::ensure!(
            (page.width, page.height, page.depth) == (self.width, self.height, self.depth),
            "page is {}x{} {:?}, file expects {}x{} {:?}",
            page.width, page.height, page.depth, self.width, self.height, self.depth
        );
        self.push_packed(&pack(page))
    }

    /// Append a page already encoded with `pack` (must match this file's size and depth).
    pub fn push_packed(&mut self, page: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(page.len() == page_size(self.width, self.height, self.depth), "packed page has wrong size {}", page.len());
        anyhow::ensure!(self.pushed < self.expected, "more pages than the declared {}", self.expected);
        self.out.write_all(page)?;
        self.pushed += 1;
        Ok(())
    }

    pub fn finish(mut self) -> anyhow::Result<W> {
        anyhow::ensure!(self.pushed == self.expected, "wrote {} of {} pages", self.pushed, self.expected);
        self.out.flush()?;
        Ok(self.out)
    }
}

pub struct XtcFile {
    pub depth: Depth,
    pub page_count: u16,
    pub title: String,
    pub author: String,
    pub read_direction: u8,
    pub chapters: Vec<Chapter>,
}

fn read_at(f: &mut File, off: u64, len: usize) -> anyhow::Result<Vec<u8>> {
    let mut b = vec![0u8; len];
    f.seek(SeekFrom::Start(off))?;
    f.read_exact(&mut b)?;
    Ok(b)
}

struct Header {
    depth: Depth,
    pages: u16,
    read_direction: u8,
    has_chapters: bool,
    table: u64,
    chapters: u64,
}

fn header(f: &mut File) -> anyhow::Result<Header> {
    let h = read_at(f, 0, 0x38)?;
    let depth = match &h[0..4] {
        b"XTC\0" => Depth::One,
        b"XTCH" => Depth::Two,
        m => anyhow::bail!("not an XTC file (magic {m:?})"),
    };
    let u64_at = |o: usize| u64::from_le_bytes(h[o..o + 8].try_into().unwrap());
    Ok(Header {
        depth,
        pages: u16::from_le_bytes([h[6], h[7]]),
        read_direction: h[8],
        has_chapters: h[0x0B] == 1,
        table: u64_at(0x18),
        chapters: u32::from_le_bytes(h[0x30..0x34].try_into().unwrap()) as u64,
    })
}

pub fn read_xtc(path: &Path) -> anyhow::Result<XtcFile> {
    let mut f = File::open(path)?;
    let h = header(&mut f)?;
    let meta = read_at(&mut f, TITLE_AT as u64, 128 + 64)?;
    let mut chapters = vec![];
    if h.has_chapters && h.chapters > 0 && h.table > h.chapters {
        let n = ((h.table - h.chapters) / CHAPTER_LEN as u64) as usize;
        let raw = read_at(&mut f, h.chapters, n * CHAPTER_LEN)?;
        for e in raw.chunks(CHAPTER_LEN) {
            let (start, end) = (u16::from_le_bytes([e[80], e[81]]), u16::from_le_bytes([e[82], e[83]]));
            let name = read_str(&e[..80]);
            if name.is_empty() && start == 0 && end == 0 {
                break;
            }
            chapters.push(Chapter { name, start, end });
        }
    }
    Ok(XtcFile {
        depth: h.depth,
        page_count: h.pages,
        title: read_str(&meta[..128]),
        author: read_str(&meta[128..]),
        read_direction: h.read_direction,
        chapters,
    })
}

pub fn read_page(path: &Path, index: u16) -> anyhow::Result<Levels> {
    let mut f = File::open(path)?;
    let h = header(&mut f)?;
    anyhow::ensure!(index < h.pages, "page {index} out of range (file has {} pages)", h.pages);
    let e = read_at(&mut f, h.table + ENTRY_LEN as u64 * index as u64, ENTRY_LEN)?;
    let off = u64::from_le_bytes(e[0..8].try_into().unwrap());
    let size = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
    unpack(&read_at(&mut f, off, size)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn meta() -> Meta {
        Meta { title: "Centuria".into(), author: "KCC".into() }
    }

    fn white(w: u16, h: u16, d: Depth) -> Levels {
        let v = if d == Depth::One { 1 } else { 3 };
        Levels { width: w, height: h, depth: d, px: vec![v; w as usize * h as usize] }
    }

    fn write3(d: Depth, chapters: &[Chapter]) -> Vec<u8> {
        let mut wr = XtcWriter::new(Cursor::new(vec![]), d, 480, 800, 3, &meta(), chapters).unwrap();
        for _ in 0..3 {
            wr.push(&white(480, 800, d)).unwrap();
        }
        wr.finish().unwrap().into_inner()
    }

    fn u64_at(b: &[u8], o: usize) -> u64 {
        u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
    }

    fn tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("xtc-core-test-{}-{name}", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn header_matches_reference_layout() {
        let ch = [Chapter { name: "Chapter 1 - 100 Slaves".into(), start: 1, end: 3 }];
        let b = write3(Depth::Two, &ch);
        assert_eq!(&b[0..4], b"XTCH");
        assert_eq!((b[4], b[5]), (1, 0));
        assert_eq!(u16::from_le_bytes([b[6], b[7]]), 3);
        assert_eq!(&b[8..16], &[0, 1, 0, 1, 1, 0, 0, 0]);
        assert_eq!(u64_at(&b, 0x10), 0x38);
        let table = 0x138 + 96;
        assert_eq!(u64_at(&b, 0x18), table);
        assert_eq!(u64_at(&b, 0x20), table + 3 * 16);
        assert_eq!(u64_at(&b, 0x28), 0);
        assert_eq!(u32::from_le_bytes(b[0x30..0x34].try_into().unwrap()), 0x138);
        assert_eq!(u32::from_le_bytes(b[0x34..0x38].try_into().unwrap()), 0);
        assert_eq!(&b[0x38..0x41], b"Centuria\0");
        assert_eq!(&b[0xB8..0xBC], b"KCC\0");
        assert_eq!(&b[0x138..0x138 + 22], b"Chapter 1 - 100 Slaves");
        assert_eq!(u16::from_le_bytes([b[0x138 + 0x50], b[0x138 + 0x51]]), 1);
        assert_eq!(u16::from_le_bytes([b[0x138 + 0x52], b[0x138 + 0x53]]), 3);
    }

    #[test]
    fn xtc_magic_for_one_bit() {
        assert_eq!(&write3(Depth::One, &[])[0..4], b"XTC\0");
    }

    #[test]
    fn page_table_entries_are_consecutive() {
        let b = write3(Depth::Two, &[]);
        let table = u64_at(&b, 0x18) as usize;
        let data = u64_at(&b, 0x20);
        for i in 0..3 {
            let e = &b[table + 16 * i..table + 16 * i + 16];
            assert_eq!(u64_at(e, 0), data + i as u64 * 96_022);
            assert_eq!(u32::from_le_bytes(e[8..12].try_into().unwrap()), 96_022);
            assert_eq!(u16::from_le_bytes([e[12], e[13]]), 480);
            assert_eq!(u16::from_le_bytes([e[14], e[15]]), 800);
        }
        assert_eq!(b.len() as u64, data + 3 * 96_022);
    }

    #[test]
    fn round_trip_meta_chapters_pages() {
        let ch = [
            Chapter { name: "One".into(), start: 1, end: 1 },
            Chapter { name: "Two".into(), start: 2, end: 2 },
        ];
        let mut wr = XtcWriter::new(Cursor::new(vec![]), Depth::One, 16, 4, 2, &meta(), &ch).unwrap();
        let mut p0 = white(16, 4, Depth::One);
        p0.px[5] = 0;
        let p1 = white(16, 4, Depth::One);
        wr.push(&p0).unwrap();
        wr.push(&p1).unwrap();
        let path = tmp("rt.xtc", &wr.finish().unwrap().into_inner());
        let f = read_xtc(&path).unwrap();
        assert_eq!((f.depth, f.page_count, f.title.as_str(), f.author.as_str()), (Depth::One, 2, "Centuria", "KCC"));
        assert_eq!(f.chapters.len(), 2);
        assert_eq!((f.chapters[1].name.as_str(), f.chapters[1].start, f.chapters[1].end), ("Two", 2, 2));
        assert_eq!(read_page(&path, 0).unwrap(), p0);
        assert_eq!(read_page(&path, 1).unwrap(), p1);
        assert!(read_page(&path, 2).is_err());
    }

    #[test]
    fn long_unicode_chapter_name_truncates_on_char_boundary() {
        let long = "第一話 ".repeat(20);
        let ch = [Chapter { name: long.clone(), start: 1, end: 3 }];
        let path = tmp("long.xtch", &write3(Depth::Two, &ch));
        let name = &read_xtc(&path).unwrap().chapters[0].name;
        assert!(name.len() <= 79 && !name.is_empty());
        assert!(long.starts_with(name.as_str()));
    }

    #[test]
    fn too_many_pages_is_error() {
        assert!(XtcWriter::new(Cursor::new(vec![]), Depth::One, 8, 8, 65_536, &meta(), &[]).is_err());
    }

    #[test]
    fn zero_size_page_is_error() {
        assert!(XtcWriter::new(Cursor::new(vec![]), Depth::One, 0, 800, 1, &meta(), &[]).is_err());
        assert!(XtcWriter::new(Cursor::new(vec![]), Depth::One, 480, 0, 1, &meta(), &[]).is_err());
    }

    #[test]
    fn finish_with_missing_pages_is_error() {
        let mut wr = XtcWriter::new(Cursor::new(vec![]), Depth::One, 8, 8, 2, &meta(), &[]).unwrap();
        wr.push(&white(8, 8, Depth::One)).unwrap();
        assert!(wr.finish().is_err());
    }

    #[test]
    fn push_packed_equals_push() {
        let p = white(8, 8, Depth::Two);
        let mut a = XtcWriter::new(Cursor::new(vec![]), Depth::Two, 8, 8, 1, &meta(), &[]).unwrap();
        a.push(&p).unwrap();
        let mut b = XtcWriter::new(Cursor::new(vec![]), Depth::Two, 8, 8, 1, &meta(), &[]).unwrap();
        b.push_packed(&pack(&p)).unwrap();
        assert_eq!(a.finish().unwrap().into_inner(), b.finish().unwrap().into_inner());
        let mut c = XtcWriter::new(Cursor::new(vec![]), Depth::Two, 8, 8, 1, &meta(), &[]).unwrap();
        assert!(c.push_packed(&[0u8; 5]).is_err());
    }

    #[test]
    fn push_wrong_size_or_depth_is_error() {
        let mut wr = XtcWriter::new(Cursor::new(vec![]), Depth::One, 8, 8, 2, &meta(), &[]).unwrap();
        assert!(wr.push(&white(8, 16, Depth::One)).is_err());
        assert!(wr.push(&white(8, 8, Depth::Two)).is_err());
    }

    #[test]
    #[ignore]
    fn reads_owner_reference() {
        let p = Path::new(r"C:\Users\A1ZES\Videos\E-readersAPP\Mangas\reference\Centuria.xtch");
        let f = read_xtc(p).unwrap();
        assert_eq!((f.depth, f.page_count, f.title.as_str()), (Depth::Two, 2365, "Centuria"));
        let c = &f.chapters[0];
        assert_eq!((c.name.as_str(), c.start, c.end), ("Chapter 1 - 100 Slaves", 1, 86));
        let pg = read_page(p, 2).unwrap();
        assert_eq!((pg.width, pg.height), (480, 800));
    }
}
