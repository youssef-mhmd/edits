//! Annotations as edits keeps them: in PDF user space (points, origin at the
//! bottom-left of the page's MediaBox, before /Rotate), ready to be written
//! into the file.

use serde::{Deserialize, Serialize};

/// Session-wide identifier of an annotation edits knows about.
pub type AnnotId = u64;

/// A PDF indirect object number and generation.
pub type ObjRef = (u32, u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Highlight,
    Underline,
    StrikeOut,
    Squiggly,
}

impl Kind {
    /// The PDF /Subtype name.
    pub fn subtype(self) -> &'static str {
        match self {
            Kind::Highlight => "Highlight",
            Kind::Underline => "Underline",
            Kind::StrikeOut => "StrikeOut",
            Kind::Squiggly => "Squiggly",
        }
    }

    /// Text markup: drawn over or under text, along quadrilaterals.
    pub fn is_markup(self) -> bool {
        true
    }
}

/// One annotation. Geometry is in PDF user space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annot {
    pub page: u32,
    pub kind: Kind,
    /// Bounding rectangle: left, bottom, right, top.
    pub rect: [f32; 4],
    /// Text markup areas, each as /QuadPoints orders them: upper-left,
    /// upper-right, lower-left, lower-right (x, y pairs).
    pub quads: Vec<[f32; 8]>,
    /// Red, green, blue, 0 to 1.
    pub color: [f32; 3],
    /// 0 to 1.
    pub opacity: f32,
    /// Note text (/Contents).
    pub contents: String,
    /// Author (/T).
    pub author: String,
    /// Last change, as a PDF date string (D:YYYYMMDDHHmmSSZ).
    pub modified: String,
}

impl Annot {
    /// The bounding rectangle of the quads, grown by `pad` points.
    pub fn quads_bounds(quads: &[[f32; 8]], pad: f32) -> [f32; 4] {
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for q in quads {
            for i in 0..4 {
                r[0] = r[0].min(q[2 * i]);
                r[1] = r[1].min(q[2 * i + 1]);
                r[2] = r[2].max(q[2 * i]);
                r[3] = r[3].max(q[2 * i + 1]);
            }
        }
        if r[0] > r[2] {
            return [0.0; 4];
        }
        [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad]
    }
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

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        // 2026-11-27 08:00:00 UTC
        assert_eq!(civil(1_795_766_400), (2026, 11, 27, 8, 0, 0));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0, 0));
    }

    #[test]
    fn bounds_of_quads() {
        let q = [[10.0, 20.0, 30.0, 20.0, 10.0, 5.0, 30.0, 5.0]];
        assert_eq!(Annot::quads_bounds(&q, 1.0), [9.0, 4.0, 31.0, 21.0]);
    }
}
