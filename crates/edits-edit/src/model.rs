//! Annotations as edits keeps them: in PDF user space (points, origin at the
//! bottom-left of the page's MediaBox, before /Rotate), ready to be written
//! into the file.

use serde::{Deserialize, Serialize};

use crate::pdf::{Dict, Obj};

/// Session-wide identifier of an annotation edits knows about.
pub type AnnotId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
    Ink,
    Line,
    Square,
    Circle,
    /// Any other kind found in a file: kept as it is, can be moved or deleted.
    Other,
}

impl Kind {
    /// The PDF /Subtype name.
    pub fn subtype(self) -> &'static str {
        match self {
            Kind::Highlight => "Highlight",
            Kind::Underline => "Underline",
            Kind::StrikeOut => "StrikeOut",
            Kind::Squiggly => "Squiggly",
            Kind::Ink => "Ink",
            Kind::Line => "Line",
            Kind::Square => "Square",
            Kind::Circle => "Circle",
            Kind::Other => "",
        }
    }

    pub fn from_subtype(s: &[u8]) -> Kind {
        match s {
            b"Highlight" => Kind::Highlight,
            b"Underline" => Kind::Underline,
            b"StrikeOut" => Kind::StrikeOut,
            b"Squiggly" => Kind::Squiggly,
            b"Ink" => Kind::Ink,
            b"Line" => Kind::Line,
            b"Square" => Kind::Square,
            b"Circle" => Kind::Circle,
            _ => Kind::Other,
        }
    }

    /// Text markup: drawn along the text, by quadrilaterals; not moved on its own.
    pub fn is_markup(self) -> bool {
        matches!(self, Kind::Highlight | Kind::Underline | Kind::StrikeOut | Kind::Squiggly)
    }

    /// edits can draw its appearance (so colour and width can change).
    pub fn drawable(self) -> bool {
        self != Kind::Other
    }
}

/// Kinds of annotation that are part of a form or of navigation, not
/// comments; they are never listed or edited as annotations.
pub fn is_structural(subtype: &[u8]) -> bool {
    matches!(subtype, b"Link" | b"Widget" | b"Popup" | b"PrinterMark" | b"TrapNet" | b"Watermark" | b"Screen")
}

/// One annotation. Geometry is in PDF user space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annot {
    pub page: u32,
    pub kind: Kind,
    /// The PDF subtype as written in the file (for `Other`).
    #[serde(default)]
    pub subtype: String,
    /// Bounding rectangle: left, bottom, right, top.
    pub rect: [f32; 4],
    /// Text markup areas, each as /QuadPoints orders them: upper-left,
    /// upper-right, lower-left, lower-right (x, y pairs).
    #[serde(default)]
    pub quads: Vec<[f32; 8]>,
    /// Freehand strokes.
    #[serde(default)]
    pub ink: Vec<Vec<[f32; 2]>>,
    /// A line's ends: x1, y1, x2, y2.
    #[serde(default)]
    pub line: Option<[f32; 4]>,
    /// A line with an arrowhead at its end.
    #[serde(default)]
    pub arrow: bool,
    /// Red, green, blue, 0 to 1 (stroke, or the markup colour).
    pub color: [f32; 3],
    /// Fill of rectangles and ellipses.
    #[serde(default)]
    pub fill: Option<[f32; 3]>,
    /// 0 to 1.
    pub opacity: f32,
    /// Line width in points.
    #[serde(default = "one")]
    pub width: f32,
    /// Note text (/Contents).
    #[serde(default)]
    pub contents: String,
    /// Author (/T).
    #[serde(default)]
    pub author: String,
    /// Last change, as a PDF date string (D:YYYYMMDDHHmmSSZ).
    #[serde(default)]
    pub modified: String,
    /// edits draws the appearance stream. False for annotations from
    /// other apps until their look is changed: their own appearance is kept.
    #[serde(default = "yes")]
    pub own_ap: bool,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

impl Annot {
    pub fn new(page: u32, kind: Kind) -> Self {
        Annot {
            page,
            kind,
            subtype: kind.subtype().into(),
            rect: [0.0; 4],
            quads: Vec::new(),
            ink: Vec::new(),
            line: None,
            arrow: false,
            color: [0.0; 3],
            fill: None,
            opacity: 1.0,
            width: 1.0,
            contents: String::new(),
            author: String::new(),
            modified: String::new(),
            own_ap: true,
        }
    }

    /// The bounding rectangle of the quads, grown by `pad` points.
    pub fn quads_bounds(quads: &[[f32; 8]], pad: f32) -> [f32; 4] {
        bounds(quads.iter().flat_map(|q| (0..4).map(move |i| [q[2 * i], q[2 * i + 1]])), pad)
    }

    /// Recomputes /Rect from the geometry (for kinds that have geometry).
    pub fn fit_rect(&mut self) {
        let pad = self.width.max(1.0) / 2.0 + 1.0;
        let r = match self.kind {
            k if k.is_markup() => Annot::quads_bounds(&self.quads, 1.0),
            Kind::Ink => bounds(self.ink.iter().flatten().copied(), pad),
            Kind::Line => {
                let Some(l) = self.line else { return };
                // Room for an arrowhead.
                let pad = if self.arrow { pad + self.width.max(1.0) * 4.0 } else { pad };
                bounds([[l[0], l[1]], [l[2], l[3]]].into_iter(), pad)
            }
            _ => return,
        };
        self.rect = r;
    }

    /// Moves the annotation by `dx`, `dy` points.
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.rect = [self.rect[0] + dx, self.rect[1] + dy, self.rect[2] + dx, self.rect[3] + dy];
        for q in &mut self.quads {
            for i in 0..4 {
                q[2 * i] += dx;
                q[2 * i + 1] += dy;
            }
        }
        for s in &mut self.ink {
            for p in s {
                p[0] += dx;
                p[1] += dy;
            }
        }
        if let Some(l) = &mut self.line {
            *l = [l[0] + dx, l[1] + dy, l[2] + dx, l[3] + dy];
        }
    }

    /// Fits the annotation into a new rectangle, scaling its geometry.
    pub fn resize(&mut self, to: [f32; 4]) {
        let from = self.rect;
        let sx = (to[2] - to[0]) / (from[2] - from[0]).max(1e-3);
        let sy = (to[3] - to[1]) / (from[3] - from[1]).max(1e-3);
        let map = |p: &mut [f32; 2]| {
            p[0] = to[0] + (p[0] - from[0]) * sx;
            p[1] = to[1] + (p[1] - from[1]) * sy;
        };
        for s in &mut self.ink {
            s.iter_mut().for_each(map);
        }
        if let Some(l) = &mut self.line {
            let (mut a, mut b) = ([l[0], l[1]], [l[2], l[3]]);
            map(&mut a);
            map(&mut b);
            *l = [a[0], a[1], b[0], b[1]];
        }
        self.rect = to;
    }

    /// Reads an annotation dictionary. None for kinds that are not comments.
    pub fn from_dict(page: u32, d: &Dict) -> Option<Annot> {
        let subtype = d.name(b"Subtype")?;
        if is_structural(subtype) {
            return None;
        }
        let kind = Kind::from_subtype(subtype);
        let nums = |k: &[u8]| d.get(k).and_then(Obj::numbers);
        let rect = nums(b"Rect").filter(|r| r.len() == 4)?;
        let rect = [rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3])];
        let color = |k: &[u8]| -> Option<[f32; 3]> {
            let c = nums(k)?;
            match c.len() {
                1 => Some([c[0]; 3]),
                3 => Some([c[0], c[1], c[2]]),
                4 => Some([(1.0 - c[0]) * (1.0 - c[3]), (1.0 - c[1]) * (1.0 - c[3]), (1.0 - c[2]) * (1.0 - c[3])]),
                _ => None,
            }
        };
        let width = d
            .get(b"BS")
            .and_then(Obj::as_dict)
            .and_then(|bs| bs.get(b"W"))
            .and_then(Obj::as_f32)
            .or_else(|| nums(b"Border").and_then(|b| b.get(2).copied()))
            .unwrap_or(1.0);
        let mut a = Annot::new(page, kind);
        a.subtype = String::from_utf8_lossy(subtype).into_owned();
        a.rect = rect;
        a.color = color(b"C").unwrap_or([0.0; 3]);
        a.fill = color(b"IC");
        a.opacity = d.get(b"CA").and_then(Obj::as_f32).unwrap_or(1.0).clamp(0.0, 1.0);
        a.width = width.clamp(0.0, 100.0);
        a.contents = d.get(b"Contents").and_then(Obj::text).unwrap_or_default();
        a.author = d.get(b"T").and_then(Obj::text).unwrap_or_default();
        a.modified = d.get(b"M").and_then(Obj::text).unwrap_or_default();
        a.own_ap = false;
        if kind.is_markup() {
            let q = nums(b"QuadPoints").unwrap_or_default();
            a.quads = q.as_chunks::<8>().0.to_vec();
        }
        if kind == Kind::Ink {
            a.ink = d
                .get(b"InkList")
                .and_then(Obj::as_array)
                .map(|strokes| strokes.iter().filter_map(Obj::numbers).map(|s| s.as_chunks::<2>().0.to_vec()).collect())
                .unwrap_or_default();
        }
        if kind == Kind::Line {
            a.line = nums(b"L").filter(|l| l.len() == 4).map(|l| [l[0], l[1], l[2], l[3]]);
            a.arrow = d
                .get(b"LE")
                .and_then(Obj::as_array)
                .and_then(|le| le.get(1))
                .is_some_and(|e| matches!(e, Obj::Name(n) if n != b"None"));
        }
        Some(a)
    }
}

/// Bounding rectangle of points, grown by `pad`.
fn bounds(points: impl Iterator<Item = [f32; 2]>, pad: f32) -> [f32; 4] {
    let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for p in points {
        r[0] = r[0].min(p[0]);
        r[1] = r[1].min(p[1]);
        r[2] = r[2].max(p[0]);
        r[3] = r[3].max(p[1]);
    }
    if r[0] > r[2] {
        return [0.0; 4];
    }
    [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad]
}

/// The current time as a PDF date string.
pub fn pdf_date_now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (y, mo, d, h, mi, s) = civil(secs as i64);
    format!("D:{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z")
}

/// Unix seconds to UTC calendar date and time (Howard Hinnant's algorithm).
fn civil(t: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = t.div_euclid(86_400);
    let rem = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, (rem / 3600) as u32, (rem % 3600 / 60) as u32, (rem % 60) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::object::parse;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0, 0));
    }

    #[test]
    fn bounds_of_quads() {
        let q = [[10.0, 20.0, 30.0, 20.0, 10.0, 5.0, 30.0, 5.0]];
        assert_eq!(Annot::quads_bounds(&q, 1.0), [9.0, 4.0, 31.0, 21.0]);
    }

    #[test]
    fn reads_annotations_from_other_apps() {
        let src = b"<< /Type /Annot /Subtype /Ink /Rect [10 10 60 40] /C [0 0 1] /CA 0.5 /BS << /W 3 >> \
                    /InkList [[10 10 20 20 30 10] [40 40 50 30]] /Contents (hi) /T <FEFF00410042> >>";
        let d = parse(src, &mut 0).unwrap();
        let a = Annot::from_dict(2, d.as_dict().unwrap()).unwrap();
        assert_eq!(a.kind, Kind::Ink);
        assert_eq!(a.ink.len(), 2);
        assert_eq!(a.ink[0][2], [30.0, 10.0]);
        assert_eq!(a.width, 3.0);
        assert_eq!(a.opacity, 0.5);
        assert_eq!(a.author, "AB");
        assert!(!a.own_ap);
        let link = parse(b"<< /Subtype /Link /Rect [0 0 1 1] >>", &mut 0).unwrap();
        assert!(Annot::from_dict(0, link.as_dict().unwrap()).is_none());
        let stamp = parse(b"<< /Subtype /Stamp /Rect [0 0 10 10] >>", &mut 0).unwrap();
        assert_eq!(Annot::from_dict(0, stamp.as_dict().unwrap()).unwrap().kind, Kind::Other);
    }

    #[test]
    fn translate_and_resize_move_the_geometry() {
        let mut a = Annot::new(0, Kind::Line);
        a.line = Some([0.0, 0.0, 10.0, 10.0]);
        a.rect = [0.0, 0.0, 10.0, 10.0];
        a.translate(5.0, 1.0);
        assert_eq!(a.line, Some([5.0, 1.0, 15.0, 11.0]));
        a.resize([5.0, 1.0, 25.0, 21.0]);
        assert_eq!(a.line, Some([5.0, 1.0, 25.0, 21.0]));
    }
}
