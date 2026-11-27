//! Text for writing into PDFs: shaping with rustybuzz (Arabic letters join
//! and take their contextual forms), the Unicode bidi algorithm for mixed
//! right-to-left and left-to-right text, line breaking, and the pieces a PDF
//! needs to show the result (a subset font, glyph widths, and a ToUnicode
//! map so that copy and search work in other readers).
//!
//! Text is kept in logical order everywhere; only the glyphs placed on a
//! line are in visual order. Arabic is never written without shaping.

use std::collections::BTreeMap;

use rustybuzz::{Direction, Face, UnicodeBuffer};
use unicode_bidi::BidiInfo;

/// The bundled font: IBM Plex Sans Arabic (SIL Open Font License), which
/// covers Arabic and Latin.
pub static FONT_DATA: &[u8] = include_bytes!("../fonts/IBMPlexSansArabic-Regular.ttf");
/// The name the font is embedded under.
pub const FONT_NAME: &str = "IBMPlexSansArabic";

pub struct Font {
    face: Face<'static>,
    pub units_per_em: f32,
    /// Ascent and descent in font units (descent is negative).
    pub ascent: f32,
    pub descent: f32,
    pub cap_height: f32,
    /// Font bounding box in font units.
    pub bbox: [f32; 4],
}

/// The bundled font, parsed once.
pub fn font() -> &'static Font {
    static FONT: std::sync::OnceLock<Font> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        let face = Face::from_slice(FONT_DATA, 0).expect("bundled font parses");
        let b = face.global_bounding_box();
        Font {
            units_per_em: face.units_per_em() as f32,
            ascent: face.ascender() as f32,
            descent: face.descender() as f32,
            cap_height: face.capital_height().unwrap_or(face.ascender()) as f32,
            bbox: [b.x_min as f32, b.y_min as f32, b.x_max as f32, b.y_max as f32],
            face,
        }
    })
}

/// A glyph placed on a line.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// Glyph id in the bundled font.
    pub id: u16,
    /// Pen position, points from the line's left edge and baseline.
    pub x: f32,
    pub y: f32,
    /// The text this glyph stands for (for ToUnicode); empty for the
    /// second and later glyphs of a cluster.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// In visual order, left to right.
    pub glyphs: Vec<Glyph>,
    pub width: f32,
    /// The paragraph reads right to left (the line is right-aligned).
    pub rtl: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub lines: Vec<Line>,
    pub size: f32,
    /// Distance between baselines.
    pub leading: f32,
    /// Widest line.
    pub width: f32,
}

impl Layout {
    /// Height of all lines, from the first line's top to the last's bottom.
    pub fn height(&self) -> f32 {
        let f = font();
        let asc = f.ascent / f.units_per_em * self.size;
        let desc = -f.descent / f.units_per_em * self.size;
        if self.lines.is_empty() { 0.0 } else { asc + desc + self.leading * (self.lines.len() - 1) as f32 }
    }

    /// The box the glyphs' outlines cover on a one-line layout: left, bottom,
    /// right, top, in points from the line's start and baseline. None if no
    /// glyph has ink.
    pub fn ink(&self) -> Option<[f32; 4]> {
        let f = font();
        let k = self.size / f.units_per_em;
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for g in self.lines.first()?.glyphs.iter() {
            let Some(b) = f.face.glyph_bounding_box(ttf_parser::GlyphId(g.id)) else { continue };
            r[0] = r[0].min(g.x + b.x_min as f32 * k);
            r[1] = r[1].min(g.y + b.y_min as f32 * k);
            r[2] = r[2].max(g.x + b.x_max as f32 * k);
            r[3] = r[3].max(g.y + b.y_max as f32 * k);
        }
        (r[0] <= r[2]).then_some(r)
    }

    /// From the top of the text to the first baseline.
    pub fn first_baseline(&self) -> f32 {
        let f = font();
        f.ascent / f.units_per_em * self.size
    }
}

/// Shapes `text` (one direction) and returns glyphs with x offsets from 0,
/// in visual order, and the total advance.
fn shape(text: &str, rtl: bool, size: f32) -> (Vec<Glyph>, f32) {
    let f = font();
    let scale = size / f.units_per_em;
    let mut buf = UnicodeBuffer::new();
    buf.push_str(text);
    buf.set_direction(if rtl { Direction::RightToLeft } else { Direction::LeftToRight });
    buf.guess_segment_properties();
    let out = rustybuzz::shape(&f.face, &[], buf);
    let infos = out.glyph_infos();
    let pos = out.glyph_positions();
    // Cluster starts in logical order, to cut the text of each cluster.
    let mut starts: Vec<usize> = infos.iter().map(|i| i.cluster as usize).collect();
    starts.sort_unstable();
    starts.dedup();
    let next = |c: usize| starts.iter().copied().find(|&s| s > c).unwrap_or(text.len());
    let mut x = 0.0;
    let mut glyphs = Vec::with_capacity(infos.len());
    let mut seen = std::collections::HashSet::new();
    for (i, p) in infos.iter().zip(pos) {
        let c = i.cluster as usize;
        let t = if seen.insert(c) { text.get(c..next(c)).unwrap_or("").to_string() } else { String::new() };
        glyphs.push(Glyph {
            id: i.glyph_id as u16,
            x: x + p.x_offset as f32 * scale,
            y: p.y_offset as f32 * scale,
            text: t,
        });
        x += p.x_advance as f32 * scale;
    }
    (glyphs, x)
}

/// Width of `text` set in one direction.
fn measure(text: &str, rtl: bool, size: f32) -> f32 {
    shape(text, rtl, size).1
}

/// Lays out `text` at `size` points in lines no wider than `max_width`
/// (or unbounded if it is 0). Paragraphs break at '\n'; lines break at spaces,
/// or inside a word longer than the line.
pub fn layout(text: &str, size: f32, max_width: f32) -> Layout {
    let size = size.clamp(1.0, 500.0);
    let leading = size * 1.3;
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let para = para.trim_end_matches('\r');
        let info = BidiInfo::new(para, None);
        let rtl = info.paragraphs.first().is_some_and(|p| p.level.is_rtl());
        // Break into lines, in logical order.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut start = 0;
        let mut last_space: Option<usize> = None;
        let mut i = 0;
        while i < para.len() {
            let ch = para[i..].chars().next().expect("in range");
            let end = i + ch.len_utf8();
            if ch == ' ' {
                last_space = Some(i);
            }
            if max_width > 0.0
                && ch != ' '
                && measure(para[start..end].trim_end(), rtl, size) > max_width
                && end - start > ch.len_utf8()
            {
                match last_space.filter(|s| *s > start) {
                    Some(s) => {
                        ranges.push((start, s));
                        start = s + 1;
                    }
                    None => {
                        // A word longer than the line: break inside it.
                        ranges.push((start, i));
                        start = i;
                    }
                }
                last_space = None;
                continue;
            }
            i = end;
        }
        ranges.push((start, para.len()));
        for (s, e) in ranges {
            lines.push(set_line(&info, para, s, e, rtl, size));
        }
    }
    let width = lines.iter().map(|l: &Line| l.width).fold(0.0, f32::max);
    Layout { lines, size, leading, width }
}

/// Places the line `para[s..e]` in visual order.
fn set_line(info: &BidiInfo, para: &str, s: usize, e: usize, rtl: bool, size: f32) -> Line {
    let mut glyphs = Vec::new();
    let mut x = 0.0;
    if s < e {
        let p = &info.paragraphs[0];
        let (levels, runs) = info.visual_runs(p, s..e);
        for run in runs {
            let run_rtl = levels[run.start].is_rtl();
            let (gs, w) = shape(&para[run.clone()], run_rtl, size);
            glyphs.extend(gs.into_iter().map(|mut g| {
                g.x += x;
                g
            }));
            x += w;
        }
    }
    // Trailing spaces do not count towards the width (for alignment).
    let trimmed = para[s..e].trim_end_matches(' ');
    let width = if trimmed.len() == e - s { x } else { x - measure(&para[s + trimmed.len()..e], rtl, size) };
    Line { glyphs, width: width.max(0.0), rtl }
}

/// What a PDF needs to show laid-out text with the bundled font.
pub struct PdfFont {
    /// The subset font program (for /FontFile2).
    pub program: Vec<u8>,
    /// Glyph widths in thousandths of an em, by new glyph id.
    pub widths: Vec<(u16, f32)>,
    /// The ToUnicode CMap.
    pub to_unicode: Vec<u8>,
    /// Subset tag, e.g. "ABCDEF+IBMPlexSansArabic".
    pub base_font: String,
}

/// Content-stream operators that draw `layouts` (each with its top-left
/// corner, in user space) with font resource `res`, and the subset font
/// they need.
pub struct PdfText {
    pub ops: String,
    pub font: PdfFont,
}

/// Prepares text for a PDF: `blocks` are (layout, left, top, width, colour);
/// each line is aligned within `width` (right for right-to-left paragraphs).
pub fn pdf_text(blocks: &[(&Layout, f32, f32, f32, [f32; 3])], res: &str) -> PdfText {
    let f = font();
    let mut remap = subsetter::GlyphRemapper::new();
    let mut to_uni: BTreeMap<u16, String> = BTreeMap::new();
    let mut ops = String::new();
    for (layout, left, top, width, color) in blocks {
        ops += &format!("BT\n{} {} {} rg\n/{res} {} Tf\n", n(color[0]), n(color[1]), n(color[2]), n(layout.size));
        let mut baseline = top - layout.first_baseline();
        for line in &layout.lines {
            let x0 = if line.rtl { left + width - line.width } else { *left };
            for g in &line.glyphs {
                let new = remap.remap(g.id);
                if !g.text.is_empty() {
                    to_uni.entry(new).or_insert_with(|| g.text.clone());
                }
                ops += &format!("1 0 0 1 {} {} Tm <{new:04X}> Tj\n", n(x0 + g.x), n(baseline + g.y));
            }
            baseline -= layout.leading;
        }
        ops += "ET\n";
    }
    // Glyph 0 (.notdef) is always kept by the subsetter.
    let program = subsetter::subset(FONT_DATA, 0, &remap).unwrap_or_else(|_| FONT_DATA.to_vec());
    let widths = remap
        .remapped_gids()
        .enumerate()
        .map(|(new, old)| {
            let adv = f.face.glyph_hor_advance(ttf_parser::GlyphId(old)).unwrap_or(0) as f32;
            (new as u16, adv * 1000.0 / f.units_per_em)
        })
        .collect();
    PdfText {
        ops,
        font: PdfFont {
            program,
            widths,
            to_unicode: cmap(&to_uni),
            base_font: format!("{}+{FONT_NAME}", tag(&to_uni)),
        },
    }
}

/// A six-letter subset tag derived from the glyphs used.
fn tag(m: &BTreeMap<u16, String>) -> String {
    let mut h: u32 = 2_166_136_261;
    for (k, v) in m {
        for b in k.to_le_bytes().iter().chain(v.as_bytes()) {
            h = (h ^ *b as u32).wrapping_mul(16_777_619);
        }
    }
    (0..6).map(|i| (b'A' + ((h >> (i * 5)) % 26) as u8) as char).collect()
}

/// A ToUnicode CMap mapping each glyph to its text.
fn cmap(m: &BTreeMap<u16, String>) -> Vec<u8> {
    let mut s = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<_> = m.iter().collect();
    for chunk in entries.chunks(100) {
        s += &format!("{} beginbfchar\n", chunk.len());
        for (g, t) in chunk {
            let hex: String = t.encode_utf16().map(|u| format!("{u:04X}")).collect();
            s += &format!("<{g:04X}> <{hex}>\n");
        }
        s += "endbfchar\n";
    }
    s += "endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n";
    s.into_bytes()
}

fn n(v: f32) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arabic_is_shaped_and_reads_right_to_left() {
        let l = layout("مرحبا", 12.0, 0.0);
        assert_eq!(l.lines.len(), 1);
        let line = &l.lines[0];
        assert!(line.rtl);
        // Joined forms: the first letter (mim) is drawn last, at the right,
        // and its glyph differs from the isolated form.
        let isolated = shape("م", true, 12.0).0[0].id;
        let rightmost = line.glyphs.iter().max_by(|a, b| a.x.total_cmp(&b.x)).unwrap();
        assert_eq!(rightmost.text, "م");
        assert_ne!(rightmost.id, isolated, "initial form, not isolated");
    }

    #[test]
    fn mixed_text_keeps_latin_left_to_right() {
        let l = layout("نص PDF هنا", 12.0, 0.0);
        let line = &l.lines[0];
        assert!(line.rtl);
        // "PDF" stays in reading order P, D, F from left to right.
        let pos = |t: &str| line.glyphs.iter().find(|g| g.text == t).unwrap().x;
        assert!(pos("P") < pos("D") && pos("D") < pos("F"));
        // The first Arabic word is to the right of the Latin.
        assert!(pos("ص") > pos("F"), "first word (ending in sad) is right of the Latin");
        assert!(pos("ه") < pos("P"), "last word (starting with heh) is left of the Latin");
    }

    #[test]
    fn wraps_at_spaces_and_inside_long_words() {
        let l = layout("one two three four five", 10.0, 40.0);
        assert!(l.lines.len() > 2);
        assert!(
            l.lines.iter().all(|line| line.width <= 40.5),
            "{:?}",
            l.lines.iter().map(|x| x.width).collect::<Vec<_>>()
        );
        let long = layout("abcdefghijklmnopqrstuvwxyz", 10.0, 30.0);
        assert!(long.lines.len() > 3);
        let paras = layout("a\nb\n\nc", 10.0, 0.0);
        assert_eq!(paras.lines.len(), 4);
    }

    #[test]
    fn pdf_text_subsets_and_maps_back_to_unicode() {
        let l = layout("سلام hi", 14.0, 0.0);
        let t = pdf_text(&[(&l, 10.0, 100.0, 200.0, [0.0, 0.0, 0.0])], "F0");
        assert!(t.ops.contains("/F0 14 Tf") && t.ops.contains("Tj"));
        assert!(t.font.program.len() < FONT_DATA.len() / 4, "subset is small: {}", t.font.program.len());
        let cmap = String::from_utf8(t.font.to_unicode.clone()).unwrap();
        assert!(cmap.contains("<0068>"), "h maps back");
        assert!(cmap.contains("<0633>"), "seen maps back");
        assert!(t.font.base_font.ends_with("+IBMPlexSansArabic"));
        assert_eq!(t.font.widths[0].0, 0);
    }
}
