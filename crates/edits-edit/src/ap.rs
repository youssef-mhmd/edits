//! Appearance streams: the drawing other readers show for an annotation.
//! Acrobat draws only what /AP says, so every annotation edits writes has one.

use crate::model::{Annot, Kind};

/// The content stream of an annotation's normal appearance, in user space
/// (the form's /BBox is the annotation's /Rect with an identity /Matrix).
/// Uses the graphics state `/GS0` (opacity, and multiply blending for
/// highlights) from the form's resources.
pub fn content(a: &Annot) -> String {
    let mut s = String::from("/GS0 gs\n");
    let [r, g, b] = a.color;
    match a.kind {
        Kind::Highlight => {
            s += &format!("{} {} {} rg\n", n(r), n(g), n(b));
            for q in &a.quads {
                let (ul, ur, ll, lr) = corners(q);
                s += &format!(
                    "{} {} m {} {} l {} {} l {} {} l h f\n",
                    n(ul.0),
                    n(ul.1),
                    n(ur.0),
                    n(ur.1),
                    n(lr.0),
                    n(lr.1),
                    n(ll.0),
                    n(ll.1)
                );
            }
        }
        Kind::Underline | Kind::StrikeOut => {
            s += &format!("{} {} {} RG\n", n(r), n(g), n(b));
            for q in &a.quads {
                let (ul, _, ll, lr) = corners(q);
                let h = dist(ul, ll);
                let w = (h / 14.0).max(0.5);
                // Along the text's baseline direction, from the lower edge.
                let up = unit((ul.0 - ll.0, ul.1 - ll.1));
                let at = if a.kind == Kind::Underline { w * 1.5 } else { h * 0.45 };
                let p0 = (ll.0 + up.0 * at, ll.1 + up.1 * at);
                let p1 = (lr.0 + up.0 * at, lr.1 + up.1 * at);
                s += &format!("{} w {} {} m {} {} l S\n", n(w), n(p0.0), n(p0.1), n(p1.0), n(p1.1));
            }
        }
        Kind::Squiggly => {
            s += &format!("{} {} {} RG\n", n(r), n(g), n(b));
            for q in &a.quads {
                let (ul, _, ll, lr) = corners(q);
                let h = dist(ul, ll);
                let w = (h / 18.0).max(0.4);
                let amp = (h / 10.0).max(0.8);
                let along = unit((lr.0 - ll.0, lr.1 - ll.1));
                let up = unit((ul.0 - ll.0, ul.1 - ll.1));
                let len = dist(ll, lr);
                let step = amp * 2.0;
                let count = ((len / step).ceil() as usize).clamp(1, 10_000);
                let base = (ll.0 + up.0 * amp, ll.1 + up.1 * amp);
                s += &format!("{} w {} {} m", n(w), n(base.0), n(base.1));
                for i in 1..=count {
                    let t = (i as f32 * step).min(len);
                    let off = if i % 2 == 1 { amp } else { 0.0 };
                    let x = base.0 + along.0 * t - up.0 * off;
                    let y = base.1 + along.1 * t - up.1 * off;
                    s += &format!(" {} {} l", n(x), n(y));
                }
                s += " S\n";
            }
        }
    }
    s
}

/// The graphics state for `/GS0`: opacity, and multiply for highlights so
/// the text under them stays readable.
pub fn graphics_state(a: &Annot) -> Vec<(&'static str, GsValue)> {
    let mut v = vec![("CA", GsValue::Num(a.opacity)), ("ca", GsValue::Num(a.opacity))];
    if a.kind == Kind::Highlight {
        v.push(("BM", GsValue::Name("Multiply")));
    }
    v
}

pub enum GsValue {
    Num(f32),
    Name(&'static str),
}

type Pt = (f32, f32);

/// Upper-left, upper-right, lower-left, lower-right of a /QuadPoints entry.
fn corners(q: &[f32; 8]) -> (Pt, Pt, Pt, Pt) {
    ((q[0], q[1]), (q[2], q[3]), (q[4], q[5]), (q[6], q[7]))
}

fn dist(a: Pt, b: Pt) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn unit(v: Pt) -> Pt {
    let l = (v.0 * v.0 + v.1 * v.1).sqrt();
    if l < 1e-6 { (0.0, 1.0) } else { (v.0 / l, v.1 / l) }
}

/// A number for a content stream: short, never in exponent form.
fn n(v: f32) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annot(kind: Kind) -> Annot {
        Annot {
            page: 0,
            kind,
            rect: [0.0; 4],
            quads: vec![[10.0, 20.0, 50.0, 20.0, 10.0, 6.0, 50.0, 6.0]],
            color: [1.0, 0.5, 0.0],
            opacity: 0.5,
            contents: String::new(),
            author: String::new(),
            modified: String::new(),
        }
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(n(1.0), "1");
        assert_eq!(n(0.5), "0.5");
        assert_eq!(n(-0.0001), "0");
        assert_eq!(n(f32::NAN), "0");
        assert_eq!(n(12.3456), "12.346");
    }

    #[test]
    fn highlight_fills_each_quad() {
        let c = content(&annot(Kind::Highlight));
        assert!(c.contains("1 0.5 0 rg"));
        assert!(c.contains("10 20 m 50 20 l 50 6 l 10 6 l h f"), "{c}");
    }

    #[test]
    fn underline_and_strikeout_are_lines() {
        let u = content(&annot(Kind::Underline));
        let s = content(&annot(Kind::StrikeOut));
        assert!(u.contains(" l S") && s.contains(" l S"));
        assert_ne!(u, s);
        assert!(content(&annot(Kind::Squiggly)).matches(" l").count() > 5);
    }
}
