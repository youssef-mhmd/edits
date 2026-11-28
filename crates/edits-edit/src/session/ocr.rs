//! Text recognised on scanned pages, written as an invisible text layer
//! (PRD 5.7) so that the page can be searched, selected and copied.
//!
//! The method Tesseract, OCRmyPDF and arabic-ocr use: each word is written
//! in text render mode 3 (invisible), stretched to cover the word on the
//! page, in a "glyphless" font whose glyphs are empty. Character codes are
//! Unicode code points (CID = code point) with a ToUnicode map back, so any
//! letter works without embedding a real font. Words are written in display
//! order, left to right as they appear on the page (arabic-ocr's measured
//! choice: PDFium and MuPDF both rebuild right-to-left text from positions).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::pages::Out;
use super::{EditError, Session};
use crate::pdf::object::real;
use crate::pdf::{Dict, Obj, Ref};

/// One word in PDF user space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrWord {
    /// Corners: top-left, top-right, bottom-right, bottom-left, as the word
    /// is read (top-left to top-right is the direction of the text).
    pub quad: [[f32; 2]; 4],
    /// The characters in display order, left to right as drawn (for Arabic
    /// the logical text reversed with numbers kept left to right), followed
    /// by a space.
    pub shown: String,
}

/// The text layer of one page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OcrLayer {
    pub words: Vec<OcrWord>,
}

/// Glyph advance in the glyphless font, in 1/1000 of the font size.
const GLYPH_WIDTH: f32 = 500.0;

/// A TrueType font with two empty glyphs, each GLYPH_WIDTH wide, ascent
/// 1000 and descent 0 (made with fontTools, as arabic-ocr makes it).
const GLYPHLESS: &[u8] = include_bytes!("../glyphless.ttf");

const TO_UNICODE: &[u8] = b"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
1 beginbfrange
<0000> <FFFF> <0000>
endbfrange
endcmap
CMapName currentdict /CMap defineresource pop
end
end
";

/// Combining marks of the Arabic script (and the general ones): drawn
/// with no advance, over the middle of the letter they belong to.
const MARKS: [(u32, u32); 9] = [
    (0x0300, 0x036F),
    (0x0610, 0x061A),
    (0x064B, 0x065F),
    (0x0670, 0x0670),
    (0x06D6, 0x06DC),
    (0x06DF, 0x06E4),
    (0x06E7, 0x06E8),
    (0x06EA, 0x06ED),
    (0x08D3, 0x08FF),
];

fn is_mark(c: char) -> bool {
    MARKS.iter().any(|(a, b)| (*a..=*b).contains(&(c as u32)))
}

/// The font's resource name on pages.
pub(super) const FONT: &str = "EditsOcr";

/// The glyphless Type0 font's objects; returns the font.
pub(super) fn font_objects(out: &mut Out) -> Ref {
    let mut file = Dict::default();
    file.set(b"Length1", Obj::Int(GLYPHLESS.len() as i64));
    file.set(b"Filter", Obj::name("FlateDecode"));
    let file = out.add(Obj::Stream(file, super::deflate(GLYPHLESS)));
    let mut desc = Dict::default();
    desc.set(b"Type", Obj::name("FontDescriptor"));
    desc.set(b"FontName", Obj::name("GlyphLessFont"));
    desc.set(b"Flags", Obj::Int(5));
    desc.set(b"FontBBox", Obj::Array(vec![Obj::Int(0), Obj::Int(0), Obj::Int(GLYPH_WIDTH as i64), Obj::Int(1000)]));
    desc.set(b"ItalicAngle", Obj::Int(0));
    desc.set(b"Ascent", Obj::Int(1000));
    desc.set(b"Descent", Obj::Int(0));
    desc.set(b"CapHeight", Obj::Int(1000));
    desc.set(b"StemV", Obj::Int(80));
    desc.set(b"FontFile2", Obj::Ref(file));
    let desc = out.add(Obj::Dict(desc));
    let mut info = Dict::default();
    info.set(b"Registry", Obj::Str(b"Adobe".to_vec()));
    info.set(b"Ordering", Obj::Str(b"Identity".to_vec()));
    info.set(b"Supplement", Obj::Int(0));
    // Every CID (code point) draws glyph 1, the empty one.
    let mut map = Dict::default();
    map.set(b"Filter", Obj::name("FlateDecode"));
    let map = out.add(Obj::Stream(map, super::deflate(&[0u8, 1].repeat(65536))));
    let mut cid = Dict::default();
    cid.set(b"Type", Obj::name("Font"));
    cid.set(b"Subtype", Obj::name("CIDFontType2"));
    cid.set(b"BaseFont", Obj::name("GlyphLessFont"));
    cid.set(b"CIDSystemInfo", Obj::Dict(info));
    cid.set(b"FontDescriptor", Obj::Ref(desc));
    cid.set(b"DW", Obj::Int(GLYPH_WIDTH as i64));
    let mut widths = Vec::new();
    for (a, b) in MARKS {
        widths.extend([Obj::Int(a as i64), Obj::Int(b as i64), Obj::Int(0)]);
    }
    cid.set(b"W", Obj::Array(widths));
    cid.set(b"CIDToGIDMap", Obj::Ref(map));
    let cid = out.add(Obj::Dict(cid));
    let mut tu = Dict::default();
    tu.set(b"Filter", Obj::name("FlateDecode"));
    let tu = out.add(Obj::Stream(tu, super::deflate(TO_UNICODE)));
    let mut t0 = Dict::default();
    t0.set(b"Type", Obj::name("Font"));
    t0.set(b"Subtype", Obj::name("Type0"));
    t0.set(b"BaseFont", Obj::name("GlyphLessFont"));
    t0.set(b"Encoding", Obj::name("Identity-H"));
    t0.set(b"DescendantFonts", Obj::Array(vec![Obj::Ref(cid)]));
    t0.set(b"ToUnicode", Obj::Ref(tu));
    out.add(Obj::Dict(t0))
}

/// A word's characters for TJ: letters advance by GLYPH_WIDTH; the marks
/// that come before a letter in display order (its own marks, the word
/// being reversed) are drawn with no advance over its middle, so readers
/// that place marks by position find them on their letter. Returns the
/// array and the number of letters.
fn shown_codes(shown: &str) -> (String, usize) {
    let half = GLYPH_WIDTH / 2.0;
    let mut out = String::from("[");
    let mut n = 0;
    let mut in_marks = false;
    for c in shown.chars().filter(|c| (*c as u32) <= 0xFFFF) {
        let mark = is_mark(c);
        if mark && !in_marks {
            out += &format!("-{half} ");
        } else if !mark && in_marks {
            out += &format!("{half} ");
        }
        in_marks = mark;
        out += &format!("<{:04X}> ", c as u32);
        n += usize::from(!mark);
    }
    if in_marks {
        out += &format!("{half} ");
    }
    out.pop();
    out.push(']');
    (out, n)
}

/// The content stream drawn after the page's own (which is wrapped in
/// q/Q): every word, invisible, its font size the box height and stretched
/// to the box width. Coordinates are user space.
pub(super) fn content(layer: &OcrLayer) -> Vec<u8> {
    let mut s = String::from("Q\nq\nBT\n3 Tr\n");
    for w in &layer.words {
        let (codes, n) = shown_codes(&w.shown);
        if n == 0 {
            continue;
        }
        let [tl, tr, _, bl] = w.quad;
        let (dx, dy) = (tr[0] - tl[0], tr[1] - tl[1]);
        let width = dx.hypot(dy);
        let height = (bl[0] - tl[0]).hypot(bl[1] - tl[1]);
        if !(width >= 0.1 && height >= 0.1 && width.is_finite() && height.is_finite()) {
            continue;
        }
        let (cos, sin) = (dx / width, dy / width);
        let stretch = 100.0 * width / (n as f32 * GLYPH_WIDTH / 1000.0 * height);
        let r = |v: f32| real(v as f64);
        s += &format!(
            "/{FONT} {} Tf {} Tz {} {} {} {} {} {} Tm {codes} TJ\n",
            r(height),
            r(stretch),
            r(cos),
            r(sin),
            r(-sin),
            r(cos),
            r(bl[0]),
            r(bl[1]),
        );
    }
    s += "ET\nQ\n";
    s.into_bytes()
}

impl Session {
    /// Gives pages (indexes) the text recognised on them, replacing any
    /// layer edits gave them before. `run` names one OCR run: while the
    /// last change is the same run's, pages are added to that change, so a
    /// whole run is undone in one step.
    pub fn set_page_text(&mut self, run: u64, layers: Vec<(u32, OcrLayer)>) -> Result<(), EditError> {
        let idx: Vec<u32> = layers.iter().map(|l| l.0).collect();
        self.checked(&idx)?;
        let mut order = self.order().to_vec();
        for (i, layer) in layers {
            self.ocr.push(Arc::new(layer));
            order[i as usize].ocr = Some(self.ocr.len() as u32 - 1);
        }
        let same_run = self.ocr_run.is_some_and(|(r, seq)| r == run && self.undo.last().is_some_and(|u| u.0 == seq));
        // Pages keep their size and place: no new page list to show.
        let layout = self.layout_changed;
        let r = if same_run { self.replace_order(order) } else { self.set_order(order) };
        self.layout_changed = layout;
        r?;
        if !same_run {
            self.ocr_run = self.undo.last().map(|u| (run, u.0));
        }
        Ok(())
    }

    /// Takes away the text layers edits added to `pages` since the file was
    /// last saved.
    pub fn remove_page_text(&mut self, pages: &[u32]) -> Result<(), EditError> {
        let set = self.checked(pages)?;
        let mut order = self.order().to_vec();
        for i in set {
            order[i].ocr = None;
        }
        self.set_order(order)
    }

    /// The pages' keys in order: it changes when pages are added, removed or
    /// moved, so work begun on page indexes can tell they still hold.
    pub fn page_keys(&self) -> Vec<u32> {
        self.order().iter().map(|s| s.key).collect()
    }

    /// True if edits added a text layer to the page at `index`.
    pub fn has_page_text(&self, index: u32) -> bool {
        self.order().get(index as usize).is_some_and(|s| s.ocr.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_stretched_over_their_box() {
        let layer = OcrLayer {
            words: vec![OcrWord {
                quad: [[100.0, 700.0], [200.0, 700.0], [200.0, 680.0], [100.0, 680.0]],
                shown: "ba ".into(),
            }],
        };
        let s = String::from_utf8(content(&layer)).unwrap();
        // 3 glyphs of 0.5 em at 20 pt = 30 pt, stretched to 100 pt.
        assert!(s.contains("/EditsOcr 20 Tf 333.33"), "{s}");
        assert!(s.contains(" 100 680 Tm [<0062> <0061> <0020>] TJ"), "{s}");
        assert!(s.starts_with("Q\nq\nBT\n3 Tr\n") && s.ends_with("ET\nQ\n"));
    }

    #[test]
    fn marks_sit_on_their_letter() {
        // "لَى " reversed for display: space, alef maksura, fatha, lam.
        let (codes, n) = shown_codes(" \u{649}\u{64E}\u{644}");
        assert_eq!(n, 3, "marks take no room");
        assert_eq!(codes, "[<0020> <0649> -250 <064E> 250 <0644>]");
        // Two marks, then the letter they belong to.
        let (codes, _) = shown_codes("\u{64E}\u{651}\u{644}");
        assert_eq!(codes, "[-250 <064E> <0651> 250 <0644>]");
    }
}
