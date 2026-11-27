//! Filling form fields (PRD 5.4). A field's value has a history like an
//! annotation's; the update writes the field's new /V and, for each of its
//! widgets, the state shown (check boxes, radio buttons) or a new
//! appearance drawn by edits (text, combo boxes, lists), so every reader
//! shows the value without having to draw it. Text is shaped (Arabic) and
//! uses the bundled font, one subset shared by all fields in the update.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::pages::Out;
use super::{Command, EditError, Session, font_objects, reals};
use crate::forms::{Field, FieldKind, FieldValue, read_fields};
use crate::pdf::object::{encode_text, real};
use crate::pdf::{Dict, Obj, PdfError, Ref};

/// A field's values over time; `cur` is in effect, `saved` is in the base file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FieldEntry {
    pub versions: Vec<FieldValue>,
    pub cur: usize,
    pub saved: usize,
}

/// A widget as the interface sees it: its page now and its box in user space.
#[derive(Debug, Clone, Serialize)]
pub struct WidgetView {
    pub page: u32,
    pub rect: [f32; 4],
    pub on: Option<String>,
}

/// A field as the interface sees it.
#[derive(Debug, Clone, Serialize)]
pub struct FieldView {
    /// Object number and generation of the field: its id.
    pub id: (u32, u16),
    #[serde(flatten)]
    pub field: Field,
    pub views: Vec<WidgetView>,
}

/// Padding inside a field's box, in points.
const PAD: f32 = 2.0;

impl Session {
    /// The form's fields as read from the base file (read once).
    pub(super) fn base_fields(&mut self) -> Result<&Vec<Field>, PdfError> {
        if self.fields.is_none() {
            self.fields = Some(read_fields(&self.base)?);
        }
        Ok(self.fields.as_ref().expect("just read"))
    }

    /// Every field with its value now and where its widgets are now.
    /// Widgets on deleted pages are left out (and fields left without any).
    pub fn fields(&mut self) -> Result<Vec<FieldView>, PdfError> {
        if self.flattened {
            return Ok(Vec::new());
        }
        let base = self.base_fields()?.clone();
        let mut out = Vec::new();
        for mut f in base {
            if let Some(e) = self.field_values.get(&f.obj) {
                f.value = e.versions[e.cur].clone();
            }
            let views: Vec<WidgetView> = f
                .widgets
                .iter()
                .filter_map(|w| {
                    // A base page's widgets are on its first slot (its key is its index).
                    Some(WidgetView { page: self.index_of(w.page)?, rect: w.rect, on: w.on.clone() })
                })
                .collect();
            if views.is_empty() {
                continue;
            }
            out.push(FieldView { id: f.obj, field: f, views });
        }
        Ok(out)
    }

    /// Sets a field's value. Returns the pages to draw again.
    pub fn set_field(&mut self, id: Ref, value: FieldValue) -> Result<Vec<u32>, EditError> {
        let field = self
            .base_fields()?
            .iter()
            .find(|f| f.obj == id)
            .cloned()
            .ok_or_else(|| EditError::Pages("that field is gone".into()))?;
        if field.read_only {
            return Err(EditError::Pages("this field cannot be changed".into()));
        }
        let value = normalise(&field, value)?;
        let now = self.field_values.get(&id).map_or(&field.value, |e| &e.versions[e.cur]);
        if *now == value {
            return Ok(Vec::new());
        }
        let e = self.field_values.entry(id).or_insert_with(|| FieldEntry {
            versions: vec![field.value.clone()],
            cur: 0,
            saved: 0,
        });
        let from = e.cur;
        e.versions.push(value);
        e.cur = e.versions.len() - 1;
        let to = e.cur;
        self.push(Command::Field { obj: id, from, to });
        self.rebuild()?;
        Ok(self.field_pages(id))
    }

    /// The pages (indexes now) a field's widgets are on.
    pub(super) fn field_pages(&self, id: Ref) -> Vec<u32> {
        let mut v: Vec<u32> = self
            .fields
            .iter()
            .flatten()
            .filter(|f| f.obj == id)
            .flat_map(|f| f.widgets.iter().filter_map(|w| self.index_of(w.page)))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Writes changed fields: values, states shown, new appearances, and the
    /// form without XFA (readers that know XFA would show its old data).
    pub(super) fn field_objects(
        &self,
        out: &mut Out,
        cat: &mut Option<Dict>,
        catalog: Option<&Dict>,
    ) -> Result<(), PdfError> {
        let mut changed: BTreeMap<Ref, &FieldValue> =
            self.field_values.iter().filter(|(_, e)| e.cur != e.saved).map(|(r, e)| (*r, &e.versions[e.cur])).collect();
        // Flattening needs a drawing of every filled field: those the file
        // left for readers to draw (no appearance) get one from edits.
        let read;
        let fields = match &self.fields {
            Some(f) => f,
            None if self.flattened => {
                read = read_fields(&self.base)?;
                &read
            }
            None => return Ok(()),
        };
        if self.flattened {
            for f in fields {
                let filled = match &f.value {
                    FieldValue::Text(t) => !t.is_empty(),
                    FieldValue::Choice(c) => !c.is_empty(),
                    FieldValue::State(_) => false,
                };
                let bare = f.widgets.iter().any(|w| {
                    self.base.get(w.obj).ok().and_then(|o| o.as_dict().map(|d| d.get(b"AP").is_none())).unwrap_or(false)
                });
                if filled && bare && matches!(f.kind, FieldKind::Text | FieldKind::Combo | FieldKind::List) {
                    changed.entry(f.obj).or_insert(&f.value);
                }
            }
        }
        if changed.is_empty() {
            return Ok(());
        }
        // Dictionaries to write, by object: a field and its widget are often one object.
        let mut dicts: BTreeMap<Ref, Dict> = BTreeMap::new();
        let get = |r: Ref, dicts: &mut BTreeMap<Ref, Dict>| -> Result<(), PdfError> {
            if let std::collections::btree_map::Entry::Vacant(v) = dicts.entry(r) {
                v.insert(self.base.get(r)?.as_dict().cloned().unwrap_or_default());
            }
            Ok(())
        };
        // Text drawn into appearances: (widget, field, lines) collected first, one font for all.
        let mut drawn: Vec<(Ref, &Field, Appearance)> = Vec::new();
        for f in fields.iter().filter(|f| changed.contains_key(&f.obj)) {
            let value = changed[&f.obj];
            get(f.obj, &mut dicts)?;
            let d = dicts.get_mut(&f.obj).expect("just read");
            match value {
                FieldValue::Text(t) => d.set(b"V", encode_text(t)),
                FieldValue::State(Some(s)) => d.set(b"V", Obj::name(s)),
                FieldValue::State(None) => d.set(b"V", Obj::name("Off")),
                FieldValue::Choice(c) if f.multi_select => {
                    d.set(b"V", Obj::Array(c.iter().map(|v| encode_text(v)).collect()));
                }
                FieldValue::Choice(c) => match c.first() {
                    Some(v) => d.set(b"V", encode_text(v)),
                    None => d.remove(b"V"),
                },
            }
            if f.kind == FieldKind::List {
                let idx: Vec<Obj> = match value {
                    FieldValue::Choice(c) => f
                        .options
                        .iter()
                        .enumerate()
                        .filter(|(_, o)| c.contains(&o.0))
                        .map(|(i, _)| Obj::Int(i as i64))
                        .collect(),
                    _ => Vec::new(),
                };
                d.set(b"I", Obj::Array(idx));
            } else {
                d.remove(b"I");
            }
            for w in &f.widgets {
                get(w.obj, &mut dicts)?;
                let wd = dicts.get_mut(&w.obj).expect("just read");
                match (f.kind, value) {
                    (FieldKind::Check | FieldKind::Radio, FieldValue::State(s)) => {
                        let on = w.on.is_some() && s.as_ref() == w.on.as_ref();
                        wd.set(b"AS", Obj::name(if on { w.on.as_deref().unwrap_or("Off") } else { "Off" }));
                        if w.draw_states
                            && let Some(name) = &w.on
                        {
                            let (bw, bh) = (w.rect[2] - w.rect[0], w.rect[3] - w.rect[1]);
                            let ap = button_states(out, name, f.kind, bw, bh, f.color);
                            wd.set(b"AP", ap);
                        }
                    }
                    (FieldKind::Text | FieldKind::Combo | FieldKind::List, v) => {
                        drawn.push((w.obj, f, self.appearance_of(f, w, v, wd)));
                    }
                    _ => {}
                }
            }
        }
        if !drawn.is_empty() {
            type Block<'a> = (&'a edits_text::Layout, f32, f32, f32, [f32; 3]);
            let parts: Vec<Vec<Block>> = drawn
                .iter()
                .map(|(_, _, a)| a.text.iter().map(|(l, x, y, w)| (l, *x, *y, *w, a.color)).collect())
                .collect();
            let refs: Vec<&edits_text::Blocks> = parts.iter().map(|p| p.as_slice()).collect();
            let (ops, font) = edits_text::pdf_text_parts(&refs, "EditsF0");
            let font = font_objects(&font, &mut |o| out.add(o));
            for ((w, _, a), text_ops) in drawn.iter().zip(ops) {
                let mut fonts = Dict::default();
                fonts.set(b"EditsF0", Obj::Ref(font));
                let mut res = Dict::default();
                res.set(b"Font", Obj::Dict(fonts));
                let mut form = Dict::default();
                form.set(b"Type", Obj::name("XObject"));
                form.set(b"Subtype", Obj::name("Form"));
                form.set(b"BBox", reals(&[0.0, 0.0, a.w, a.h]));
                if let Some(m) = a.matrix {
                    form.set(b"Matrix", reals(&m));
                }
                form.set(b"Resources", Obj::Dict(res));
                let content = format!(
                    "{}/Tx BMC\nq {} {} {} {} re W n\n{}Q\nEMC\n",
                    a.frame,
                    real(PAD as f64 / 2.0),
                    real(PAD as f64 / 2.0),
                    real((a.w - PAD) as f64),
                    real((a.h - PAD) as f64),
                    a.highlight.clone() + &text_ops
                );
                let ap = out.add(Obj::Stream(form, content.into_bytes()));
                let wd = dicts.get_mut(w).expect("read above");
                let mut apd = Dict::default();
                apd.set(b"N", Obj::Ref(ap));
                wd.set(b"AP", Obj::Dict(apd));
                wd.remove(b"AS");
            }
        }
        for (r, d) in dicts {
            out.objects.push((r, Obj::Dict(d)));
        }
        // No XFA: the form's fields are the truth now.
        if let Some(c) = catalog
            && let Some(af) = c.get(b"AcroForm")
        {
            match af {
                Obj::Ref(r) => {
                    let mut d = self.base.get(*r)?.as_dict().cloned().unwrap_or_default();
                    if d.get(b"XFA").is_some() {
                        d.remove(b"XFA");
                        out.objects.push((*r, Obj::Dict(d)));
                    }
                }
                Obj::Dict(d) if d.get(b"XFA").is_some() => {
                    let mut d = d.clone();
                    d.remove(b"XFA");
                    cat.get_or_insert_with(|| c.clone()).set(b"AcroForm", Obj::Dict(d));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// How a text field, combo box or list widget shows `value`.
    fn appearance_of(&self, f: &Field, w: &crate::forms::Widget, value: &FieldValue, wd: &Dict) -> Appearance {
        let (rw, rh) = (w.rect[2] - w.rect[0], w.rect[3] - w.rect[1]);
        let (bw, bh) = if w.turn % 2 == 1 { (rh, rw) } else { (rw, rh) };
        let matrix = match w.turn {
            1 => Some([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]),
            2 => Some([-1.0, 0.0, 0.0, -1.0, 0.0, 0.0]),
            3 => Some([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]),
            _ => None,
        };
        let frame = frame(&self.base, wd, bw, bh);
        let inner = (bw - 2.0 * PAD).max(1.0);
        let mut text = Vec::new();
        let mut highlight = String::new();
        let shown = |v: &str| f.options.iter().find(|o| o.0 == v).map_or(v.to_string(), |o| o.1.clone());
        match (f.kind, value) {
            (FieldKind::List, FieldValue::Choice(chosen)) => {
                let size = if f.font_size > 0.0 { f.font_size } else { 12.0 };
                let mut top = bh - PAD;
                for (export, label) in &f.options {
                    let lay = edits_text::layout(label, size, 0.0);
                    let lh = lay.height();
                    if chosen.contains(export) {
                        highlight += &format!(
                            "0.6 0.75 0.95 rg {} {} {} {} re f\n",
                            real(PAD as f64 / 2.0),
                            real((top - lh) as f64),
                            real((bw - PAD) as f64),
                            real(lh as f64)
                        );
                    }
                    let lw = lay.lines.first().map_or(0.0, |l| l.width);
                    text.push((lay, PAD, top, lw));
                    top -= lh;
                    if top < 0.0 {
                        break;
                    }
                }
            }
            (_, v) => {
                let s = match v {
                    FieldValue::Text(t) if f.password => "*".repeat(t.chars().count()),
                    FieldValue::Text(t) => t.clone(),
                    FieldValue::Choice(c) => c.first().map(|v| shown(v)).unwrap_or_default(),
                    FieldValue::State(_) => String::new(),
                };
                if s.is_empty() {
                    // Nothing to show.
                } else if f.comb && f.max_len.is_some() && !f.multiline {
                    let cells = f.max_len.unwrap_or(1).max(1) as f32;
                    let cw = bw / cells;
                    let size = if f.font_size > 0.0 { f.font_size } else { (bh * 0.7).min(cw * 1.2).clamp(4.0, 24.0) };
                    for (i, ch) in s.chars().take(cells as usize).enumerate() {
                        let lay = edits_text::layout(&ch.to_string(), size, 0.0);
                        let lw = lay.lines.first().map_or(0.0, |l| l.width);
                        let top = (bh + lay.height()) / 2.0;
                        text.push((lay, cw * i as f32 + (cw - lw) / 2.0, top, lw));
                    }
                } else if f.multiline {
                    let mut size = if f.font_size > 0.0 { f.font_size } else { 12.0 };
                    let mut lay = edits_text::layout(&s, size, inner);
                    while f.font_size <= 0.0 && lay.height() > bh - 2.0 * PAD && size > 4.0 {
                        size -= 0.5;
                        lay = edits_text::layout(&s, size, inner);
                    }
                    let left = PAD;
                    let top = bh - PAD;
                    let width = inner;
                    let aligned = align_lines(lay, f.align, width);
                    text.push((aligned, left, top, width));
                } else {
                    let one = s.replace(['\n', '\r'], " ");
                    let mut size =
                        if f.font_size > 0.0 { f.font_size } else { ((bh - 2.0 * PAD) * 0.75).clamp(4.0, 12.0) };
                    let mut lay = edits_text::layout(&one, size, 0.0);
                    let lw = |l: &edits_text::Layout| l.lines.first().map_or(0.0, |x| x.width);
                    if f.font_size <= 0.0 && lw(&lay) > inner {
                        size = (size * inner / lw(&lay)).max(4.0);
                        lay = edits_text::layout(&one, size, 0.0);
                    }
                    let w = lw(&lay);
                    let rtl = lay.lines.first().is_some_and(|l| l.rtl);
                    let left = match (f.align, rtl) {
                        (1, _) => (bw - w) / 2.0,
                        (2, _) | (0, true) => bw - PAD - w,
                        _ => PAD,
                    };
                    // Centred top to bottom.
                    let top = (bh + lay.height()) / 2.0 - lay.height() * 0.08;
                    text.push((lay, left, top, w));
                }
            }
        }
        Appearance { w: bw, h: bh, matrix, frame, highlight, text, color: f.color }
    }
}

/// "On" and "off" appearances for a box that has none: a tick for check
/// boxes, a dot for radio buttons.
fn button_states(out: &mut Out, on: &str, kind: FieldKind, w: f32, h: f32, color: [f32; 3]) -> Obj {
    let n = |v: f32| real(v as f64);
    let s = w.min(h);
    let (x0, y0) = ((w - s) / 2.0, (h - s) / 2.0);
    let c = format!("{} {} {}", n(color[0]), n(color[1]), n(color[2]));
    let mark = if kind == FieldKind::Radio {
        // A filled circle from four Bezier curves.
        let (cx, cy, r) = (w / 2.0, h / 2.0, s * 0.3);
        let k = r * 0.5523;
        format!(
            "{c} rg {} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c f
",
            n(cx + r),
            n(cy),
            n(cx + r),
            n(cy + k),
            n(cx + k),
            n(cy + r),
            n(cx),
            n(cy + r),
            n(cx - k),
            n(cy + r),
            n(cx - r),
            n(cy + k),
            n(cx - r),
            n(cy),
            n(cx - r),
            n(cy - k),
            n(cx - k),
            n(cy - r),
            n(cx),
            n(cy - r),
            n(cx + k),
            n(cy - r),
            n(cx + r),
            n(cy - k),
            n(cx + r),
            n(cy),
        )
    } else {
        format!(
            "{c} RG {} w 1 J 1 j {} {} m {} {} l {} {} l S
",
            n(s * 0.12),
            n(x0 + s * 0.2),
            n(y0 + s * 0.5),
            n(x0 + s * 0.42),
            n(y0 + s * 0.25),
            n(x0 + s * 0.8),
            n(y0 + s * 0.78),
        )
    };
    let form = |content: String| {
        let mut d = Dict::default();
        d.set(b"Type", Obj::name("XObject"));
        d.set(b"Subtype", Obj::name("Form"));
        d.set(b"BBox", reals(&[0.0, 0.0, w, h]));
        Obj::Stream(d, content.into_bytes())
    };
    let on_ref = out.add(form(mark));
    let off_ref = out.add(form(String::new()));
    let mut states = Dict::default();
    states.set(on.as_bytes(), Obj::Ref(on_ref));
    states.set(b"Off", Obj::Ref(off_ref));
    let mut ap = Dict::default();
    ap.set(b"N", Obj::Dict(states));
    Obj::Dict(ap)
}

/// Lines of a multiline field placed for its alignment: each line keeps its
/// own direction; the block is laid out in `width`.
fn align_lines(lay: edits_text::Layout, align: u8, width: f32) -> edits_text::Layout {
    let mut lay = lay;
    for line in &mut lay.lines {
        // pdf_text puts right-to-left lines at the right of the width given
        // and others at the left; centred and right-aligned text moves here.
        let shift = match (align, line.rtl) {
            (1, _) => (width - line.width) / 2.0 * if line.rtl { -1.0 } else { 1.0 },
            (2, false) => width - line.width,
            _ => 0.0,
        };
        for g in &mut line.glyphs {
            g.x += shift;
        }
    }
    lay
}

/// A widget appearance: its box, its frame (background and border), and text.
struct Appearance {
    w: f32,
    h: f32,
    matrix: Option<[f32; 6]>,
    frame: String,
    highlight: String,
    text: Vec<(edits_text::Layout, f32, f32, f32)>,
    color: [f32; 3],
}

/// The widget's background and border, as /MK and /BS ask.
fn frame<S: crate::pdf::Source>(f: &crate::pdf::PdfFile<S>, wd: &Dict, w: f32, h: f32) -> String {
    let mk = wd.get(b"MK").and_then(|m| f.resolve(m).ok()).and_then(|m| m.as_dict().cloned()).unwrap_or_default();
    let color = |k: &[u8], stroke: bool| -> Option<String> {
        let c = mk.get(k).and_then(Obj::numbers)?;
        let n = |v: f32| real(v as f64);
        Some(match c.len() {
            1 => format!("{} {}", n(c[0]), if stroke { "G" } else { "g" }),
            3 => format!("{} {} {} {}", n(c[0]), n(c[1]), n(c[2]), if stroke { "RG" } else { "rg" }),
            4 => format!("{} {} {} {} {}", n(c[0]), n(c[1]), n(c[2]), n(c[3]), if stroke { "K" } else { "k" }),
            _ => return None,
        })
    };
    let mut s = String::new();
    if let Some(bg) = color(b"BG", false) {
        s += &format!("{bg} 0 0 {} {} re f\n", real(w as f64), real(h as f64));
    }
    let bw = wd
        .get(b"BS")
        .and_then(|b| f.resolve(b).ok())
        .and_then(|b| b.as_dict().and_then(|d| d.get(b"W")).and_then(Obj::as_f32))
        .unwrap_or(1.0);
    if let Some(bc) = color(b"BC", true)
        && bw > 0.0
    {
        let half = bw / 2.0;
        s += &format!(
            "{bc} {} w {} {} {} {} re S\n",
            real(bw as f64),
            real(half as f64),
            real(half as f64),
            real((w - bw) as f64),
            real((h - bw) as f64)
        );
    }
    s
}

/// Checks a value against the field's kind and choices.
fn normalise(f: &Field, v: FieldValue) -> Result<FieldValue, EditError> {
    let bad = || EditError::Pages(format!("that value does not fit the field {}", f.name));
    Ok(match (f.kind, v) {
        (FieldKind::Text, FieldValue::Text(mut t)) => {
            if !f.multiline {
                t = t.replace(['\r', '\n'], " ");
            }
            if let Some(m) = f.max_len {
                t = t.chars().take(m as usize).collect();
            }
            FieldValue::Text(t)
        }
        (FieldKind::Check | FieldKind::Radio, FieldValue::State(s)) => {
            if let Some(on) = &s
                && !f.widgets.iter().any(|w| w.on.as_ref() == Some(on))
            {
                return Err(bad());
            }
            FieldValue::State(s)
        }
        (FieldKind::Combo, FieldValue::Choice(c)) => {
            let c: Vec<String> = c.into_iter().take(1).collect();
            if !f.editable && c.iter().any(|v| !f.options.iter().any(|o| &o.0 == v)) {
                return Err(bad());
            }
            FieldValue::Choice(c)
        }
        (FieldKind::List, FieldValue::Choice(c)) => {
            let c: Vec<String> = c.into_iter().filter(|v| f.options.iter().any(|o| &o.0 == v)).collect();
            FieldValue::Choice(if f.multi_select { c } else { c.into_iter().take(1).collect() })
        }
        _ => return Err(bad()),
    })
}

/// Field histories, keyed for JSON (recovery files).
pub(super) fn to_list(m: &BTreeMap<Ref, FieldEntry>) -> Vec<(Ref, FieldEntry)> {
    m.iter().map(|(r, e)| (*r, e.clone())).collect()
}

pub(super) fn from_list(v: Vec<(Ref, FieldEntry)>) -> BTreeMap<Ref, FieldEntry> {
    v.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::PdfFile;

    fn session() -> Session {
        let dir = std::env::temp_dir().join(format!("edits-fields-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("form-{}.pdf", rand_name()));
        std::fs::write(&p, crate::forms::tests::form_pdf()).unwrap();
        Session::open(&p).unwrap()
    }

    fn rand_name() -> u128 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    }

    fn reread(s: &Session) -> (PdfFile<Vec<u8>>, Vec<Field>) {
        let mut bytes = std::fs::read(s.path()).unwrap();
        if let Some(t) = s.tail() {
            bytes.extend_from_slice(&t);
        }
        let f = PdfFile::open(bytes).unwrap();
        let fields = read_fields(&f).unwrap();
        (f, fields)
    }

    #[test]
    fn filled_values_are_written_with_appearances() {
        let mut s = session();
        let ids: Vec<Ref> = s.fields().unwrap().iter().map(|f| f.field.obj).collect();
        s.set_field(ids[0], FieldValue::Text("علي Ali\nsecond".into())).unwrap();
        s.set_field(ids[1], FieldValue::State(Some("Yes".into()))).unwrap();
        s.set_field(ids[2], FieldValue::State(Some("S".into()))).unwrap();
        s.set_field(ids[3], FieldValue::Choice(vec!["CAI".into()])).unwrap();
        s.set_field(ids[4], FieldValue::Choice(vec!["English".into(), "Nope".into()])).unwrap();
        assert!(s.set_field(ids[2], FieldValue::State(Some("XL".into()))).is_err(), "no such state");
        assert!(s.set_field(ids[3], FieldValue::Choice(vec!["Giza".into()])).is_err(), "not editable");

        let (f, fields) = reread(&s);
        assert_eq!(fields[0].value, FieldValue::Text("علي Ali second".into()), "one line");
        assert_eq!(fields[1].value, FieldValue::State(Some("Yes".into())));
        assert_eq!(fields[2].value, FieldValue::State(Some("S".into())));
        assert_eq!(fields[3].value, FieldValue::Choice(vec!["CAI".into()]));
        assert_eq!(fields[4].value, FieldValue::Choice(vec!["English".into()]));
        // Radio widgets show the chosen one.
        let state = |r: Ref| f.get(r).unwrap().as_dict().unwrap().name(b"AS").map(<[u8]>::to_vec);
        assert_eq!(state(fields[2].widgets[0].obj), Some(b"S".to_vec()));
        assert_eq!(state(fields[2].widgets[1].obj), Some(b"Off".to_vec()));
        // The text field has a new appearance with the embedded font.
        let w = f.get(fields[0].widgets[0].obj).unwrap();
        let ap = f.resolve(w.as_dict().unwrap().get(b"AP").unwrap()).unwrap();
        let n = f.resolve(ap.as_dict().unwrap().get(b"N").unwrap()).unwrap();
        let Obj::Stream(d, data) = n else { panic!("no appearance stream") };
        assert!(String::from_utf8_lossy(&data).contains("/Tx BMC"));
        assert!(d.get(b"Resources").unwrap().as_dict().unwrap().get(b"Font").is_some());
        // The list knows which options are chosen.
        let list = f.get(fields[4].obj).unwrap();
        assert_eq!(list.as_dict().unwrap().get(b"I"), Some(&Obj::Array(vec![Obj::Int(1)])));

        // Undo goes back one field at a time; the file as it was at the start.
        for _ in 0..5 {
            s.undo().unwrap();
        }
        assert!(s.tail().is_none());
        s.redo().unwrap();
        let (_, fields) = reread(&s);
        assert!(matches!(&fields[0].value, FieldValue::Text(t) if t.starts_with("علي")));

        // Saved and read back; survives a crash before that.
        let json = serde_json::to_string(&s.recovery().unwrap()).unwrap();
        let path = s.path().to_path_buf();
        let mut s2 = Session::open(&path).unwrap();
        s2.restore(serde_json::from_str(&json).unwrap()).unwrap();
        assert!(s2.tail() == s.tail(), "the same update after restoring");
        s.save(&path).unwrap();
        let mut s3 = Session::open(&path).unwrap();
        let f3 = s3.fields().unwrap();
        assert!(matches!(&f3[0].field.value, FieldValue::Text(t) if t.starts_with("علي")));
    }

    #[test]
    fn flattening_draws_fields_into_the_page() {
        let mut s = session();
        let ids: Vec<Ref> = s.fields().unwrap().iter().map(|f| f.field.obj).collect();
        s.set_field(ids[0], FieldValue::Text("Ali".into())).unwrap();
        s.set_field(ids[1], FieldValue::State(Some("Yes".into()))).unwrap();
        s.flatten().unwrap();
        assert!(s.take_layout_changed());
        assert!(s.fields().unwrap().is_empty());
        let (f, fields) = reread(&s);
        assert!(fields.is_empty(), "no form left");
        let page = f.pages().unwrap()[0];
        let d = crate::session::pages::page_dict(&f, page).unwrap();
        assert!(d.get(b"Annots").is_none());
        let contents = d.get(b"Contents").and_then(Obj::as_array).unwrap().to_vec();
        let Obj::Stream(_, last) = f.resolve(contents.last().unwrap()).unwrap() else { panic!() };
        let last = String::from_utf8(last).unwrap();
        // The text field, the check box, the radio button shown on, the combo box and the list.
        assert!(last.matches(" Do Q").count() >= 4, "{last}");
        let xo = d.get(b"Resources").unwrap().as_dict().unwrap().get(b"XObject").unwrap().as_dict().unwrap().clone();
        assert!(xo.get(b"EditsFl0").is_some());
        // Undone, the form is back.
        s.undo().unwrap();
        assert_eq!(s.fields().unwrap().len(), 5);
        let (_, fields) = reread(&s);
        assert_eq!(fields.len(), 5);
    }
}
