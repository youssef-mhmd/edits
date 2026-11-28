//! Rebuilds logical (reading) order from positioned glyphs.
//!
//! PDFium's character order is the content stream's order. For Arabic that is
//! often visual order, word by word or run by run, and diacritics drawn as
//! separate objects end up on lines of their own. Copy and search need the
//! order a person would type. This module:
//!
//! 1. groups glyphs into clusters: characters that share one box (a ligature
//!    such as lam-alef carries two letters), and combining marks with the
//!    letter they sit on;
//! 2. groups clusters into lines by baseline, following PDFium's order from
//!    line to line so columns keep their reading order;
//! 3. sorts each line left to right (visual order), keeps real spaces where
//!    they are drawn and adds spaces at wide gaps, and applies the Unicode
//!    bidirectional algorithm to that visual sequence to get logical order
//!    back, as pdf.js does. Latin words and numbers inside Arabic lines stay
//!    left to right.
//!
//! Glyph boxes are not line-height: in Arabic fonts a meem can be a quarter
//! of the height of a tah. Baselines are what lines share.

use unicode_bidi::{BidiInfo, Level};

/// A glyph from PDFium, in points from the page's top-left. `rect` is
/// [left, top, right, bottom]; `None` for characters PDFium generated.
#[derive(Debug, Clone, Copy)]
pub struct Glyph {
    pub c: char,
    pub rect: Option<[f32; 4]>,
    /// Baseline: the y of the glyph's origin.
    pub baseline: f32,
    /// Font size in points (at least 1).
    pub size: f32,
}

/// Rebuilt text and, for each of its characters, the glyph it came from
/// (`None` for spaces and line breaks this module inserted).
pub struct Rebuilt {
    pub text: Vec<char>,
    pub source: Vec<Option<usize>>,
}

fn is_mark(c: char) -> bool {
    // Combining marks used by Arabic, Persian, Urdu and Latin.
    matches!(c as u32,
        0x0300..=0x036F | 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4
        | 0x06E7 | 0x06E8 | 0x06EA..=0x06ED | 0x08D3..=0x08FF | 0xFE20..=0xFE2F)
}

fn is_rtl(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF)
}

struct Cluster {
    /// Glyph indices, in reading order within the cluster (base first, then marks).
    glyphs: Vec<usize>,
    rect: [f32; 4],
    baseline: f32,
    size: f32,
    space: bool,
}

impl Cluster {
    fn cx(&self) -> f32 {
        (self.rect[0] + self.rect[2]) / 2.0
    }
    fn width(&self) -> f32 {
        self.rect[2] - self.rect[0]
    }
}

fn x_overlap(a: [f32; 4], b: [f32; 4]) -> f32 {
    (a[2].min(b[2]) - a[0].max(b[0])).max(0.0)
}

fn same_line(a_base: f32, a_size: f32, b_base: f32, b_size: f32) -> bool {
    (a_base - b_base).abs() <= 0.35 * a_size.max(b_size)
}

/// Step 1: clusters in PDFium order.
fn clusters(glyphs: &[Glyph]) -> Vec<Cluster> {
    let mut out: Vec<Cluster> = Vec::new();
    let mut marks: Vec<usize> = Vec::new();
    for (i, g) in glyphs.iter().enumerate() {
        let Some(r) = g.rect else { continue }; // generated: spaces come back from gaps
        if is_mark(g.c) {
            marks.push(i);
            continue;
        }
        // Spaces come back from gaps: drawn space positions are unreliable in
        // right-to-left runs, and Latin text often has none drawn at all.
        if g.c.is_whitespace() {
            continue;
        }
        let space = false;
        // A ligature: consecutive letters drawn by one glyph share (nearly) its box.
        if let Some(last) = out.last_mut()
            && !space
            && !last.space
            && last.glyphs.last() == Some(&(i - 1))
            && same_line(last.baseline, last.size, g.baseline, g.size)
            && x_overlap(last.rect, r) >= 0.8 * last.width().min(r[2] - r[0]).max(0.01)
        {
            last.glyphs.push(i);
            continue;
        }
        out.push(Cluster { glyphs: vec![i], rect: r, baseline: g.baseline, size: g.size, space });
    }
    // Each mark joins a letter on (about) its baseline: the letter its centre
    // falls in, or else the nearest one. Between equally near letters, the
    // one to its right wins: some producers (Chrome) draw a mark's box just
    // left of its letter. Marks are often drawn with their own vertical
    // offset, so their baseline is only near their letter's.
    for i in marks {
        let g = glyphs[i];
        let r = g.rect.expect("marks have boxes");
        let mid = (r[0] + r[2]) / 2.0;
        let best = out
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.space && (c.baseline - g.baseline).abs() <= c.size.max(g.size))
            .map(|(k, c)| {
                let d = (c.rect[0] - mid).max(mid - c.rect[2]).max(0.0);
                // Lines close together: a letter of the next line can be as
                // near; one on the mark's own line comes first.
                let other_line = !same_line(c.baseline, c.size, g.baseline, g.size);
                (k, d, other_line, (c.rect[0] - r[2]).abs())
            })
            .filter(|&(k, d, _, _)| d <= out[k].size)
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.2.cmp(&b.2)).then(a.3.total_cmp(&b.3)))
            .map(|(k, _, _, _)| k);
        match best {
            Some(k) => out[k].glyphs.push(i),
            None => out.push(Cluster { glyphs: vec![i], rect: r, baseline: g.baseline, size: g.size, space: false }),
        }
    }
    out
}

/// Step 2: lines, in PDFium order. A cluster joins the newest line on its
/// baseline among the last few, or starts a new line.
fn lines(clusters: Vec<Cluster>) -> Vec<Vec<Cluster>> {
    struct Line {
        clusters: Vec<Cluster>,
        baseline: f32,
        size: f32,
        left: f32,
        right: f32,
    }
    let mut out: Vec<Line> = Vec::new();
    for c in clusters {
        // The newest line on this baseline, or an earlier one (within a few
        // lines) that this cluster sits next to: PDFium sometimes breaks a
        // line into pieces. Lines of another column are too far away to join.
        let n = out.len();
        let found = (n.saturating_sub(8)..n).rev().find(|&k| {
            let l = &out[k];
            same_line(l.baseline, l.size, c.baseline, c.size)
                && (k + 1 == n || (c.rect[0] - l.right).max(l.left - c.rect[2]) < 3.0 * l.size.max(c.size))
        });
        match found {
            Some(k) => {
                let l = &mut out[k];
                l.size = l.size.max(c.size);
                l.left = l.left.min(c.rect[0]);
                l.right = l.right.max(c.rect[2]);
                l.clusters.push(c);
            }
            None => out.push(Line {
                baseline: c.baseline,
                size: c.size,
                left: c.rect[0],
                right: c.rect[2],
                clusters: vec![c],
            }),
        }
    }
    out.into_iter().map(|l| l.clusters).collect()
}

/// Step 3: one line from visual to logical order.
/// Whether a space separates two visually adjacent clusters. If PDFium has
/// them close together in its order, its order decides: a space (drawn or
/// generated) between them means a word break, nothing but marks means the
/// same word, however the boxes fall. Otherwise a gap wider than letters
/// leave between them does.
fn space_between(glyphs: &[Glyph], a: &Cluster, b: &Cluster) -> bool {
    let (ia, ib) = (a.glyphs[0], b.glyphs[0]);
    let (lo, hi) = (ia.min(ib), ia.max(ib));
    let gap = b.rect[0] - a.rect[2];
    let size = a.size.max(b.size);
    if hi - lo <= 8 && gap < 0.5 * size {
        // Line breaks PDFium generates fall inside words too; they say nothing.
        let between: Vec<char> = glyphs[lo + 1..hi].iter().map(|g| g.c).filter(|c| !matches!(c, '\r' | '\n')).collect();
        if between.iter().any(|c| c.is_whitespace()) {
            // A drawn space between letters that touch is an artefact (Chrome
            // puts one before each separately drawn cluster); word spaces leave a gap.
            return gap > 0.1 * size;
        }
        if between.iter().all(|c| is_mark(*c)) {
            return false;
        }
    }
    gap > 0.15 * size
}

fn logical_line(glyphs: &[Glyph], mut line: Vec<Cluster>, text: &mut Vec<char>, source: &mut Vec<Option<usize>>) {
    line.sort_by(|a, b| a.cx().total_cmp(&b.cx()));
    // Visual sequence of units: a cluster (by index), or a space at a gap
    // wider than letters leave between them.
    let mut units: Vec<Option<usize>> = Vec::new();
    for k in 0..line.len() {
        if k > 0 && space_between(glyphs, &line[k - 1], &line[k]) {
            units.push(None);
        }
        units.push(Some(k));
    }
    let unit_text: Vec<String> = units
        .iter()
        .map(|u| match u {
            Some(k) => line[*k].glyphs.iter().map(|&g| glyphs[g].c).collect(),
            None => " ".to_string(),
        })
        .collect();
    let joined: String = unit_text.concat();
    // Nothing right to left: visual order is reading order.
    if !joined.chars().any(is_rtl) {
        for u in &units {
            push_unit(glyphs, &line, *u, text, source);
        }
        return;
    }
    // The paragraph direction is the majority of strong characters, so an
    // Arabic line with English words in it is right to left.
    let rtl = joined.chars().filter(|c| is_rtl(*c) && c.is_alphabetic()).count();
    let ltr = joined.chars().filter(|c| c.is_alphabetic() && !is_rtl(*c)).count();
    let para = if rtl > ltr { Level::rtl() } else { Level::ltr() };
    let info = BidiInfo::new(&joined, Some(para));
    let mut levels = Vec::with_capacity(units.len());
    let mut byte = 0;
    for t in &unit_text {
        levels.push(info.levels.get(byte).map(Level::number).unwrap_or(para.number()));
        byte += t.len();
    }
    // Reverse runs from the highest level down to the lowest odd level (rule L2).
    let mut order: Vec<usize> = (0..units.len()).collect();
    let max = levels.iter().copied().max().unwrap_or(0);
    let min_odd = levels.iter().copied().filter(|l| l % 2 == 1).min().unwrap_or(max + 1);
    let mut level = max;
    while level >= min_odd && level > 0 {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] >= level {
                let start = i;
                while i < order.len() && levels[order[i]] >= level {
                    i += 1;
                }
                order[start..i].reverse();
            } else {
                i += 1;
            }
        }
        level -= 1;
    }
    for u in order {
        push_unit(glyphs, &line, units[u], text, source);
    }
}

fn push_unit(
    glyphs: &[Glyph],
    line: &[Cluster],
    unit: Option<usize>,
    text: &mut Vec<char>,
    source: &mut Vec<Option<usize>>,
) {
    match unit {
        Some(k) => {
            for &g in &line[k].glyphs {
                text.push(if line[k].space { ' ' } else { glyphs[g].c });
                source.push(Some(g));
            }
        }
        None => {
            text.push(' ');
            source.push(None);
        }
    }
}

/// Rebuilds the page's text in reading order.
pub fn rebuild(glyphs: &[Glyph]) -> Rebuilt {
    let mut text = Vec::new();
    let mut source = Vec::new();
    for (n, line) in lines(clusters(glyphs)).into_iter().enumerate() {
        if n > 0 {
            text.push('\n');
            source.push(None);
        }
        logical_line(glyphs, line, &mut text, &mut source);
    }
    Rebuilt { text, source }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Glyphs laid out left to right on one line, 10 pt apart, in the given
    /// (PDFium) order; spaces are drawn as zero-width glyphs.
    fn line_of(visual: &str, y: f32, order: impl Fn(usize) -> usize) -> Vec<Glyph> {
        let mut out: Vec<(usize, Glyph)> = visual
            .chars()
            .enumerate()
            .map(|(i, c)| {
                let x = i as f32 * 10.0;
                let rect = if c == ' ' { [x + 4.0, y + 12.0, x + 4.0, y + 12.0] } else { [x, y, x + 9.0, y + 12.0] };
                (order(i), Glyph { c, rect: Some(rect), baseline: y + 12.0, size: 12.0 })
            })
            .collect();
        out.sort_by_key(|(k, _)| *k);
        out.into_iter().map(|(_, g)| g).collect()
    }

    fn text(glyphs: &[Glyph]) -> String {
        rebuild(glyphs).text.into_iter().collect()
    }

    fn g(c: char, r: [f32; 4]) -> Glyph {
        Glyph { c, rect: Some(r), baseline: 12.0, size: 12.0 }
    }

    #[test]
    fn arabic_in_visual_order_comes_back_logical() {
        let visual: String = "سلام عليكم".chars().rev().collect();
        // PDFium hands the glyphs over in visual order...
        assert_eq!(text(&line_of(&visual, 0.0, |i| i)), "سلام عليكم");
        // ...or in logical order: geometry decides, so the result is the same.
        let n = visual.chars().count();
        assert_eq!(text(&line_of(&visual, 0.0, |i| n - i)), "سلام عليكم");
    }

    #[test]
    fn latin_and_numbers_inside_arabic_stay_left_to_right() {
        assert_eq!(text(&line_of("26 ددع PDF تافلم", 0.0, |i| i)), "ملفات PDF عدد 26");
    }

    #[test]
    fn latin_lines_unchanged_and_lines_kept_apart() {
        let mut gl = line_of("Hello world", 0.0, |i| i);
        gl.extend(line_of("second line", 20.0, |i| i + 100));
        assert_eq!(text(&gl), "Hello world\nsecond line");
    }

    #[test]
    fn short_and_tall_letters_share_a_line() {
        // A meem's box is a quarter the height of a tah's; they share a baseline.
        let gl = vec![g('ط', [10.0, 0.0, 19.0, 12.0]), g('م', [0.0, 8.0, 9.0, 12.0])];
        assert_eq!(text(&gl), "طم");
    }

    #[test]
    fn marks_follow_their_letter_even_when_drawn_elsewhere() {
        // "مَن": meem with fatha, then noon. Visual order: noon, meem.
        let mut gl = vec![g('ن', [0.0, 0.0, 9.0, 12.0]), g('م', [10.0, 0.0, 19.0, 12.0])];
        // The fatha sits above the meem and arrives last, as its own object.
        gl.push(g('\u{064E}', [11.0, -5.0, 17.0, -1.0]));
        assert_eq!(text(&gl), "مَن");
    }

    #[test]
    fn a_mark_never_joins_the_line_above() {
        let mut gl = vec![g('ب', [0.0, 0.0, 9.0, 12.0])];
        // Next line: a letter with a fatha that, by x, also overlaps the letter above.
        gl.push(Glyph { c: 'ر', rect: Some([0.0, 20.0, 9.0, 32.0]), baseline: 32.0, size: 12.0 });
        gl.push(Glyph { c: '\u{064E}', rect: Some([2.0, 16.0, 7.0, 19.0]), baseline: 32.0, size: 12.0 });
        assert_eq!(text(&gl), "ب\nرَ");
    }

    #[test]
    fn a_ligature_keeps_its_letters_in_order() {
        // Lam-alef: one glyph box carrying two letters (boxes nearly equal), then a meem to its left.
        let gl = vec![g('م', [0.0, 0.0, 9.0, 12.0]), g('ل', [10.0, 0.0, 19.0, 12.0]), g('ا', [10.2, 0.0, 19.0, 12.0])];
        assert_eq!(text(&gl), "لام");
    }

    #[test]
    fn wide_gaps_without_drawn_spaces_become_spaces() {
        let gl = vec![g('a', [0.0, 0.0, 9.0, 12.0]), g('b', [40.0, 0.0, 49.0, 12.0])];
        assert_eq!(text(&gl), "a b");
    }
}
