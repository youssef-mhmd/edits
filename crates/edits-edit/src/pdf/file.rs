//! Reading a PDF lazily: only the cross-reference data at first, then the
//! objects asked for. A 500 MB file costs a few milliseconds to open here,
//! and nothing is held in memory but what was read.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::Arc;

use super::object::{Dict, Obj, Ref, is_white, parse, skip_ws, token};

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("could not read the file: {0}")]
    Io(#[from] std::io::Error),
    #[error("the file's structure is damaged ({0})")]
    Damaged(String),
    #[error("the file is encrypted")]
    Encrypted,
}

fn damaged<T>(msg: impl Into<String>) -> Result<T, PdfError> {
    Err(PdfError::Damaged(msg.into()))
}

/// Bytes the reader can fetch by position.
pub trait Source {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Reads up to `buf.len()` bytes at `offset`; fewer only at the end.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize>;
}

/// A file on disk.
pub struct FileSource {
    file: std::fs::File,
    len: u64,
}

impl FileSource {
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }
}

impl Source for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut done = 0;
        while done < buf.len() {
            #[cfg(windows)]
            let n = std::os::windows::fs::FileExt::seek_read(&self.file, &mut buf[done..], offset + done as u64)?;
            #[cfg(unix)]
            let n = std::os::unix::fs::FileExt::read_at(&self.file, &mut buf[done..], offset + done as u64)?;
            if n == 0 {
                break;
            }
            done += n;
        }
        Ok(done)
    }
}

/// A file with bytes appended in memory: the document as it will be once
/// saved. PDFium reads it through [`Layered::reader`].
#[derive(Clone)]
pub struct Layered {
    base: Arc<FileSource>,
    pub tail: Arc<Vec<u8>>,
}

impl Layered {
    pub fn new(base: Arc<FileSource>, tail: Arc<Vec<u8>>) -> Self {
        Self { base, tail }
    }

    /// A `Read + Seek` view of the whole, for PDFium.
    pub fn reader(&self) -> LayeredReader {
        LayeredReader { src: self.clone(), pos: 0 }
    }
}

impl Source for Layered {
    fn len(&self) -> u64 {
        self.base.len() + self.tail.as_slice().len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let base_len = self.base.len();
        let mut done = 0;
        if offset < base_len {
            let want = buf.len().min((base_len - offset) as usize);
            done = self.base.read_at(offset, &mut buf[..want])?;
            if done < want {
                return Ok(done);
            }
        }
        let t_off = (offset + done as u64).saturating_sub(base_len) as usize;
        let tail = self.tail.as_slice();
        if t_off < tail.len() && done < buf.len() {
            let n = (buf.len() - done).min(tail.len() - t_off);
            buf[done..done + n].copy_from_slice(&tail[t_off..t_off + n]);
            done += n;
        }
        Ok(done)
    }
}

pub struct LayeredReader {
    src: Layered,
    pos: u64,
}

impl Read for LayeredReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.src.read_at(self.pos, buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl std::io::Seek for LayeredReader {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        let len = self.src.len() as i64;
        let p = match to {
            std::io::SeekFrom::Start(p) => p as i64,
            std::io::SeekFrom::End(d) => len + d,
            std::io::SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if p < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = p as u64;
        Ok(self.pos)
    }
}

impl<T: Source> Source for Arc<T> {
    fn len(&self) -> u64 {
        (**self).len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        (**self).read_at(offset, buf)
    }
}

impl Source for Vec<u8> {
    fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let o = (offset as usize).min(self.as_slice().len());
        let n = buf.len().min(self.as_slice().len() - o);
        buf[..n].copy_from_slice(&self[o..o + n]);
        Ok(n)
    }
}

/// The objects of a decoded object stream: number and value.
type Members = Vec<(u32, Obj)>;

#[derive(Debug, Clone, Copy)]
enum Entry {
    Offset(u64),
    InStream(u32),
}

/// Largest stream decoded (cross-reference and object streams): bounds
/// memory on hostile files.
const MAX_DECODED: usize = 64 << 20;
/// Largest number of cross-reference sections followed.
const MAX_SECTIONS: usize = 4096;

pub struct PdfFile<S: Source> {
    src: S,
    xref: HashMap<u32, Entry>,
    /// The newest trailer.
    pub trailer: Dict,
    /// Where the newest cross-reference section starts.
    pub startxref: u64,
    /// The newest section is a cross-reference stream (an update must be too).
    pub xref_stream: bool,
    /// One more than the highest object number in use.
    pub size: u32,
    streams: RefCell<HashMap<u32, Arc<Members>>>,
    pages: RefCell<Option<Vec<Ref>>>,
}

impl<S: Source> PdfFile<S> {
    pub fn open(src: S) -> Result<Self, PdfError> {
        let len = src.len();
        let tail_len = len.min(2048);
        let mut tail = vec![0u8; tail_len as usize];
        src.read_at(len - tail_len, &mut tail)?;
        let pos = tail.windows(9).rposition(|w| w == b"startxref").ok_or(PdfError::Damaged("no startxref".into()))?;
        let mut p = pos + 9;
        skip_ws(&tail, &mut p);
        let startxref: u64 = std::str::from_utf8(token(&tail, &mut p))
            .ok()
            .and_then(|t| t.parse().ok())
            .ok_or(PdfError::Damaged("bad startxref".into()))?;
        let mut f = PdfFile {
            src,
            xref: HashMap::new(),
            trailer: Dict::default(),
            startxref,
            xref_stream: false,
            size: 0,
            streams: RefCell::new(HashMap::new()),
            pages: RefCell::new(None),
        };
        let mut queue = vec![startxref];
        let mut seen = HashSet::new();
        let mut first = true;
        while let Some(off) = queue.pop() {
            if !seen.insert(off) || seen.len() > MAX_SECTIONS {
                continue;
            }
            let (trailer, is_stream) = f.read_section(off)?;
            if first {
                f.trailer = trailer.clone();
                f.xref_stream = is_stream;
                first = false;
            }
            if let Some(n) = trailer.get(b"Size").and_then(Obj::as_int) {
                f.size = f.size.max(n.clamp(0, u32::MAX as i64) as u32);
            }
            // /Prev last so that /XRefStm of this section is read before older ones.
            if let Some(prev) = trailer.get(b"Prev").and_then(Obj::as_int).filter(|v| *v >= 0) {
                queue.push(prev as u64);
            }
            if let Some(x) = trailer.get(b"XRefStm").and_then(Obj::as_int).filter(|v| *v >= 0) {
                queue.push(x as u64);
            }
        }
        if let Some(m) = f.xref.keys().max() {
            f.size = f.size.max(m + 1);
        }
        Ok(f)
    }

    pub fn len(&self) -> u64 {
        self.src.len()
    }

    pub fn is_empty(&self) -> bool {
        self.src.len() == 0
    }

    pub fn is_encrypted(&self) -> bool {
        self.trailer.get(b"Encrypt").is_some()
    }

    /// Reads `len` bytes at `off` (fewer at the end of the file).
    fn read(&self, off: u64, len: usize) -> Result<Vec<u8>, PdfError> {
        let len = len.min(self.src.len().saturating_sub(off) as usize);
        let mut b = vec![0u8; len];
        let n = self.src.read_at(off, &mut b)?;
        b.truncate(n);
        Ok(b)
    }

    /// Reads one cross-reference section; returns its trailer and whether it is a stream.
    fn read_section(&mut self, off: u64) -> Result<(Dict, bool), PdfError> {
        let mut window = 64 * 1024;
        loop {
            let b = self.read(off, window)?;
            let mut p = 0;
            skip_ws(&b, &mut p);
            let complete = b.len() as u64 + off >= self.src.len();
            if b[p..].starts_with(b"xref") {
                // Read until the trailer is in the window, then parse once.
                let has_trailer = b[p..].windows(7).position(|w| w == b"trailer").is_some_and(|t| {
                    let rest = &b[p + t..];
                    rest.windows(2).any(|w| w == b">>")
                });
                if !has_trailer && !complete {
                    window *= 4;
                    continue;
                }
                return self.parse_table(&b, p + 4).map(|t| (t, false));
            } else {
                let (_, obj) = self.object_at(off)?;
                let Obj::Stream(d, data) = obj else { return damaged("cross-reference section not found") };
                if d.name(b"Type") != Some(b"XRef") {
                    return damaged("cross-reference section not found");
                }
                let data = decode(&d, &data)?;
                self.parse_xref_stream(&d, &data)?;
                return Ok((d, true));
            }
        }
    }

    fn parse_table(&mut self, b: &[u8], mut p: usize) -> Result<Dict, PdfError> {
        let mut found = Vec::new();
        loop {
            skip_ws(b, &mut p);
            if b[p..].starts_with(b"trailer") {
                p += 7;
                let trailer = match parse(b, &mut p) {
                    Ok(Obj::Dict(d)) => d,
                    _ => return damaged("bad trailer"),
                };
                for (num, e) in found {
                    self.xref.entry(num).or_insert(e);
                }
                return Ok(trailer);
            }
            let start: u64 = num_token(b, &mut p).ok_or(PdfError::Damaged("bad xref".into()))?;
            let count: u64 = num_token(b, &mut p).ok_or(PdfError::Damaged("bad xref".into()))?;
            if count > 10_000_000 {
                return damaged("xref too large");
            }
            for i in 0..count {
                let off: u64 = num_token(b, &mut p).ok_or(PdfError::Damaged("bad xref entry".into()))?;
                let _generation: u64 = num_token(b, &mut p).ok_or(PdfError::Damaged("bad xref entry".into()))?;
                skip_ws(b, &mut p);
                let kind = token(b, &mut p);
                // Free entries are left out, so that a hybrid file's stream can fill them.
                if kind == b"n" && off > 0 {
                    found.push(((start + i) as u32, Entry::Offset(off)));
                }
            }
        }
    }

    fn parse_xref_stream(&mut self, d: &Dict, data: &[u8]) -> Result<(), PdfError> {
        let w: Vec<usize> = d
            .get(b"W")
            .and_then(Obj::numbers)
            .filter(|w| w.len() == 3)
            .map(|w| w.iter().map(|v| *v as usize).collect())
            .ok_or(PdfError::Damaged("bad /W".into()))?;
        if w.iter().any(|v| *v > 8) {
            return damaged("bad /W");
        }
        let size = d.get(b"Size").and_then(Obj::as_int).unwrap_or(0);
        let index: Vec<i64> = match d.get(b"Index").and_then(Obj::as_array) {
            Some(a) => a.iter().filter_map(Obj::as_int).collect(),
            None => vec![0, size],
        };
        let row = w[0] + w[1] + w[2];
        if row == 0 {
            return damaged("bad /W");
        }
        let field = |b: &[u8], len: usize, default: u64| -> u64 {
            if len == 0 { default } else { b[..len].iter().fold(0u64, |a, &c| a << 8 | c as u64) }
        };
        let mut rows = data.chunks_exact(row);
        for pair in index.as_chunks::<2>().0 {
            let (start, count) = (pair[0].max(0) as u64, pair[1].max(0) as u64);
            for i in 0..count {
                let Some(r) = rows.next() else { return Ok(()) };
                let t = field(r, w[0], 1);
                let a = field(&r[w[0]..], w[1], 0);
                let _generation = field(&r[w[0] + w[1]..], w[2], 0);
                let num = (start + i) as u32;
                let e = match t {
                    1 if a > 0 => Entry::Offset(a),
                    2 => Entry::InStream(a as u32),
                    _ => continue,
                };
                self.xref.entry(num).or_insert(e);
            }
        }
        Ok(())
    }

    /// Parses the indirect object at `off`: its reference and value, with
    /// stream data read.
    fn object_at(&self, off: u64) -> Result<(Ref, Obj), PdfError> {
        let mut window = 16 * 1024;
        loop {
            let b = self.read(off, window)?;
            let complete = off + b.len() as u64 >= self.src.len();
            let mut p = 0;
            let parsed = (|| {
                let n: u64 = num_token(&b, &mut p)?;
                let g: u64 = num_token(&b, &mut p)?;
                skip_ws(&b, &mut p);
                if token(&b, &mut p) != b"obj" {
                    return None;
                }
                let o = parse(&b, &mut p).ok()?;
                Some(((n as u32, g as u16), o, p))
            })();
            match parsed {
                Some((r, o, mut p)) => {
                    let mut q = p;
                    skip_ws(&b, &mut q);
                    if let Obj::Dict(d) = &o
                        && b[q..].starts_with(b"stream")
                    {
                        let d = d.clone();
                        p = q + 6;
                        if b.get(p) == Some(&b'\r') {
                            p += 1;
                        }
                        if b.get(p) == Some(&b'\n') {
                            p += 1;
                        }
                        let data_off = off + p as u64;
                        let data = self.stream_data(&d, data_off, r)?;
                        return Ok((r, Obj::Stream(d, data)));
                    }
                    return Ok((r, o));
                }
                None if !complete && window < (64 << 20) => window *= 4,
                None => return damaged(format!("no object at offset {off}")),
            }
        }
    }

    fn stream_data(&self, d: &Dict, off: u64, own: Ref) -> Result<Vec<u8>, PdfError> {
        let len = match d.get(b"Length") {
            Some(Obj::Ref(r)) if *r != own => self.get(*r).ok().and_then(|o| o.as_int()),
            Some(o) => o.as_int(),
            None => None,
        };
        if let Some(len) = len.filter(|l| *l >= 0 && (*l as u64) <= self.src.len().saturating_sub(off)) {
            let data = self.read(off, len as usize)?;
            // Check the length against the end marker; damaged files lie.
            let after = self.read(off + len as u64, 32)?;
            let mut p = 0;
            skip_ws(&after, &mut p);
            if after[p..].starts_with(b"endstream") {
                return Ok(data);
            }
        }
        // Search for the end marker instead.
        let mut window = 64 * 1024;
        loop {
            let b = self.read(off, window)?;
            if let Some(end) = b.windows(9).position(|w| w == b"endstream") {
                let mut e = end;
                while e > 0 && is_white(b[e - 1]) && (b[e - 1] == b'\n' || b[e - 1] == b'\r') {
                    e -= 1;
                }
                return Ok(b[..e].to_vec());
            }
            if off + b.len() as u64 >= self.src.len() || window > (256 << 20) {
                return damaged("stream without end");
            }
            window *= 4;
        }
    }

    /// The object `r`, or `Null` if it does not exist (as PDF readers treat it).
    pub fn get(&self, r: Ref) -> Result<Obj, PdfError> {
        match self.xref.get(&r.0) {
            None => Ok(Obj::Null),
            Some(Entry::Offset(off)) => {
                let (got, o) = self.object_at(*off)?;
                if got.0 != r.0 {
                    return damaged(format!("object {} is not where the table says", r.0));
                }
                Ok(o)
            }
            Some(Entry::InStream(container)) => {
                let objs = self.object_stream(*container)?;
                Ok(objs.iter().find(|(n, _)| *n == r.0).map(|(_, o)| o.clone()).unwrap_or(Obj::Null))
            }
        }
    }

    /// Follows a reference; other objects come back as they are.
    pub fn resolve(&self, o: &Obj) -> Result<Obj, PdfError> {
        match o {
            Obj::Ref(r) => self.get(*r),
            _ => Ok(o.clone()),
        }
    }

    fn object_stream(&self, container: u32) -> Result<Arc<Members>, PdfError> {
        if let Some(s) = self.streams.borrow().get(&container) {
            return Ok(s.clone());
        }
        let Some(Entry::Offset(off)) = self.xref.get(&container).copied() else {
            return damaged("object stream missing");
        };
        let (_, o) = self.object_at(off)?;
        let Obj::Stream(d, raw) = o else { return damaged("object stream missing") };
        let data = decode(&d, &raw)?;
        let n = d.get(b"N").and_then(Obj::as_int).unwrap_or(0).clamp(0, 1_000_000) as usize;
        let first = d.get(b"First").and_then(Obj::as_int).unwrap_or(0).max(0) as usize;
        let mut p = 0;
        let mut heads = Vec::with_capacity(n);
        for _ in 0..n {
            let (Some(num), Some(o)) = (num_token::<u64>(&data, &mut p), num_token::<u64>(&data, &mut p)) else {
                break;
            };
            heads.push((num as u32, o as usize));
        }
        let mut out = Vec::with_capacity(heads.len());
        for (num, rel) in heads {
            let mut q = first + rel;
            if q < data.len()
                && let Ok(o) = parse(&data, &mut q)
            {
                out.push((num, o));
            }
        }
        let out = Arc::new(out);
        self.streams.borrow_mut().insert(container, out.clone());
        Ok(out)
    }

    /// Page objects in order.
    pub fn pages(&self) -> Result<Vec<Ref>, PdfError> {
        if let Some(p) = self.pages.borrow().as_ref() {
            return Ok(p.clone());
        }
        let root = self.resolve(self.trailer.get(b"Root").unwrap_or(&Obj::Null))?;
        let pages = root.as_dict().and_then(|d| d.get(b"Pages")).and_then(Obj::as_ref);
        let Some(top) = pages else { return damaged("no page tree") };
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        self.walk(top, &mut out, &mut seen, 0)?;
        *self.pages.borrow_mut() = Some(out.clone());
        Ok(out)
    }

    fn walk(&self, node: Ref, out: &mut Vec<Ref>, seen: &mut HashSet<u32>, depth: usize) -> Result<(), PdfError> {
        if depth > 64 || !seen.insert(node.0) {
            return Ok(());
        }
        let o = self.get(node)?;
        let Some(d) = o.as_dict() else { return Ok(()) };
        match d.get(b"Kids").and_then(Obj::as_array) {
            Some(kids) if d.name(b"Type") != Some(b"Page") => {
                for k in kids.iter().filter_map(Obj::as_ref) {
                    self.walk(k, out, seen, depth + 1)?;
                }
            }
            _ => out.push(node),
        }
        Ok(())
    }

    /// A page's dictionary and its /Annots entries.
    pub fn page_annots(&self, page: u32) -> Result<(Ref, Dict, Vec<Obj>), PdfError> {
        let pages = self.pages()?;
        let r = *pages.get(page as usize).ok_or(PdfError::Damaged(format!("no page {page}")))?;
        let o = self.get(r)?;
        let d = o.as_dict().cloned().unwrap_or_default();
        let annots = match d.get(b"Annots") {
            Some(a) => self.resolve(a)?.as_array().map(<[Obj]>::to_vec).unwrap_or_default(),
            None => Vec::new(),
        };
        Ok((r, d, annots))
    }
}

fn num_token<T: std::str::FromStr>(b: &[u8], p: &mut usize) -> Option<T> {
    skip_ws(b, p);
    std::str::from_utf8(token(b, p)).ok()?.parse().ok()
}

/// Decodes a stream's data (Flate, with PNG predictors), as cross-reference
/// and object streams need. Other filters are not needed here.
pub fn decode(d: &Dict, raw: &[u8]) -> Result<Vec<u8>, PdfError> {
    let filters: Vec<Vec<u8>> = match d.get(b"Filter") {
        None => Vec::new(),
        Some(Obj::Name(n)) => vec![n.clone()],
        Some(Obj::Array(a)) => a
            .iter()
            .filter_map(|o| match o {
                Obj::Name(n) => Some(n.clone()),
                _ => None,
            })
            .collect(),
        Some(_) => return damaged("bad /Filter"),
    };
    let mut data = raw.to_vec();
    for f in &filters {
        match f.as_slice() {
            b"FlateDecode" | b"Fl" => {
                let mut out = Vec::new();
                let mut z = flate2::read::ZlibDecoder::new(&data[..]).take(MAX_DECODED as u64 + 1);
                // A truncated stream still gives what it has.
                let _ = z.read_to_end(&mut out);
                if out.len() > MAX_DECODED {
                    return damaged("stream too large");
                }
                data = out;
            }
            _ => return damaged(format!("unsupported filter {}", String::from_utf8_lossy(f))),
        }
    }
    let parms = match d.get(b"DecodeParms") {
        Some(Obj::Dict(p)) => Some(p.clone()),
        Some(Obj::Array(a)) => a.iter().find_map(|o| o.as_dict().cloned()),
        _ => None,
    };
    if let Some(p) = parms {
        let predictor = p.get(b"Predictor").and_then(Obj::as_int).unwrap_or(1);
        if predictor >= 10 {
            let columns = p.get(b"Columns").and_then(Obj::as_int).unwrap_or(1).clamp(1, 1 << 20) as usize;
            data = png_unpredict(&data, columns)?;
        }
    }
    Ok(data)
}

fn png_unpredict(data: &[u8], columns: usize) -> Result<Vec<u8>, PdfError> {
    let row = columns + 1;
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; columns];
    for r in data.chunks(row) {
        if r.len() < row {
            break;
        }
        let (kind, src) = (r[0], &r[1..]);
        let mut cur = vec![0u8; columns];
        for i in 0..columns {
            let left = if i > 0 { cur[i - 1] } else { 0 };
            let up = prev[i];
            let ul = if i > 0 { prev[i - 1] } else { 0 };
            cur[i] = match kind {
                0 => src[i],
                1 => src[i].wrapping_add(left),
                2 => src[i].wrapping_add(up),
                3 => src[i].wrapping_add(((left as u16 + up as u16) / 2) as u8),
                4 => {
                    let pa = (up as i16 - ul as i16).abs();
                    let pb = (left as i16 - ul as i16).abs();
                    let pc = (left as i16 + up as i16 - 2 * ul as i16).abs();
                    let pred = if pa <= pb && pa <= pc {
                        left
                    } else if pb <= pc {
                        up
                    } else {
                        ul
                    };
                    src[i].wrapping_add(pred)
                }
                _ => return damaged("bad PNG predictor"),
            };
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    Ok(out)
}
