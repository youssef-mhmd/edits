//! Text for writing into PDFs: shaping with rustybuzz (Arabic letters join
//! and take their contextual forms), the Unicode bidi algorithm for mixed
//! right-to-left and left-to-right text, line breaking, and the pieces a PDF
//! needs to show the result (a subset font per font used, glyph widths, and
//! a ToUnicode map so that copy and search work in other readers).
//!
//! Text is kept in logical order everywhere; only the glyphs placed on a
//! line are in visual order. Arabic is never written without shaping.
//!
//! Fonts: font 0 is the bundled IBM Plex Sans Arabic (and 1 its bold).
//! Other fonts (the system's, see [`system`]) are loaded with [`load`]. A
//! character a font does not have is drawn in Plex.

pub mod system;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use rustybuzz::{Direction, Face, UnicodeBuffer};
use unicode_bidi::BidiInfo;

/// The bundled font: IBM Plex Sans Arabic (SIL Open Font License), which
/// covers Arabic and Latin.
pub static FONT_DATA: &[u8] = include_bytes!("../fonts/IBMPlexSansArabic-Regular.ttf");
static BOLD_DATA: &[u8] = include_bytes!("../fonts/IBMPlexSansArabic-Bold.ttf");
/// The name the font is embedded under.
pub const FONT_NAME: &str = "IBMPlexSansArabic";

/// A font in this process's registry.
pub type FontId = u16;
/// IBM Plex Sans Arabic, the default and the fallback.
pub const DEFAULT_FONT: FontId = 0;
/// IBM Plex Sans Arabic Bold.
pub const DEFAULT_BOLD: FontId = 1;

pub struct Font {
    face: Face<'static>,
    data: &'static [u8],
    /// Index in a font collection (.ttc), 0 otherwise.
    index: u32,
    /// The PostScript name, as embedded (letters and digits only).
    pub name: String,
    pub units_per_em: f32,
    /// Ascent and descent in font units (descent is negative).
    pub ascent: f32,
    pub descent: f32,
    pub cap_height: f32,
    /// Font bounding box in font units.
    pub bbox: [f32; 4],
}

impl Font {
    fn parse(data: &'static [u8], index: u32, fallback_name: &str) -> Result<Font, String> {
        let face = Face::from_slice(data, index).ok_or("not a font edits can read")?;
        if face.tables().glyf.is_none() {
            return Err("only TrueType fonts can be embedded".into());
        }
        let name = face
            .names()
            .into_iter()
            .filter(|n| n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
            .find_map(|n| n.to_string())
            .unwrap_or_else(|| fallback_name.to_string())
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect::<String>();
        let b = face.global_bounding_box();
        Ok(Font {
            units_per_em: face.units_per_em() as f32,
            ascent: face.ascender() as f32,
            descent: face.descender() as f32,
            cap_height: face.capital_height().unwrap_or(face.ascender()) as f32,
            bbox: [b.x_min as f32, b.y_min as f32, b.x_max as f32, b.y_max as f32],
            name: if name.is_empty() { "Font".into() } else { name },
            face,
            data,
            index,
        })
    }

    fn has(&self, c: char) -> bool {
        self.face.glyph_index(c).is_some_and(|g| g.0 != 0)
    }
}

struct Registry {
    fonts: Vec<&'static Font>,
    by_path: HashMap<(PathBuf, u32), FontId>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| {
        // Embedded under fixed names (the regular one as earlier versions did).
        let plex = |data: &'static [u8], name: &str| -> &'static Font {
            let mut f = Font::parse(data, 0, name).expect("bundled font parses");
            f.name = name.to_string();
            Box::leak(Box::new(f))
        };
        Mutex::new(Registry {
            fonts: vec![plex(FONT_DATA, FONT_NAME), plex(BOLD_DATA, "IBMPlexSansArabicBold")],
            by_path: HashMap::new(),
        })
    })
}

/// A font by id (the bundled font for an id never handed out).
pub fn get(id: FontId) -> &'static Font {
    let r = registry().lock().unwrap_or_else(|e| e.into_inner());
    r.fonts.get(id as usize).copied().unwrap_or(r.fonts[0])
}

/// The bundled font.
pub fn font() -> &'static Font {
    get(DEFAULT_FONT)
}

/// Loads font `index` of the file at `path` (once per process) and returns
/// its id. Fails for files that are not TrueType fonts and for fonts whose
/// licence does not allow embedding.
pub fn load(path: &Path, index: u32) -> Result<FontId, String> {
    let key = (path.to_path_buf(), index);
    if let Some(id) = registry().lock().unwrap_or_else(|e| e.into_inner()).by_path.get(&key) {
        return Ok(*id);
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let face = ttf_parser::Face::parse(&bytes, index).map_err(|e| format!("{}: {e}", path.display()))?;
    if !system::embeddable(&face) {
        return Err(format!("{} may not be embedded in documents", path.display()));
    }
    let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let f: &'static Font = Box::leak(Box::new(Font::parse(data, index, &stem)?));
    let mut r = registry().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(id) = r.by_path.get(&key) {
        return Ok(*id);
    }
    let id = r.fonts.len() as FontId;
    r.fonts.push(f);
    r.by_path.insert(key, id);
    Ok(id)
}

/// A glyph placed on a line.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    /// Glyph id in its font.
    pub id: u16,
    pub font: FontId,
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

/// How lines sit in the width they are given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Align {
    /// At the start of the paragraph: left for left-to-right, right for right-to-left.
    #[default]
    Start,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub lines: Vec<Line>,
    pub size: f32,
    /// Distance between baselines.
    pub leading: f32,
    /// Widest line.
    pub width: f32,
    /// The font the text was set in (line height comes from it).
    pub font: FontId,
    pub align: Align,
}

impl Layout {
    /// Height of all lines, from the first line's top to the last's bottom.
    pub fn height(&self) -> f32 {
        let f = get(self.font);
        let asc = f.ascent / f.units_per_em * self.size;
        let desc = -f.descent / f.units_per_em * self.size;
        if self.lines.is_empty() { 0.0 } else { asc + desc + self.leading * (self.lines.len() - 1) as f32 }
    }

    /// The box the glyphs' outlines cover on a one-line layout: left, bottom,
    /// right, top, in points from the line's start and baseline. None if no
    /// glyph has ink.
    pub fn ink(&self) -> Option<[f32; 4]> {
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for g in self.lines.first()?.glyphs.iter() {
            let f = get(g.font);
            let k = self.size / f.units_per_em;
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
        let f = get(self.font);
        f.ascent / f.units_per_em * self.size
    }

    /// Where `line` starts in a box `width` wide.
    pub fn line_x(&self, line: &Line, width: f32) -> f32 {
        let right = match self.align {
            Align::Start => line.rtl,
            Align::Right => true,
            Align::Left | Align::Center => false,
        };
        if self.align == Align::Center {
            (width - line.width) / 2.0
        } else if right {
            width - line.width
        } else {
            0.0
        }
    }
}

/// Characters drawn with the letter before them (combining marks, joiners).
fn follows(c: char) -> bool {
    matches!(c as u32,
        0x0300..=0x036F | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4
        | 0x06E7 | 0x06E8 | 0x06EA..=0x06ED | 0x08D3..=0x08FF | 0x200C | 0x200D | 0xFE00..=0xFE0F)
}

/// `text` cut where the font that draws it changes: `font` where it has
/// the character, Plex where only Plex has it. Byte ranges in logical order.
fn font_runs(text: &str, font: FontId) -> Vec<(usize, usize, FontId)> {
    let primary = get(font);
    let plex = get(DEFAULT_FONT);
    let mut out: Vec<(usize, usize, FontId)> = Vec::new();
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        let want = match out.last() {
            Some(&(_, _, f)) if follows(c) || c == ' ' => f,
            _ if font != DEFAULT_FONT && !primary.has(c) && plex.has(c) => DEFAULT_FONT,
            _ => font,
        };
        match out.last_mut() {
            Some(last) if last.2 == want => last.1 = end,
            _ => out.push((i, end, want)),
        }
    }
    out
}

/// Shapes `text` (one direction) in `font` and returns glyphs with x
/// offsets from 0, in visual order, and the total advance.
fn shape(text: &str, rtl: bool, size: f32, font: FontId) -> (Vec<Glyph>, f32) {
    let mut runs = font_runs(text, font);
    if rtl {
        runs.reverse();
    }
    let mut glyphs = Vec::new();
    let mut x = 0.0;
    for (s, e, f) in runs {
        let (gs, w) = shape_one(&text[s..e], rtl, size, f);
        glyphs.extend(gs.into_iter().map(|mut g| {
            g.x += x;
            g
        }));
        x += w;
    }
    (glyphs, x)
}

fn shape_one(text: &str, rtl: bool, size: f32, font: FontId) -> (Vec<Glyph>, f32) {
    let f = get(font);
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
            font,
            x: x + p.x_offset as f32 * scale,
            y: p.y_offset as f32 * scale,
            text: t,
        });
        x += p.x_advance as f32 * scale;
    }
    (glyphs, x)
}

/// Width of `text` set in one direction.
fn measure(text: &str, rtl: bool, size: f32, font: FontId) -> f32 {
    shape(text, rtl, size, font).1
}

/// Lays out `text` in the bundled font at `size` points in lines no wider
/// than `max_width` (or unbounded if it is 0). Paragraphs break at '\n';
/// lines break at spaces, or inside a word longer than the line.
pub fn layout(text: &str, size: f32, max_width: f32) -> Layout {
    layout_with(text, &Style { size, ..Style::default() }, max_width)
}

/// How text is set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub font: FontId,
    pub size: f32,
    /// Distance between baselines as a multiple of the size.
    pub line_height: f32,
    pub align: Align,
    /// The paragraph direction: None from the text itself, Some(true) right to left.
    pub rtl: Option<bool>,
}

impl Default for Style {
    fn default() -> Self {
        Style { font: DEFAULT_FONT, size: 12.0, line_height: 1.3, align: Align::Start, rtl: None }
    }
}

/// Like [`layout`], with a font, line height, alignment and direction.
pub fn layout_with(text: &str, style: &Style, max_width: f32) -> Layout {
    let size = style.size.clamp(1.0, 500.0);
    let font = style.font;
    let leading = size * style.line_height.clamp(0.5, 5.0);
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let para = para.trim_end_matches('\r');
        let level = style.rtl.map(|r| if r { unicode_bidi::Level::rtl() } else { unicode_bidi::Level::ltr() });
        let info = BidiInfo::new(para, level);
        let rtl = match style.rtl {
            Some(r) => r,
            None => info.paragraphs.first().is_some_and(|p| p.level.is_rtl()),
        };
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
                && measure(para[start..end].trim_end(), rtl, size, font) > max_width
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
            lines.push(set_line(&info, para, s, e, rtl, size, font));
        }
    }
    let width = lines.iter().map(|l: &Line| l.width).fold(0.0, f32::max);
    Layout { lines, size, leading, width, font, align: style.align }
}

/// Places the line `para[s..e]` in visual order.
fn set_line(info: &BidiInfo, para: &str, s: usize, e: usize, rtl: bool, size: f32, font: FontId) -> Line {
    let mut glyphs = Vec::new();
    let mut x = 0.0;
    if s < e && !info.paragraphs.is_empty() {
        let p = &info.paragraphs[0];
        let (levels, runs) = info.visual_runs(p, s..e);
        for run in runs {
            let run_rtl = levels[run.start].is_rtl();
            let (gs, w) = shape(&para[run.clone()], run_rtl, size, font);
            glyphs.extend(gs.into_iter().map(|mut g| {
                g.x += x;
                g
            }));
            x += w;
        }
    }
    // Trailing spaces do not count towards the width (for alignment).
    let trimmed = para[s..e].trim_end_matches(' ');
    let width = if trimmed.len() == e - s { x } else { x - measure(&para[s + trimmed.len()..e], rtl, size, font) };
    Line { glyphs, width: width.max(0.0), rtl }
}

/// What a PDF needs to show laid-out text in one font.
pub struct PdfFont {
    /// The subset font program (for /FontFile2).
    pub program: Vec<u8>,
    /// Glyph widths in thousandths of an em, by new glyph id.
    pub widths: Vec<(u16, f32)>,
    /// The ToUnicode CMap.
    pub to_unicode: Vec<u8>,
    /// Subset tag and name, e.g. "ABCDEF+IBMPlexSansArabic".
    pub base_font: String,
    /// Ascent, descent and cap height in thousandths of an em.
    pub ascent: f32,
    pub descent: f32,
    pub cap_height: f32,
    /// Bounding box in thousandths of an em.
    pub bbox: [f32; 4],
}

/// Content-stream operators that draw `layouts` (each with its top-left
/// corner, in user space) with font resource `res`, and the subset font
/// they need.
pub struct PdfText {
    pub ops: String,
    pub font: PdfFont,
}

/// Prepares text in the bundled font for a PDF: `blocks` are (layout, left,
/// top, width, colour); each line is aligned within `width`.
pub fn pdf_text(blocks: &[(&Layout, f32, f32, f32, [f32; 3])], res: &str) -> PdfText {
    let (mut ops, font) = pdf_text_parts(&[blocks], res);
    PdfText { ops: ops.pop().unwrap_or_default(), font }
}

/// Text blocks: (layout, left, top, width, colour).
pub type Blocks<'a> = [(&'a Layout, f32, f32, f32, [f32; 3])];

/// Like [`pdf_text`] for several content streams (pages) that share one
/// font: the operators of each part, and the font they all use. For text in
/// the bundled font only; see [`pdf_text_fonts`].
pub fn pdf_text_parts(parts: &[&Blocks], res: &str) -> (Vec<String>, PdfFont) {
    let (ops, mut fonts) = pdf_text_fonts(parts, res);
    let font = match fonts.iter().position(|(n, _)| n == res) {
        Some(i) => fonts.swap_remove(i).1,
        None => subset_font(DEFAULT_FONT, &subsetter::GlyphRemapper::new(), &BTreeMap::new()),
    };
    (ops, font)
}

/// The resource name of font `id` in text written with resource prefix `res`.
pub fn font_res(res: &str, id: FontId) -> String {
    if id == DEFAULT_FONT { res.to_string() } else { format!("{res}_{id}") }
}

/// Text in any fonts for several content streams: the operators of each
/// part, and each font used with its resource name ([`font_res`]).
pub fn pdf_text_fonts(parts: &[&Blocks], res: &str) -> (Vec<String>, Vec<(String, PdfFont)>) {
    let mut used: BTreeMap<FontId, (subsetter::GlyphRemapper, BTreeMap<u16, String>)> = BTreeMap::new();
    let mut all = Vec::with_capacity(parts.len());
    for blocks in parts {
        let mut ops = String::new();
        for (layout, left, top, width, color) in blocks.iter() {
            ops += &format!("BT\n{} {} {} rg\n", n(color[0]), n(color[1]), n(color[2]));
            let mut current: Option<FontId> = None;
            let mut baseline = top - layout.first_baseline();
            for line in &layout.lines {
                let x0 = left + layout.line_x(line, *width);
                for g in &line.glyphs {
                    if current != Some(g.font) {
                        ops += &format!("/{} {} Tf\n", font_res(res, g.font), n(layout.size));
                        current = Some(g.font);
                    }
                    let (remap, to_uni) = used.entry(g.font).or_default();
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
        all.push(ops);
    }
    let fonts =
        used.iter().map(|(id, (remap, to_uni))| (font_res(res, *id), subset_font(*id, remap, to_uni))).collect();
    (all, fonts)
}

fn subset_font(id: FontId, remap: &subsetter::GlyphRemapper, to_uni: &BTreeMap<u16, String>) -> PdfFont {
    let f = get(id);
    // Glyph 0 (.notdef) is always kept by the subsetter.
    let program = subsetter::subset(f.data, f.index, remap).unwrap_or_else(|_| f.data.to_vec());
    let k = 1000.0 / f.units_per_em;
    let widths = remap
        .remapped_gids()
        .enumerate()
        .map(|(new, old)| {
            let adv = f.face.glyph_hor_advance(ttf_parser::GlyphId(old)).unwrap_or(0) as f32;
            (new as u16, adv * k)
        })
        .collect();
    PdfFont {
        program,
        widths,
        to_unicode: cmap(to_uni),
        base_font: format!("{}+{}", tag(to_uni), f.name),
        ascent: f.ascent * k,
        descent: f.descent * k,
        cap_height: f.cap_height * k,
        bbox: f.bbox.map(|v| v * k),
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
        let isolated = shape("م", true, 12.0, DEFAULT_FONT).0[0].id;
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

    #[test]
    fn bold_is_its_own_font_and_alignment_and_direction_hold() {
        let style = Style { font: DEFAULT_BOLD, size: 12.0, align: Align::Center, ..Style::default() };
        let l = layout_with("Hello", &style, 0.0);
        assert!(l.lines[0].glyphs.iter().all(|g| g.font == DEFAULT_BOLD));
        let (ops, fonts) = pdf_text_fonts(&[&[(&l, 0.0, 100.0, 100.0, [0.0; 3])]], "F");
        assert_eq!(fonts.len(), 1);
        assert_eq!(fonts[0].0, "F_1");
        assert!(fonts[0].1.base_font.ends_with("+IBMPlexSansArabicBold"), "{}", fonts[0].1.base_font);
        // Centred: the first glyph starts at half the spare width.
        let x0 = (100.0 - l.lines[0].width) / 2.0;
        assert!(ops[0].contains(&format!("1 0 0 1 {} ", n(x0))), "{}", ops[0]);
        // A Latin word set right to left on purpose keeps its letters in order
        // but the line is right-aligned.
        let rtl = layout_with("abc", &Style { rtl: Some(true), ..Style::default() }, 0.0);
        assert!(rtl.lines[0].rtl);
    }

    #[test]
    fn characters_a_font_lacks_come_from_plex() {
        // Arial has Arabic; a Latin-only system font shows the fallback. Use
        // whatever Latin-only TrueType font this machine has.
        let Some(face) = system::fonts().into_iter().find(|f| !f.arabic) else { return };
        let Ok(id) = load(&face.path, face.index) else { return };
        let l = layout_with("ab سلام", &Style { font: id, ..Style::default() }, 0.0);
        let fonts: std::collections::BTreeSet<FontId> = l.lines[0].glyphs.iter().map(|g| g.font).collect();
        assert!(fonts.contains(&id) && fonts.contains(&DEFAULT_FONT), "{fonts:?}");
        let (_, pdf) = pdf_text_fonts(&[&[(&l, 0.0, 100.0, 100.0, [0.0; 3])]], "F");
        assert_eq!(pdf.len(), 2);
    }
}
