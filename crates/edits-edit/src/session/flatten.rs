//! Flattening (PRD 5.4): every annotation and form field is drawn into its
//! page's content and removed, so what was filled in or marked up can no
//! longer be changed or hidden by other readers. Links stay links.
//!
//! Each annotation's normal appearance (a Form XObject) is placed on the
//! page the way readers place it (ISO 32000-2, 12.5.5): its bounding box,
//! turned by its matrix, is fitted to the annotation's rectangle.

use std::collections::HashMap;

use super::Session;
use super::pages::Out;
use crate::pdf::object::real;
use crate::pdf::{Dict, Obj, PdfError, Ref};

/// Annotation flags: hidden (2) and no view (32).
const NOT_SHOWN: i64 = 2 | 32;

impl Session {
    /// Draws the annotations of the pages `pages` (written in `out`) into
    /// their content and keeps only links.
    pub(super) fn flatten_pages(&self, out: &mut Out, pages: &[Ref]) -> Result<(), PdfError> {
        let written: HashMap<Ref, usize> = out.objects.iter().enumerate().map(|(i, (r, _))| (*r, i)).collect();
        // An object as this update has it, or as the file has it.
        let get = |out: &Out, r: Ref| -> Result<Obj, PdfError> {
            match written.get(&r) {
                Some(i) => Ok(out.objects[*i].1.clone()),
                None => self.base.get(r),
            }
        };
        let resolve = |out: &Out, o: &Obj| -> Result<Obj, PdfError> {
            match o {
                Obj::Ref(r) => get(out, *r),
                other => Ok(other.clone()),
            }
        };
        let open = out.add(Obj::Stream(Dict::default(), b"q\n".to_vec()));
        for page in pages {
            let Some(&at) = written.get(page) else { continue };
            let Obj::Dict(dict) = out.objects[at].1.clone() else { continue };
            let annots = match dict.get(b"Annots") {
                Some(a) => resolve(out, a)?.as_array().map(<[Obj]>::to_vec).unwrap_or_default(),
                None => continue,
            };
            let mut kept = Vec::new();
            let mut ops = String::new();
            let mut xobjects = Vec::new();
            for a in annots {
                let Ok(ad) = resolve(out, &a) else { continue };
                let Some(ad) = ad.as_dict() else { continue };
                match ad.name(b"Subtype") {
                    Some(b"Link") => {
                        kept.push(a);
                        continue;
                    }
                    Some(b"Popup") => continue,
                    _ => {}
                }
                let flags = ad.get(b"F").and_then(Obj::as_int).unwrap_or(0);
                if flags & NOT_SHOWN != 0 {
                    continue;
                }
                let Some(ap) = self.normal_appearance(out, ad, &resolve)? else { continue };
                let Some(rect) = ad.get(b"Rect").and_then(Obj::numbers).filter(|r| r.len() == 4) else { continue };
                let form = get(out, ap)?;
                let Some(fd) = form.as_dict() else { continue };
                let Some(m) = placement(
                    fd,
                    [rect[0].min(rect[2]), rect[1].min(rect[3]), rect[0].max(rect[2]), rect[1].max(rect[3])],
                ) else {
                    continue;
                };
                let name = format!("EditsFl{}", xobjects.len());
                ops +=
                    &format!("q {} cm /{name} Do Q\n", m.iter().map(|v| real(*v as f64)).collect::<Vec<_>>().join(" "));
                xobjects.push((name, ap));
            }
            let Obj::Dict(d) = &mut out.objects[at].1 else { continue };
            if kept.is_empty() {
                d.remove(b"Annots");
            } else {
                d.set(b"Annots", Obj::Array(kept));
            }
            if xobjects.is_empty() {
                continue;
            }
            let content = out.add(Obj::Stream(Dict::default(), format!("Q\n{ops}").into_bytes()));
            let Obj::Dict(d) = &mut out.objects[at].1 else { continue };
            let mut contents = vec![Obj::Ref(open)];
            match d.get(b"Contents") {
                Some(Obj::Array(a)) => contents.extend(a.iter().cloned()),
                Some(c @ Obj::Ref(_)) => contents.push(c.clone()),
                _ => {}
            }
            contents.push(Obj::Ref(content));
            d.set(b"Contents", Obj::Array(contents));
            let mut res = d.get(b"Resources").and_then(Obj::as_dict).cloned().unwrap_or_default();
            let mut xo = match res.get(b"XObject") {
                Some(Obj::Dict(x)) => x.clone(),
                Some(Obj::Ref(r)) => self.base.get(*r)?.as_dict().cloned().unwrap_or_default(),
                _ => Dict::default(),
            };
            for (name, r) in xobjects {
                xo.set(name.as_bytes(), Obj::Ref(r));
            }
            res.set(b"XObject", Obj::Dict(xo));
            d.set(b"Resources", Obj::Dict(res));
        }
        Ok(())
    }

    /// The annotation's normal appearance stream: /AP /N, or the state /AS
    /// names when /N holds several.
    fn normal_appearance(
        &self,
        out: &Out,
        ad: &Dict,
        resolve: &dyn Fn(&Out, &Obj) -> Result<Obj, PdfError>,
    ) -> Result<Option<Ref>, PdfError> {
        let Some(ap) = ad.get(b"AP") else { return Ok(None) };
        let ap = resolve(out, ap)?;
        let Some(n) = ap.as_dict().and_then(|d| d.get(b"N")) else { return Ok(None) };
        match n {
            Obj::Ref(r) => {
                // A stream, or a dictionary of states held as its own object.
                let o = resolve(out, n)?;
                match o {
                    Obj::Stream(..) => Ok(Some(*r)),
                    Obj::Dict(states) => Ok(state(&states, ad)),
                    _ => Ok(None),
                }
            }
            Obj::Dict(states) => Ok(state(states, ad)),
            _ => Ok(None),
        }
    }
}

fn state(states: &Dict, ad: &Dict) -> Option<Ref> {
    let name = ad.name(b"AS")?;
    states.get(name).and_then(Obj::as_ref)
}

/// The matrix that puts a form with dictionary `fd` into `rect`: its
/// /BBox, turned by its /Matrix, fitted to the rectangle.
fn placement(fd: &Dict, rect: [f32; 4]) -> Option<[f32; 6]> {
    let bb = fd.get(b"BBox").and_then(Obj::numbers).filter(|b| b.len() == 4)?;
    let m =
        fd.get(b"Matrix").and_then(Obj::numbers).filter(|m| m.len() == 6).unwrap_or(vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let apply = |x: f32, y: f32| (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
    let corners = [apply(bb[0], bb[1]), apply(bb[2], bb[1]), apply(bb[0], bb[3]), apply(bb[2], bb[3])];
    let (x0, x1) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), c| (a.min(c.0), b.max(c.0)));
    let (y0, y1) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), c| (a.min(c.1), b.max(c.1)));
    if x1 - x0 <= 0.0 || y1 - y0 <= 0.0 {
        return None;
    }
    let sx = (rect[2] - rect[0]) / (x1 - x0);
    let sy = (rect[3] - rect[1]) / (y1 - y0);
    // A = scale and move the transformed box onto the rectangle; the form's
    // own matrix is applied by the reader inside Do.
    Some([sx, 0.0, 0.0, sy, rect[0] - x0 * sx, rect[1] - y0 * sy])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appearances_fit_their_rectangle() {
        let mut fd = Dict::default();
        fd.set(b"BBox", Obj::Array(vec![Obj::Int(0), Obj::Int(0), Obj::Int(20), Obj::Int(10)]));
        assert_eq!(placement(&fd, [100.0, 200.0, 140.0, 220.0]), Some([2.0, 0.0, 0.0, 2.0, 100.0, 200.0]));
        // Turned a quarter: the box is 10 wide, 20 high after the matrix.
        fd.set(b"Matrix", Obj::Array([0, 1, -1, 0, 0, 0].iter().map(|v| Obj::Int(*v)).collect()));
        let m = placement(&fd, [0.0, 0.0, 10.0, 20.0]).unwrap();
        assert_eq!(m, [1.0, 0.0, 0.0, 1.0, 10.0, 0.0]);
    }
}
