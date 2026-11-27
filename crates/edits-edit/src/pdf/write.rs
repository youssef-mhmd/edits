//! Writing an incremental update: changed and new objects after the
//! original bytes, a cross-reference section for them, and a trailer that
//! points back to the previous one.

use super::file::{PdfFile, Source};
use super::object::{Dict, Obj, Ref, write};

/// The bytes to append to `base` so that it holds `objects` (new objects,
/// or new versions of existing ones). Uses a cross-reference stream if the
/// file's newest section is one, as readers expect.
pub fn update<S: Source>(base: &PdfFile<S>, objects: &[(Ref, Obj)]) -> Vec<u8> {
    let base_len = base.len();
    let mut out = Vec::new();
    // The original may not end with a line break.
    out.push(b'\n');
    let mut offsets: Vec<(u32, u16, u64)> = Vec::with_capacity(objects.len() + 1);
    for ((n, g), o) in objects {
        offsets.push((*n, *g, base_len + out.len() as u64));
        out.extend_from_slice(format!("{n} {g} obj\n").as_bytes());
        write(o, &mut out);
        out.extend_from_slice(b"\nendobj\n");
    }
    let size = objects.iter().map(|((n, _), _)| n + 1).max().unwrap_or(0).max(base.size);

    let mut trailer = Dict::default();
    for key in [&b"Root"[..], b"Info", b"ID"] {
        if let Some(v) = base.trailer.get(key) {
            trailer.set(key, v.clone());
        }
    }
    trailer.set(b"Prev", Obj::Int(base.startxref as i64));

    let xref_at = base_len + out.len() as u64;
    if base.xref_stream {
        // The stream itself takes the next free number.
        let own = size;
        offsets.push((own, 0, xref_at));
        offsets.sort_by_key(|e| e.0);
        let width = if xref_at > u32::MAX as u64 { 8 } else { 4 };
        let mut data = Vec::new();
        let mut index = Vec::new();
        for run in runs(&offsets) {
            index.push(Obj::Int(run[0].0 as i64));
            index.push(Obj::Int(run.len() as i64));
            for &(_, g, off) in run {
                data.push(1u8);
                data.extend_from_slice(&off.to_be_bytes()[8 - width..]);
                data.extend_from_slice(&g.to_be_bytes());
            }
        }
        trailer.set(b"Type", Obj::name("XRef"));
        trailer.set(b"Size", Obj::Int(own as i64 + 1));
        trailer.set(b"W", Obj::Array(vec![Obj::Int(1), Obj::Int(width as i64), Obj::Int(2)]));
        trailer.set(b"Index", Obj::Array(index));
        out.extend_from_slice(format!("{own} 0 obj\n").as_bytes());
        write(&Obj::Stream(trailer, data), &mut out);
        out.extend_from_slice(b"\nendobj\n");
    } else {
        offsets.sort_by_key(|e| e.0);
        out.extend_from_slice(b"xref\n");
        for run in runs(&offsets) {
            out.extend_from_slice(format!("{} {}\n", run[0].0, run.len()).as_bytes());
            for &(_, g, off) in run {
                out.extend_from_slice(format!("{off:010} {g:05} n\r\n").as_bytes());
            }
        }
        trailer.set(b"Size", Obj::Int(size as i64));
        out.extend_from_slice(b"trailer\n");
        write(&Obj::Dict(trailer), &mut out);
        out.push(b'\n');
    }
    out.extend_from_slice(format!("startxref\n{xref_at}\n%%EOF\n").as_bytes());
    out
}

/// Consecutive object numbers, grouped.
fn runs(sorted: &[(u32, u16, u64)]) -> Vec<&[(u32, u16, u64)]> {
    let mut out = Vec::new();
    let mut start = 0;
    for i in 1..=sorted.len() {
        if i == sorted.len() || sorted[i].0 != sorted[i - 1].0 + 1 {
            if start < i {
                out.push(&sorted[start..i]);
            }
            start = i;
        }
    }
    out
}

/// A minimal one-page PDF (200 x 100 pt) with a classic cross-reference
/// table and one red square annotation, for tests.
#[cfg(test)]
pub(crate) fn tiny_pdf() -> Vec<u8> {
    tests::tiny_pdf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::file::PdfFile;

    pub fn tiny_pdf() -> Vec<u8> {
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Annots [4 0 R] >>",
            "<< /Type /Annot /Subtype /Square /Rect [10 10 50 50] /C [1 0 0] >>",
        ];
        let mut b = b"%PDF-1.7\n".to_vec();
        let mut offs = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offs.push(b.len());
            b.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let x = b.len();
        b.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", objs.len() + 1).as_bytes());
        for o in offs {
            b.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
        }
        b.extend_from_slice(format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{x}\n%%EOF").as_bytes());
        b
    }

    #[test]
    fn reads_pages_and_annotations() {
        let f = PdfFile::open(tiny_pdf()).unwrap();
        assert_eq!(f.pages().unwrap(), vec![(3, 0)]);
        let (r, _, annots) = f.page_annots(0).unwrap();
        assert_eq!(r, (3, 0));
        assert_eq!(annots, vec![Obj::Ref((4, 0))]);
        let a = f.get((4, 0)).unwrap();
        assert_eq!(a.as_dict().unwrap().name(b"Subtype"), Some(&b"Square"[..]));
    }

    #[test]
    fn an_update_is_read_back_and_chains_to_the_original() {
        let base = tiny_pdf();
        let f = PdfFile::open(base.clone()).unwrap();
        let (page_ref, mut page, mut annots) = f.page_annots(0).unwrap();
        let new = (f.size, 0);
        annots.push(Obj::Ref(new));
        page.set(b"Annots", Obj::Array(annots));
        let mut circle = Dict::default();
        circle.set(b"Type", Obj::name("Annot"));
        circle.set(b"Subtype", Obj::name("Circle"));
        let tail = update(&f, &[(page_ref, Obj::Dict(page)), (new, Obj::Dict(circle))]);
        let mut whole = base.clone();
        whole.extend_from_slice(&tail);
        let g = PdfFile::open(whole).unwrap();
        let (_, _, annots) = g.page_annots(0).unwrap();
        assert_eq!(annots.len(), 2);
        assert_eq!(g.get(new).unwrap().as_dict().unwrap().name(b"Subtype"), Some(&b"Circle"[..]));
        assert_eq!(g.get((4, 0)).unwrap().as_dict().unwrap().name(b"Subtype"), Some(&b"Square"[..]));
        assert!(!g.xref_stream);
    }

    #[test]
    fn runs_group_consecutive_numbers() {
        let v = [(3, 0, 0), (4, 0, 0), (9, 0, 0)];
        let r = runs(&v);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].len(), 2);
    }
}
