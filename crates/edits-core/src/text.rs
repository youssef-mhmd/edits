//! Page text: characters with their boxes, and search with Arabic-aware
//! matching.
//!
//! PDFium's character order is not reliable for right-to-left text (words
//! often come out reversed, diacritics on lines of their own), so the text is
//! rebuilt in reading order from glyph positions (see `reading`). Every
//! character is one UTF-16 unit, with characters outside the Basic
//! Multilingual Plane replaced by U+FFFD, so indices line up in JavaScript
//! too. `boxes[4 * i..4 * i + 4]` is the box of character `i`.

use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// Text of one page. `boxes` has four numbers per character: left, top,
/// right, bottom as page fractions (as for links); all zero for spaces and
/// line breaks that were inserted.
#[derive(Debug, Clone, Serialize)]
pub struct PageText {
    pub text: String,
    pub boxes: Vec<f32>,
}

/// Characters of a page in reading order, without boxes: what the text index stores.
pub fn page_chars(page: &PdfPage) -> String {
    page_text(page).text
}

fn one_unit(u: u32) -> char {
    match char::from_u32(u) {
        Some(c) if (c as u32) <= 0xFFFF => c,
        _ => '\u{FFFD}',
    }
}

/// PDF user space to page fractions (top-left origin, /Rotate applied).
struct PageMap {
    // fx = a*x + b*y + c, fy = d*x + e*y + f
    m: [f64; 6],
}

impl PageMap {
    fn new(page: &PdfPage) -> Option<Self> {
        const SIZE: f64 = 1_000_000.0;
        let config = PdfRenderConfig::new().set_fixed_size(SIZE as Pixels, SIZE as Pixels);
        // Probe far apart so whole-pixel results still give a precise mapping.
        let probe = |x: f32, y: f32| -> Option<(f64, f64)> {
            let (dx, dy) = page.points_to_pixels(PdfPoints::new(x), PdfPoints::new(y), &config).ok()?;
            Some((dx as f64 / SIZE, dy as f64 / SIZE))
        };
        let (o, ox, oy) = (probe(0.0, 0.0)?, probe(1000.0, 0.0)?, probe(0.0, 1000.0)?);
        Some(Self {
            m: [(ox.0 - o.0) / 1000.0, (oy.0 - o.0) / 1000.0, o.0, (ox.1 - o.1) / 1000.0, (oy.1 - o.1) / 1000.0, o.1],
        })
    }

    fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let (x, y) = (x as f64, y as f64);
        let m = &self.m;
        let fx = (m[0] * x + m[1] * y + m[2]).clamp(0.0, 1.0);
        let fy = (m[3] * x + m[4] * y + m[5]).clamp(0.0, 1.0);
        (fx as f32, fy as f32)
    }
}

/// Characters of a page in reading order, with their boxes, for selection,
/// highlighting and search.
pub fn page_text(page: &PdfPage) -> PageText {
    let Ok(text) = page.text() else { return PageText { text: String::new(), boxes: Vec::new() } };
    let chars = text.chars();
    // Page fractions through PDFium's own page-to-device mapping (handles
    // /Rotate and crop offsets). The mapping is affine, so it is sampled at
    // three points once and then applied as arithmetic: one PDFium call per
    // character is the cost that matters when indexing thousands of pages.
    let Some(map) = PageMap::new(page) else { return PageText { text: String::new(), boxes: Vec::new() } };
    let to_frac = |x: f32, y: f32| -> Option<(f32, f32)> { Some(map.apply(x, y)) };
    let (pw, ph) = (page.width().value.max(1.0), page.height().value.max(1.0));
    let mut fracs = Vec::with_capacity(chars.len());
    let mut glyphs = Vec::with_capacity(chars.len());
    for c in chars.iter() {
        let generated = c.is_generated().unwrap_or(false);
        let frac = if generated { None } else { c.loose_bounds().ok() }.and_then(|r| {
            let a = to_frac(r.left().value, r.top().value)?;
            let b = to_frac(r.right().value, r.bottom().value)?;
            Some([a.0.min(b.0), a.1.min(b.1), a.0.max(b.0), a.1.max(b.1)])
        });
        fracs.push(frac);
        // Geometry in points from the top-left, so gaps compare with font sizes.
        let rect = frac.map(|f| [f[0] * pw, f[1] * ph, f[2] * pw, f[3] * ph]);
        let baseline = c
            .origin()
            .ok()
            .and_then(|(x, y)| to_frac(x.value, y.value))
            .map(|(_, fy)| fy * ph)
            .or(rect.map(|r| r[3]))
            .unwrap_or(0.0);
        glyphs.push(crate::reading::Glyph {
            c: one_unit(c.unicode_value()),
            rect,
            baseline,
            // Some producers set font size 1 and scale with the text matrix;
            // never trust a size smaller than the glyph's own box.
            size: c.scaled_font_size().value.abs().max(rect.map_or(1.0, |r| r[3] - r[1])).max(1.0),
        });
    }
    let rebuilt = crate::reading::rebuild(&glyphs);
    // Four decimals is a tenth of a point on an A4 page; keeps the JSON small.
    let round = |v: f32| (v * 10_000.0).round() / 10_000.0;
    let mut boxes = Vec::with_capacity(rebuilt.text.len() * 4);
    for src in &rebuilt.source {
        boxes.extend(src.and_then(|g| fracs[g]).unwrap_or([0.0; 4]).map(round));
    }
    PageText { text: rebuilt.text.into_iter().collect(), boxes }
}

// ------------------------------------------------------------------ search

/// How text is matched.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    /// Ignore Arabic diacritics (harakat, superscript alef, Quranic marks) and tatweel.
    pub ignore_diacritics: bool,
    /// Treat أ إ آ ٱ as ا, ى and ی as ي, ة as ه, ک as ك, and Arabic-Indic digits as 0-9.
    pub letter_variants: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self { match_case: false, whole_word: false, ignore_diacritics: true, letter_variants: true }
    }
}

fn is_arabic_mark(c: char) -> bool {
    matches!(c as u32, 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E8 | 0x06EA..=0x06ED)
        || c == '\u{0640}' // tatweel
}

fn variant(c: char) -> char {
    match c {
        'أ' | 'إ' | 'آ' | 'ٱ' => 'ا',
        'ى' | 'ی' => 'ي',
        'ة' => 'ه',
        'ک' => 'ك',
        '٠'..='٩' => char::from_u32(c as u32 - 0x0660 + '0' as u32).unwrap_or(c),
        '۰'..='۹' => char::from_u32(c as u32 - 0x06F0 + '0' as u32).unwrap_or(c),
        _ => c,
    }
}

fn is_presentation_form(c: char) -> bool {
    matches!(c as u32, 0xFB50..=0xFDFF | 0xFE70..=0xFEFF | 0xFB00..=0xFB06)
}

/// Normalises text for matching. Returns the normalised characters and, for
/// each, the index of the original character it came from.
pub fn normalize(text: &str, o: &SearchOptions) -> (Vec<char>, Vec<u32>) {
    let mut out = Vec::with_capacity(text.len());
    let mut map = Vec::with_capacity(text.len());
    let mut last_space = true; // drop leading whitespace
    for (i, c) in text.chars().enumerate() {
        let i = i as u32;
        // Presentation forms (and Latin ligatures) become their plain letters.
        let mut expanded: Vec<char> = if is_presentation_form(c) { c.nfkc().collect() } else { vec![c] };
        if !o.match_case {
            expanded = expanded.into_iter().flat_map(char::to_lowercase).collect();
        }
        for c in expanded {
            if c.is_whitespace() || c == '\u{FFFE}' {
                if !last_space {
                    out.push(' ');
                    map.push(i);
                    last_space = true;
                }
                continue;
            }
            if o.ignore_diacritics && is_arabic_mark(c) {
                continue;
            }
            out.push(if o.letter_variants { variant(c) } else { c });
            map.push(i);
            last_space = false;
        }
    }
    (out, map)
}

/// Matches of `query` in `text` as ranges of original character indices
/// (start inclusive, end exclusive), in text order, at most `limit`.
pub fn find(text: &str, query: &str, o: &SearchOptions, limit: usize) -> Vec<[u32; 2]> {
    let (q, _) = normalize(query.trim(), o);
    if q.is_empty() {
        return Vec::new();
    }
    let (t, map) = normalize(text, o);
    let original: Vec<char> = text.chars().collect();
    let word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || is_arabic_mark(*c));
    let mut out = Vec::new();
    let mut i = 0;
    while i + q.len() <= t.len() && out.len() < limit {
        if t[i..i + q.len()] == q[..]
            && (!o.whole_word || (!word(i.checked_sub(1).and_then(|p| t.get(p))) && !word(t.get(i + q.len()))))
        {
            // Take in marks that follow the last letter, so the highlight covers the word as written.
            let mut end = map[i + q.len() - 1] as usize + 1;
            while o.ignore_diacritics && original.get(end).is_some_and(|c| is_arabic_mark(*c)) {
                end += 1;
            }
            out.push([map[i], end as u32]);
            i += q.len();
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o() -> SearchOptions {
        SearchOptions::default()
    }

    #[test]
    fn ignores_diacritics_and_tatweel() {
        let text = "العِلْمُ نُورٌ والســلام";
        assert_eq!(find(text, "العلم", &o(), 10), vec![[0, 8]]);
        assert_eq!(find(text, "السلام", &o(), 10).len(), 1);
        let strict = SearchOptions { ignore_diacritics: false, ..o() };
        assert!(find(text, "العلم", &strict, 10).is_empty());
    }

    #[test]
    fn letter_variants_by_option() {
        let text = "أحمد إبراهيم آمنة مستشفى مدرسة ١٢٣";
        assert_eq!(find(text, "احمد", &o(), 10).len(), 1);
        assert_eq!(find(text, "ابراهيم", &o(), 10).len(), 1);
        assert_eq!(find(text, "امنه", &o(), 10).len(), 1);
        assert_eq!(find(text, "مستشفي", &o(), 10).len(), 1);
        assert_eq!(find(text, "123", &o(), 10).len(), 1);
        let strict = SearchOptions { letter_variants: false, ..o() };
        assert!(find(text, "احمد", &strict, 10).is_empty());
        assert_eq!(find(text, "أحمد", &strict, 10).len(), 1);
    }

    #[test]
    fn presentation_forms_match_plain_letters() {
        // "سلام" written with presentation forms, as some PDFs store it.
        let text = "\u{FEB3}\u{FEE0}\u{FE8E}\u{FEE1}";
        assert_eq!(find(text, "سلام", &o(), 10), vec![[0, 4]]);
        // Lam-alef ligature is two letters; both map to the one original character.
        assert_eq!(find("\u{FEFB}", "لا", &o(), 10), vec![[0, 1]]);
    }

    #[test]
    fn case_words_and_whitespace() {
        let text = "The cat\r\nscattered  Cats";
        assert_eq!(find(text, "cat", &o(), 10).len(), 3);
        let whole = SearchOptions { whole_word: true, ..o() };
        assert_eq!(find(text, "cat", &whole, 10), vec![[4, 7]]);
        let case = SearchOptions { match_case: true, ..o() };
        assert_eq!(find(text, "Cat", &case, 10).len(), 1);
        // A line break in the text matches a space in the query.
        assert_eq!(find(text, "cat scattered", &o(), 10), vec![[4, 18]]);
        assert!(find(text, "  ", &o(), 10).is_empty());
    }
}
