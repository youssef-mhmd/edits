//! Copying objects out of a file: pages taken from another PDF, and a
//! whole document written anew with only what is still in use.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;

use super::file::{PdfError, PdfFile, Source};
use super::object::{Dict, Obj, Ref, write};

/// The generation that marks a copier's own object numbers until they are
/// given real ones (`place`).
pub const LOCAL: u16 = u16::MAX;

/// Objects copied from one file, numbered on their own: local object `i` is
/// referred to as `(i, LOCAL)`. Objects shared by several copied pages
/// (fonts, images) are copied once.
#[derive(Default)]
pub struct Copier {
    map: HashMap<u32, u32>,
    pub objects: Vec<Obj>,
}

impl Copier {
    /// Copies `o`, a value from `src`, and everything it refers to. A
    /// reference for which `stop` returns a value is replaced by that value
    /// instead of being followed.
    pub fn copy<S: Source>(
        &mut self,
        src: &PdfFile<S>,
        o: &Obj,
        stop: &mut dyn FnMut(Ref) -> Option<Obj>,
    ) -> Result<Obj, PdfError> {
        let mut work = Vec::new();
        let out = self.convert(o, stop, &mut work);
        while let Some((r, local)) = work.pop() {
            // An object that cannot be read is null, as readers treat it.
            let got = src.get(r).unwrap_or(Obj::Null);
            let c = self.convert(&got, stop, &mut work);
            self.objects[local as usize] = c;
        }
        Ok(out)
    }

    /// Reserves a local object (to be filled with `set`).
    pub fn reserve(&mut self) -> Ref {
        self.objects.push(Obj::Null);
        ((self.objects.len() - 1) as u32, LOCAL)
    }

    pub fn set(&mut self, r: Ref, o: Obj) {
        if r.1 == LOCAL
            && let Some(slot) = self.objects.get_mut(r.0 as usize)
        {
            *slot = o;
        }
    }

    /// The local number source object `n` was copied to, if it was.
    pub fn local_of(&self, n: u32) -> Option<Ref> {
        self.map.get(&n).map(|l| (*l, LOCAL))
    }

    fn convert(&mut self, o: &Obj, stop: &mut dyn FnMut(Ref) -> Option<Obj>, work: &mut Vec<(Ref, u32)>) -> Obj {
        match o {
            Obj::Ref(r) => {
                if let Some(v) = stop(*r) {
                    return v;
                }
                if let Some(l) = self.map.get(&r.0) {
                    return Obj::Ref((*l, LOCAL));
                }
                let l = self.objects.len() as u32;
                self.objects.push(Obj::Null);
                self.map.insert(r.0, l);
                work.push((*r, l));
                Obj::Ref((l, LOCAL))
            }
            Obj::Array(a) => Obj::Array(a.iter().map(|v| self.convert(v, stop, work)).collect()),
            Obj::Dict(d) => Obj::Dict(self.convert_dict(d, stop, work)),
            Obj::Stream(d, data) => {
                let mut d = d.clone();
                // Written again from the data.
                d.remove(b"Length");
                Obj::Stream(self.convert_dict(&d, stop, work), data.clone())
            }
            other => other.clone(),
        }
    }

    fn convert_dict(&mut self, d: &Dict, stop: &mut dyn FnMut(Ref) -> Option<Obj>, work: &mut Vec<(Ref, u32)>) -> Dict {
        Dict(d.0.iter().map(|(k, v)| (k.clone(), self.convert(v, stop, work))).collect())
    }
}

/// Gives local references real object numbers: local `i` becomes `start + i`.
pub fn place(o: &mut Obj, start: u32) {
    match o {
        Obj::Ref(r) if r.1 == LOCAL => *r = (start + r.0, 0),
        Obj::Array(a) => a.iter_mut().for_each(|v| place(v, start)),
        Obj::Dict(d) | Obj::Stream(d, _) => d.0.iter_mut().for_each(|(_, v)| place(v, start)),
        _ => {}
    }
}

/// The page tree's nodes and pages, as reached from the catalog.
fn live_tree<S: Source>(f: &PdfFile<S>) -> Result<(HashSet<u32>, HashSet<u32>), PdfError> {
    let root = f.resolve(f.trailer.get(b"Root").unwrap_or(&Obj::Null))?;
    let mut nodes = HashSet::new();
    let pages: HashSet<u32> = f.pages()?.iter().map(|r| r.0).collect();
    let mut stack: Vec<Ref> = root.as_dict().and_then(|d| d.get(b"Pages")).and_then(Obj::as_ref).into_iter().collect();
    while let Some(r) = stack.pop() {
        if pages.contains(&r.0) || !nodes.insert(r.0) || nodes.len() > 1_000_000 {
            continue;
        }
        if let Some(kids) = f.get(r)?.as_dict().and_then(|d| d.get(b"Kids")).and_then(Obj::as_array) {
            stack.extend(kids.iter().filter_map(Obj::as_ref));
        }
    }
    Ok((nodes, pages))
}

/// Writes the whole document in `f` to `out` as a new file holding only the
/// objects still in use: pages no longer in the page tree, and old versions
/// of changed objects, are left behind (their content is really gone).
/// References to dropped pages become null. Returns the bytes written.
pub fn rewrite<S: Source>(f: &PdfFile<S>, out: &mut dyn Write) -> Result<u64, PdfError> {
    let (nodes, pages) = live_tree(f)?;
    let mut w = Counting { inner: out, n: 0 };
    w.write_all(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")?;
    let mut map: HashMap<u32, u32> = HashMap::new();
    let mut offsets: Vec<u64> = vec![0];
    // First come, first numbered, first written: objects come out in order.
    let mut work: VecDeque<(Ref, u32)> = VecDeque::new();
    let mut next = 1u32;
    let mut number = |r: Ref, work: &mut VecDeque<(Ref, u32)>, map: &mut HashMap<u32, u32>| -> u32 {
        *map.entry(r.0).or_insert_with(|| {
            let n = next;
            next += 1;
            work.push_back((r, n));
            n
        })
    };
    let mut trailer = Dict::default();
    // A protected file stays protected the same way (same key and ID).
    let mut enc_obj = None;
    for key in [&b"Root"[..], b"Info", b"Encrypt"] {
        match f.trailer.get(key) {
            Some(Obj::Ref(r)) => {
                let n = number(*r, &mut work, &mut map);
                trailer.set(key, Obj::Ref((n, 0)));
                if key == b"Encrypt" {
                    enc_obj = Some(n);
                }
            }
            Some(d @ Obj::Dict(_)) if key == b"Encrypt" => trailer.set(key, d.clone()),
            _ => {}
        }
    }
    if let Some(id) = f.trailer.get(b"ID") {
        trailer.set(b"ID", f.resolve(id)?);
    }
    let mut buf = Vec::new();
    while let Some((r, n)) = work.pop_front() {
        // A file with objects that cannot be read is not written anew: what
        // they hold would be lost.
        let got = f.get(r)?;
        let dead = got.as_dict().is_some_and(|d| match d.name(b"Type") {
            Some(b"Page") => !pages.contains(&r.0),
            Some(b"Pages") => !nodes.contains(&r.0),
            _ => false,
        });
        let o = if dead { Obj::Null } else { renumber(&got, &mut |r| number(r, &mut work, &mut map)) };
        let o = match &f.crypt {
            Some(c) if Some(n) != enc_obj => c.encrypt_renumbered((n, 0), o),
            _ => o,
        };
        debug_assert_eq!(n as usize, offsets.len());
        offsets.push(w.n);
        buf.clear();
        buf.extend_from_slice(
            format!(
                "{n} 0 obj
"
            )
            .as_bytes(),
        );
        write(&o, &mut buf);
        buf.extend_from_slice(
            b"
endobj
",
        );
        w.write_all(&buf)?;
    }
    let size = offsets.len();
    let xref_at = w.n;
    let mut x = format!("xref\n0 {size}\n0000000000 65535 f\r\n");
    for off in &offsets[1..] {
        x.push_str(&format!("{off:010} 00000 n\r\n"));
    }
    w.write_all(x.as_bytes())?;
    trailer.set(b"Size", Obj::Int(size as i64));
    buf.clear();
    buf.extend_from_slice(b"trailer\n");
    write(&Obj::Dict(trailer), &mut buf);
    buf.extend_from_slice(format!("\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    w.write_all(&buf)?;
    w.flush()?;
    Ok(w.n)
}

/// `o` with every reference given the number `num` returns for it.
fn renumber(o: &Obj, num: &mut dyn FnMut(Ref) -> u32) -> Obj {
    match o {
        Obj::Ref(r) => Obj::Ref((num(*r), 0)),
        Obj::Array(a) => Obj::Array(a.iter().map(|v| renumber(v, num)).collect()),
        Obj::Dict(d) => Obj::Dict(Dict(d.0.iter().map(|(k, v)| (k.clone(), renumber(v, num))).collect())),
        Obj::Stream(d, data) => {
            let mut d = d.clone();
            d.remove(b"Length");
            let Obj::Dict(d) = renumber(&Obj::Dict(d), num) else { unreachable!() };
            Obj::Stream(d, data.clone())
        }
        other => other.clone(),
    }
}

struct Counting<'a> {
    inner: &'a mut dyn Write,
    n: u64,
}

impl Write for Counting<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let k = self.inner.write(b)?;
        self.n += k as u64;
        Ok(k)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rewrite_keeps_what_is_used_and_reads_back() {
        let f = PdfFile::open(crate::pdf::write::tiny_pdf()).unwrap();
        let mut out = Vec::new();
        rewrite(&f, &mut out).unwrap();
        let g = PdfFile::open(out).unwrap();
        assert_eq!(g.pages().unwrap().len(), 1);
        let (_, _, annots) = g.page_annots(0).unwrap();
        assert_eq!(annots.len(), 1);
        let a = g.resolve(&annots[0]).unwrap();
        assert_eq!(a.as_dict().unwrap().name(b"Subtype"), Some(&b"Square"[..]));
    }

    #[test]
    fn copies_share_objects_and_are_placed() {
        let f = PdfFile::open(crate::pdf::write::tiny_pdf()).unwrap();
        let mut c = Copier::default();
        let mut stop = |r: Ref| (r.0 == 2).then_some(Obj::Null);
        let page = f.get((3, 0)).unwrap();
        let a = c.copy(&f, &page, &mut stop).unwrap();
        let b = c.copy(&f, &Obj::Ref((4, 0)), &mut stop).unwrap();
        assert_eq!(c.objects.len(), 1, "the annotation is copied once");
        assert_eq!(b, Obj::Ref((0, LOCAL)));
        let mut a = a;
        place(&mut a, 10);
        let d = a.as_dict().unwrap();
        assert_eq!(d.get(b"Parent"), Some(&Obj::Null));
        assert_eq!(d.get(b"Annots"), Some(&Obj::Array(vec![Obj::Ref((10, 0))])));
    }
}
