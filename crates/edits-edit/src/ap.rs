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
        Kind::Ink => {
            s += &format!("{} {} {} RG {} w 1 J 1 j\n", n(r), n(g), n(b), n(a.width.max(0.1)));
            for stroke in &a.ink {
                let Some(first) = stroke.first() else { continue };
                s += &format!("{} {} m", n(first[0]), n(first[1]));
                if stroke.len() == 1 {
                    // A dot: a zero-length segment with round caps.
                    s += &format!(" {} {} l", n(first[0]), n(first[1]));
                }
                for p in &stroke[1..] {
                    s += &format!(" {} {} l", n(p[0]), n(p[1]));
                }
                s += " S\n";
            }
        }
        Kind::Line => {
            let Some([x1, y1, x2, y2]) = a.line else { return s };
            let w = a.width.max(0.1);
            s += &format!("{} {} {} RG {} {} {} rg {} w 1 J 1 j\n", n(r), n(g), n(b), n(r), n(g), n(b), n(w));
            let (dx, dy) = (x2 - x1, y2 - y1);
            let len = (dx * dx + dy * dy).sqrt().max(1e-3);
            let (ux, uy) = (dx / len, dy / len);
            let head = (w * 4.0 + 4.0).min(len * 0.8);
            // The shaft stops inside the arrowhead, so its end is not blunt.
            let (ex, ey) = if a.arrow { (x2 - ux * head * 0.8, y2 - uy * head * 0.8) } else { (x2, y2) };
            s += &format!("{} {} m {} {} l S\n", n(x1), n(y1), n(ex), n(ey));
            if a.arrow {
                let (bx, by) = (x2 - ux * head, y2 - uy * head);
                let (px, py) = (-uy * head * 0.45, ux * head * 0.45);
                s += &format!(
                    "{} {} m {} {} l {} {} l h f\n",
                    n(x2),
                    n(y2),
                    n(bx + px),
                    n(by + py),
                    n(bx - px),
                    n(by - py)
                );
            }
        }
        Kind::Square | Kind::Circle => {
            let w = a.width.max(0.0);
            let h = w / 2.0;
            let [l, bt, rt, tp] = a.rect;
            let (l, bt, rt, tp) = (l + h, bt + h, (rt - h).max(l + h), (tp - h).max(bt + h));
            s += &format!("{} {} {} RG {} w\n", n(r), n(g), n(b), n(w));
            if let Some([fr, fg, fb]) = a.fill {
                s += &format!("{} {} {} rg\n", n(fr), n(fg), n(fb));
            }
            if a.kind == Kind::Square {
                s += &format!("{} {} {} {} re", n(l), n(bt), n(rt - l), n(tp - bt));
            } else {
                // Four Bezier arcs.
                let (cx, cy, rx, ry) = ((l + rt) / 2.0, (bt + tp) / 2.0, (rt - l) / 2.0, (tp - bt) / 2.0);
                let k = 0.552_284_8;
                let (ox, oy) = (rx * k, ry * k);
                s += &format!("{} {} m ", n(cx + rx), n(cy));
                s +=
                    &format!("{} {} {} {} {} {} c ", n(cx + rx), n(cy + oy), n(cx + ox), n(cy + ry), n(cx), n(cy + ry));
                s +=
                    &format!("{} {} {} {} {} {} c ", n(cx - ox), n(cy + ry), n(cx - rx), n(cy + oy), n(cx - rx), n(cy));
                s +=
                    &format!("{} {} {} {} {} {} c ", n(cx - rx), n(cy - oy), n(cx - ox), n(cy - ry), n(cx), n(cy - ry));
                s += &format!(
                    "{} {} {} {} {} {} c h",
                    n(cx + ox),
                    n(cy - ry),
                    n(cx + rx),
                    n(cy - oy),
                    n(cx + rx),
                    n(cy)
                );
            }
            s += match (a.fill.is_some(), w > 0.0) {
                (true, true) => " B\n",
                (true, false) => " f\n",
                (false, true) => " S\n",
                (false, false) => " n\n",
            };
        }
        Kind::Note => {
            // A note icon: a coloured sheet with three lines of "text".
            let [l, bt, rt, tp] = a.rect;
            let (w, h) = (rt - l, tp - bt);
            s += &format!(
                "{} {} {} rg 0.25 0.25 0.25 RG 0.8 w {} {} {} {} re B\n1 1 1 RG 1.4 w 1 J\n",
                n(r),
                n(g),
                n(b),
                n(l + 1.0),
                n(bt + 1.0),
                n(w - 2.0),
                n(h - 2.0)
            );
            for k in [0.7, 0.5, 0.3] {
                let y = bt + h * k;
                s += &format!("{} {} m {} {} l S\n", n(l + w * 0.22), n(y), n(rt - w * 0.22), n(y));
            }
        }
        Kind::FreeText | Kind::Stamp => {
            // The box; the text or picture is added by the caller, which
            // owns the font and image resources.
            let [l, bt, rt, tp] = a.rect;
            if let Some([fr, fg, fb]) = a.fill {
                s +=
                    &format!("{} {} {} rg {} {} {} {} re f\n", n(fr), n(fg), n(fb), n(l), n(bt), n(rt - l), n(tp - bt));
            }
            let w = if a.kind == Kind::Stamp && a.image.is_none() { (a.font_size / 10.0).max(1.5) } else { a.width };
            if w > 0.0 && (a.kind == Kind::FreeText || a.image.is_none()) {
                let h = w / 2.0;
                s += &format!(
                    "{} {} {} RG {} w {} {} {} {} re S\n",
                    n(r),
                    n(g),
                    n(b),
                    n(w),
                    n(l + h),
                    n(bt + h),
                    n(rt - l - w),
                    n(tp - bt - w)
                );
            }
        }
        Kind::Other => {}
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
        let mut a = Annot::new(0, kind);
        a.quads = vec![[10.0, 20.0, 50.0, 20.0, 10.0, 6.0, 50.0, 6.0]];
        a.color = [1.0, 0.5, 0.0];
        a.opacity = 0.5;
        a
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

    #[test]
    fn shapes_lines_and_ink() {
        let mut sq = annot(Kind::Square);
        sq.rect = [0.0, 0.0, 20.0, 10.0];
        sq.width = 2.0;
        assert!(content(&sq).contains("1 1 18 8 re S"), "{}", content(&sq));
        sq.fill = Some([0.0, 0.0, 1.0]);
        assert!(content(&sq).contains("re B"));
        let mut c = annot(Kind::Circle);
        c.rect = [0.0, 0.0, 20.0, 10.0];
        assert_eq!(content(&c).matches(" c").count(), 4);
        let mut l = annot(Kind::Line);
        l.line = Some([0.0, 0.0, 100.0, 0.0]);
        l.arrow = true;
        assert!(content(&l).contains("h f"), "arrowhead");
        let mut ink = annot(Kind::Ink);
        ink.ink = vec![vec![[0.0, 0.0], [5.0, 5.0]], vec![[9.0, 9.0]]];
        assert_eq!(content(&ink).matches(" S").count(), 2);
    }
}
