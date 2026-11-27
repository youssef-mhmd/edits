//! Page numbers, headers, footers and watermarks (PRD 5.3). They are drawn
//! into the page content (not as annotations), after the page's own
//! content, upright as the page is shown whatever its rotation. Arabic text
//! is shaped like any text edits writes.

use serde::{Deserialize, Serialize};

use crate::pdf::object::real;
use crate::pdf::{Dict, Obj};

/// What to add to pages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decor {
    /// Text at the top: left, centre, right. `{n}` is the page number and
    /// `{total}` the page count.
    #[serde(default)]
    pub header: [String; 3],
    /// Text at the bottom: left, centre, right.
    #[serde(default)]
    pub footer: [String; 3],
    #[serde(default = "ten")]
    pub size: f32,
    #[serde(default)]
    pub color: [f32; 3],
    /// Distance from the page edge, in points.
    #[serde(default = "margin")]
    pub margin: f32,
    /// Added to a page's position (1 for the first page) to give `{n}`.
    #[serde(default)]
    pub offset: i32,
    /// Numbers in Arabic-Indic digits.
    #[serde(default)]
    pub arabic_digits: bool,
    /// Text across the middle of the page, at an angle.
    #[serde(default)]
    pub watermark: String,
    #[serde(default = "wm_size")]
    pub watermark_size: f32,
    #[serde(default = "wm_color")]
    pub watermark_color: [f32; 3],
    #[serde(default = "wm_opacity")]
    pub watermark_opacity: f32,
}

fn ten() -> f32 {
    10.0
}
fn margin() -> f32 {
    28.0
}
fn wm_size() -> f32 {
    60.0
}
fn wm_color() -> [f32; 3] {
    [0.8, 0.1, 0.1]
}
fn wm_opacity() -> f32 {
    0.25
}

impl Decor {
    /// Limits to sensible values.
    pub fn clamped(mut self) -> Self {
        self.size = self.size.clamp(4.0, 72.0);
        self.margin = self.margin.clamp(0.0, 300.0);
        self.watermark_size = self.watermark_size.clamp(8.0, 400.0);
        self.watermark_opacity = self.watermark_opacity.clamp(0.02, 1.0);
        self.color = self.color.map(|c| c.clamp(0.0, 1.0));
        self.watermark_color = self.watermark_color.map(|c| c.clamp(0.0, 1.0));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.header.iter().chain(&self.footer).all(|s| s.trim().is_empty()) && self.watermark.trim().is_empty()
    }

    fn fill(&self, text: &str, n: i64, total: usize) -> String {
        let digits = |v: String| {
            if self.arabic_digits {
                v.chars().map(|c| c.to_digit(10).map_or(c, |d| char::from_u32(0x0660 + d).unwrap_or(c))).collect()
            } else {
                v
            }
        };
        text.replace("{n}", &digits(n.to_string())).replace("{total}", &digits(total.to_string())).replace('\n', " ")
    }
}

/// Where a page's content is drawn: its visible box and turn.
#[derive(Debug, Clone, Copy)]
pub(super) struct Frame {
    /// Visible box in user space: left, bottom, right, top.
    pub bx: [f32; 4],
    /// Clockwise rotation in degrees (0, 90, 180, 270).
    pub rotate: i64,
}

impl Frame {
    /// Width and height as shown.
    fn shown(&self) -> (f32, f32) {
        let (w, h) = (self.bx[2] - self.bx[0], self.bx[3] - self.bx[1]);
        if self.rotate % 180 == 0 { (w, h) } else { (h, w) }
    }

    /// The matrix from shown coordinates (origin bottom-left of the page as
    /// shown) to user space.
    fn matrix(&self) -> [f32; 6] {
        let [x0, y0, x1, y1] = self.bx;
        match self.rotate {
            90 => [0.0, 1.0, -1.0, 0.0, x1, y0],
            180 => [-1.0, 0.0, 0.0, -1.0, x1, y1],
            270 => [0.0, -1.0, 1.0, 0.0, x0, y1],
            _ => [1.0, 0.0, 0.0, 1.0, x0, y0],
        }
    }
}

fn cm(m: [f32; 6]) -> String {
    m.iter().map(|v| real(*v as f64)).collect::<Vec<_>>().join(" ") + " cm\n"
}

/// One page's text, laid out: header and footer blocks, and each watermark
/// (blocks around the origin, its angle, its graphics state).
pub(super) struct PageText {
    pub frame: Frame,
    pub plain: Vec<Block>,
    /// Watermarks: text, angle, and which decoration (for its opacity).
    pub marks: Vec<(Block, f32, usize)>,
}

/// Text laid out, with its left, top, width and colour.
pub(super) type Block = (edits_text::Layout, f32, f32, f32, [f32; 3]);

/// Lays out `decors` on a page at position `index` of `total`.
pub(super) fn lay_out(decors: &[(usize, &Decor)], frame: Frame, index: usize, total: usize) -> PageText {
    let (w, h) = frame.shown();
    let mut plain = Vec::new();
    let mut marks = Vec::new();
    for (k, d) in decors {
        let n = index as i64 + 1 + d.offset as i64;
        for (texts, top_edge) in [(&d.header, true), (&d.footer, false)] {
            for (pos, text) in texts.iter().enumerate() {
                let text = d.fill(text, n, total);
                if text.trim().is_empty() {
                    continue;
                }
                let lay = edits_text::layout(&text, d.size, 0.0);
                let lw = lay.lines.first().map_or(0.0, |l| l.width);
                let left = match pos {
                    0 => d.margin,
                    1 => (w - lw) / 2.0,
                    _ => w - d.margin - lw,
                };
                let top = if top_edge { h - d.margin } else { d.margin + lay.height() };
                plain.push((lay, left, top, lw, d.color));
            }
        }
        let text = d.fill(&d.watermark, n, total);
        if !text.trim().is_empty() {
            let lay = edits_text::layout(&text, d.watermark_size, 0.0);
            let lw = lay.lines.first().map_or(0.0, |l| l.width);
            let (left, top) = (-lw / 2.0, lay.height() / 2.0);
            // Corner to corner, rising.
            let angle = h.atan2(w);
            marks.push(((lay, left, top, lw, d.watermark_color), angle, *k));
        }
    }
    PageText { frame, plain, marks }
}

/// The content stream drawn after the page's own content, given the text
/// operators of its parts (plain first, then one per watermark).
pub(super) fn content(p: &PageText, mut ops: impl Iterator<Item = String>) -> Vec<u8> {
    let (w, h) = p.frame.shown();
    let mut s = String::from("Q\nq\n");
    s += &cm(p.frame.matrix());
    s += &ops.next().unwrap_or_default();
    for (_, angle, k) in &p.marks {
        let (sin, cos) = angle.sin_cos();
        s += &format!("q /EditsGS{k} gs\n");
        s += &cm([cos, sin, -sin, cos, w / 2.0, h / 2.0]);
        s += &ops.next().unwrap_or_default();
        s += "Q\n";
    }
    s += "Q\n";
    s.into_bytes()
}

/// The graphics state for watermark opacity.
pub(super) fn opacity_state(opacity: f32) -> Dict {
    let mut gs = Dict::default();
    gs.set(b"Type", Obj::name("ExtGState"));
    gs.set(b"ca", Obj::Real(opacity as f64));
    gs.set(b"CA", Obj::Real(opacity as f64));
    gs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_turn_shown_coordinates_into_user_space() {
        let apply = |m: [f32; 6], x: f32, y: f32| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        let bx = [0.0, 0.0, 200.0, 100.0];
        // Shown top-left corner, for each rotation, lands on the corner shown there.
        let cases = [(0, (0.0, 100.0)), (90, (0.0, 0.0)), (180, (200.0, 0.0)), (270, (200.0, 100.0))];
        for (rotate, want) in cases {
            let f = Frame { bx, rotate };
            let (_, h) = f.shown();
            assert_eq!(apply(f.matrix(), 0.0, h), want, "rotation {rotate}");
        }
    }

    #[test]
    fn numbers_fill_in() {
        let d = Decor {
            header: Default::default(),
            footer: [String::new(), "Page {n} of {total}".into(), String::new()],
            arabic_digits: false,
            ..serde_json::from_str("{}").unwrap()
        };
        assert_eq!(d.fill(&d.footer[1], 3, 12), "Page 3 of 12");
        let ar = Decor { arabic_digits: true, ..d };
        assert_eq!(ar.fill("{n}/{total}", 3, 12), "\u{663}/\u{661}\u{662}");
    }
}
