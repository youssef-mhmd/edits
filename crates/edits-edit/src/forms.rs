//! Interactive form fields (PRD 5.4), as the file's /AcroForm describes
//! them: text fields, check boxes, radio buttons, combo boxes and lists.
//!
//! A field is a node of the field tree with a value; its widgets are the
//! boxes on pages (often the same object as the field). Some attributes are
//! inherited from parent fields.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::pdf::object::decode_text;
use crate::pdf::{Dict, Obj, PdfError, PdfFile, Ref, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Text,
    Check,
    Radio,
    Combo,
    List,
    /// Push buttons and signature fields: shown, not filled.
    Other,
}

/// A field's value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum FieldValue {
    Text(String),
    /// Check boxes and radio buttons: the "on" state chosen, or None (off).
    State(Option<String>),
    /// Combo boxes and lists: the chosen export values.
    Choice(Vec<String>),
}

/// One box of a field on a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Widget {
    pub obj: Ref,
    /// Page index in the file the field was read from.
    pub page: u32,
    /// Left, bottom, right, top in user space.
    pub rect: [f32; 4],
    /// Check boxes and radio buttons: the name of this box's "on" state.
    pub on: Option<String>,
    /// Quarter turns of the widget's content (/MK /R).
    pub turn: u8,
    /// A check box or radio button without "on" and "off" appearances of
    /// its own: edits draws them.
    #[serde(default)]
    pub draw_states: bool,
}

/// A field as edits reads it from the file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// The terminal field's object (it holds the value).
    pub obj: Ref,
    /// Fully qualified name (parent names joined with dots).
    pub name: String,
    /// Tooltip (/TU), shown to the reader.
    pub tooltip: String,
    pub kind: FieldKind,
    pub read_only: bool,
    pub required: bool,
    pub multiline: bool,
    pub password: bool,
    /// Text fields: one character per cell (/MaxLen cells).
    pub comb: bool,
    /// Combo boxes the reader may type into.
    pub editable: bool,
    pub multi_select: bool,
    /// Radio buttons: clicking the chosen one does not turn it off.
    pub no_toggle_off: bool,
    pub max_len: Option<u32>,
    /// Choices: (export value, text shown).
    pub options: Vec<(String, String)>,
    pub value: FieldValue,
    /// Default appearance: font size (0 = fit) and colour.
    pub font_size: f32,
    pub color: [f32; 3],
    /// Alignment: 0 left, 1 centre, 2 right.
    pub align: u8,
    pub widgets: Vec<Widget>,
}

const READ_ONLY: i64 = 1;
const REQUIRED: i64 = 1 << 1;
const MULTILINE: i64 = 1 << 12;
const PASSWORD: i64 = 1 << 13;
const NO_TOGGLE_OFF: i64 = 1 << 14;
const RADIO: i64 = 1 << 15;
const PUSH: i64 = 1 << 16;
const COMBO: i64 = 1 << 17;
const EDIT: i64 = 1 << 18;
const MULTI_SELECT: i64 = 1 << 21;
const COMB: i64 = 1 << 24;

/// Attributes a field takes from its parents.
#[derive(Clone, Default)]
struct Inherited {
    ft: Option<Vec<u8>>,
    ff: Option<i64>,
    v: Option<Obj>,
    da: Option<Vec<u8>>,
    q: Option<i64>,
    opt: Option<Obj>,
    max_len: Option<i64>,
    name: String,
}

/// Every field of the document, in the order of the field tree.
pub fn read_fields<S: Source>(f: &PdfFile<S>) -> Result<Vec<Field>, PdfError> {
    let catalog = f.resolve(f.trailer.get(b"Root").unwrap_or(&Obj::Null))?;
    let Some(form) = catalog.as_dict().and_then(|c| c.get(b"AcroForm")) else { return Ok(Vec::new()) };
    let form = f.resolve(form)?;
    let Some(form) = form.as_dict() else { return Ok(Vec::new()) };
    let roots: Vec<Ref> = match form.get(b"Fields").map(|o| f.resolve(o)).transpose()? {
        Some(Obj::Array(a)) => a.iter().filter_map(Obj::as_ref).collect(),
        _ => Vec::new(),
    };
    let base = Inherited {
        da: form.get(b"DA").and_then(|d| str_bytes(f, d)),
        q: form.get(b"Q").and_then(Obj::as_int),
        ..Default::default()
    };
    // Which page each widget is on, from the pages' /Annots.
    let mut page_of: HashMap<u32, u32> = HashMap::new();
    for (i, _) in f.pages()?.iter().enumerate() {
        if let Ok((_, _, annots)) = f.page_annots(i as u32) {
            for a in annots.iter().filter_map(Obj::as_ref) {
                page_of.entry(a.0).or_insert(i as u32);
            }
        }
    }
    let pages: HashMap<u32, u32> = f.pages()?.iter().enumerate().map(|(i, r)| (r.0, i as u32)).collect();
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for r in roots {
        walk(f, r, &base, &page_of, &pages, &mut seen, &mut out, 0)?;
    }
    Ok(out)
}

fn str_bytes<S: Source>(f: &PdfFile<S>, o: &Obj) -> Option<Vec<u8>> {
    match f.resolve(o).ok()? {
        Obj::Str(s) => Some(s),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn walk<S: Source>(
    f: &PdfFile<S>,
    r: Ref,
    parent: &Inherited,
    page_of: &HashMap<u32, u32>,
    pages: &HashMap<u32, u32>,
    seen: &mut HashSet<u32>,
    out: &mut Vec<Field>,
    depth: usize,
) -> Result<(), PdfError> {
    if depth > 32 || !seen.insert(r.0) || seen.len() > 100_000 {
        return Ok(());
    }
    let o = f.get(r)?;
    let Some(d) = o.as_dict() else { return Ok(()) };
    let mut inh = parent.clone();
    if let Some(Obj::Name(n)) = d.get(b"FT") {
        inh.ft = Some(n.clone());
    }
    if let Some(v) = d.get(b"Ff").and_then(|o| f.resolve(o).ok()).and_then(|o| o.as_int()) {
        inh.ff = Some(v);
    }
    if let Some(v) = d.get(b"V") {
        inh.v = Some(f.resolve(v)?);
    }
    if let Some(v) = d.get(b"DA").and_then(|o| str_bytes(f, o)) {
        inh.da = Some(v);
    }
    if let Some(v) = d.get(b"Q").and_then(Obj::as_int) {
        inh.q = Some(v);
    }
    if let Some(v) = d.get(b"Opt") {
        inh.opt = Some(f.resolve(v)?);
    }
    if let Some(v) = d.get(b"MaxLen").and_then(|o| f.resolve(o).ok()).and_then(|o| o.as_int()) {
        inh.max_len = Some(v);
    }
    if let Some(t) = d.get(b"T").and_then(|o| str_bytes(f, o)) {
        let t = decode_text(&t);
        inh.name = if inh.name.is_empty() { t } else { format!("{}.{t}", inh.name) };
    }
    // Kids that are fields (have /T) make this a non-terminal field; kids
    // without /T are its widgets.
    let kids: Vec<Ref> = match d.get(b"Kids").map(|k| f.resolve(k)).transpose()? {
        Some(Obj::Array(a)) => a.iter().filter_map(Obj::as_ref).collect(),
        _ => Vec::new(),
    };
    let mut widget_refs = Vec::new();
    let mut child_fields = Vec::new();
    for k in &kids {
        let kd = f.get(*k)?;
        match kd.as_dict() {
            Some(kd) if kd.get(b"T").is_some() || kd.get(b"Kids").is_some() => child_fields.push(*k),
            Some(_) => widget_refs.push(*k),
            None => {}
        }
    }
    for k in child_fields {
        walk(f, k, &inh, page_of, pages, seen, out, depth + 1)?;
    }
    if kids.is_empty() {
        // A field that is its own widget.
        widget_refs.push(r);
    } else if widget_refs.is_empty() {
        return Ok(());
    }
    let ff = inh.ff.unwrap_or(0);
    let kind = match inh.ft.as_deref() {
        Some(b"Tx") => FieldKind::Text,
        Some(b"Btn") if ff & PUSH != 0 => FieldKind::Other,
        Some(b"Btn") if ff & RADIO != 0 => FieldKind::Radio,
        Some(b"Btn") => FieldKind::Check,
        Some(b"Ch") if ff & COMBO != 0 => FieldKind::Combo,
        Some(b"Ch") => FieldKind::List,
        _ => FieldKind::Other,
    };
    let mut widgets = Vec::new();
    for w in widget_refs {
        let wo = f.get(w)?;
        let Some(wd) = wo.as_dict() else { continue };
        let rect = match wd.get(b"Rect").and_then(|o| f.resolve(o).ok()).and_then(|o| o.numbers()) {
            Some(r) if r.len() == 4 => [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])],
            _ => continue,
        };
        let page = match wd.get(b"P").and_then(Obj::as_ref).and_then(|p| pages.get(&p.0)) {
            Some(p) => *p,
            None => match page_of.get(&w.0) {
                Some(p) => *p,
                None => continue,
            },
        };
        let button = matches!(kind, FieldKind::Check | FieldKind::Radio);
        let found = if button { on_state(f, wd) } else { None };
        let draw_states = button && found.is_none();
        // Without appearances, a box is "Yes" when on (radio buttons: one name each).
        let on = match (found, kind) {
            (Some(s), _) => Some(s),
            (None, FieldKind::Check) => Some("Yes".into()),
            (None, FieldKind::Radio) => Some(format!("Choice{}", widgets.len() + 1)),
            _ => None,
        };
        let mk = wd.get(b"MK").map(|m| f.resolve(m)).transpose()?;
        let turn = mk
            .as_ref()
            .and_then(|m| m.as_dict())
            .and_then(|m| m.get(b"R"))
            .and_then(Obj::as_int)
            .map_or(0, |r| (r.rem_euclid(360) / 90) as u8);
        widgets.push(Widget { obj: w, page, rect, on, turn, draw_states });
    }
    if widgets.is_empty() {
        return Ok(());
    }
    let text_of = |o: &Obj| match o {
        Obj::Str(s) => Some(decode_text(s)),
        Obj::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    };
    let options: Vec<(String, String)> = match &inh.opt {
        Some(Obj::Array(a)) => a
            .iter()
            .filter_map(|o| match f.resolve(o).ok()? {
                Obj::Array(pair) if pair.len() >= 2 => Some((text_of(&pair[0])?, text_of(&pair[1])?)),
                o => text_of(&o).map(|t| (t.clone(), t)),
            })
            .collect(),
        _ => Vec::new(),
    };
    let value = match kind {
        FieldKind::Check | FieldKind::Radio => {
            let v = match &inh.v {
                Some(Obj::Name(n)) if n != b"Off" => Some(String::from_utf8_lossy(n).into_owned()),
                _ => None,
            };
            // Without /V, a box shown on (/AS) counts.
            let v = v.or_else(|| {
                widgets.iter().find_map(|w| {
                    let wd = f.get(w.obj).ok()?;
                    match wd.as_dict()?.get(b"AS") {
                        Some(Obj::Name(n)) if n != b"Off" => Some(String::from_utf8_lossy(n).into_owned()),
                        _ => None,
                    }
                })
            });
            FieldValue::State(v)
        }
        FieldKind::Combo | FieldKind::List => FieldValue::Choice(match &inh.v {
            Some(Obj::Array(a)) => a.iter().filter_map(text_of).collect(),
            Some(o) => text_of(o).into_iter().collect(),
            None => Vec::new(),
        }),
        _ => FieldValue::Text(inh.v.as_ref().and_then(text_of).unwrap_or_default()),
    };
    let (font_size, color) = parse_da(inh.da.as_deref().unwrap_or(b""));
    out.push(Field {
        obj: r,
        name: inh.name.clone(),
        tooltip: d.get(b"TU").and_then(|o| str_bytes(f, o)).map(|t| decode_text(&t)).unwrap_or_default(),
        kind,
        read_only: ff & READ_ONLY != 0,
        required: ff & REQUIRED != 0,
        multiline: ff & MULTILINE != 0,
        password: ff & PASSWORD != 0,
        comb: ff & COMB != 0 && inh.max_len.is_some(),
        editable: ff & EDIT != 0,
        multi_select: ff & MULTI_SELECT != 0,
        no_toggle_off: ff & NO_TOGGLE_OFF != 0,
        max_len: inh.max_len.filter(|m| *m > 0).map(|m| m.min(100_000) as u32),
        options,
        value,
        font_size,
        color,
        align: inh.q.unwrap_or(0).clamp(0, 2) as u8,
        widgets,
    });
    Ok(())
}

/// The name of a button widget's "on" appearance (any state but Off).
fn on_state<S: Source>(f: &PdfFile<S>, wd: &Dict) -> Option<String> {
    let ap = f.resolve(wd.get(b"AP")?).ok()?;
    for key in [&b"N"[..], b"D"] {
        let Some(states) = ap.as_dict()?.get(key).and_then(|n| f.resolve(n).ok()) else { continue };
        if let Some(d) = states.as_dict()
            && !matches!(states, Obj::Stream(..))
            && let Some((k, _)) = d.0.iter().find(|(k, _)| k != b"Off")
        {
            return Some(String::from_utf8_lossy(k).into_owned());
        }
    }
    None
}

/// Font size and colour from a default appearance string such as
/// "/Helv 0 Tf 0 g" or "/F1 9 Tf 0 0 1 rg".
pub fn parse_da(da: &[u8]) -> (f32, [f32; 3]) {
    let text = String::from_utf8_lossy(da);
    let toks: Vec<&str> = text.split_whitespace().collect();
    let num = |i: usize| toks.get(i).and_then(|t| t.parse::<f32>().ok());
    let mut size = 0.0;
    let mut color = [0.0; 3];
    for (i, t) in toks.iter().enumerate() {
        match *t {
            "Tf" if i >= 1 => size = num(i - 1).unwrap_or(0.0),
            "g" if i >= 1 => color = [num(i - 1).unwrap_or(0.0); 3],
            "rg" if i >= 3 => color = [num(i - 3).unwrap_or(0.0), num(i - 2).unwrap_or(0.0), num(i - 1).unwrap_or(0.0)],
            "k" if i >= 4 => {
                let (c, m, y, k) = (
                    num(i - 4).unwrap_or(0.0),
                    num(i - 3).unwrap_or(0.0),
                    num(i - 2).unwrap_or(0.0),
                    num(i - 1).unwrap_or(0.0),
                );
                color = [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)];
            }
            _ => {}
        }
    }
    (size.clamp(0.0, 200.0), color.map(|c| c.clamp(0.0, 1.0)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A one-page form with a text field, a check box, a radio group of
    /// two, a combo box with export values, and a list (fields nested under
    /// a parent named "person").
    pub fn form_pdf() -> Vec<u8> {
        let objs: Vec<(u32, String)> = vec![
            (1, "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [4 0 R 6 0 R 7 0 R 10 0 R 11 0 R] /DA (/Helv 0 Tf 0 g) /DR << >> >> >>".into()),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into()),
            (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 400] /Annots [5 0 R 6 0 R 8 0 R 9 0 R 10 0 R 11 0 R] >>".into()),
            (4, "<< /T (person) /Kids [5 0 R] >>".into()),
            (5, "<< /Type /Annot /Subtype /Widget /Parent 4 0 R /T (name) /FT /Tx /Rect [10 350 200 370] /P 3 0 R /DA (/Helv 11 Tf 0 0 1 rg) /Q 2 /MaxLen 40 /V (Ali) /TU (Full name) >>".into()),
            (6, "<< /Type /Annot /Subtype /Widget /T (agree) /FT /Btn /Rect [10 320 24 334] /P 3 0 R /AP << /N << /Yes 12 0 R /Off 12 0 R >> >> /AS /Off >>".into()),
            (7, "<< /T (size) /FT /Btn /Ff 49152 /V /L /Kids [8 0 R 9 0 R] >>".into()),
            (8, "<< /Type /Annot /Subtype /Widget /Parent 7 0 R /Rect [10 290 24 304] /AP << /N << /S 12 0 R /Off 12 0 R >> >> /AS /Off >>".into()),
            (9, "<< /Type /Annot /Subtype /Widget /Parent 7 0 R /Rect [40 290 54 304] /AP << /N << /L 12 0 R /Off 12 0 R >> >> /AS /L >>".into()),
            (10, "<< /Type /Annot /Subtype /Widget /T (city) /FT /Ch /Ff 131072 /Opt [[(CAI) (Cairo)] [(ALX) (Alexandria)]] /V (ALX) /Rect [10 250 200 270] /P 3 0 R >>".into()),
            (11, "<< /Type /Annot /Subtype /Widget /T (langs) /FT /Ch /Ff 2097152 /Opt [(Arabic) (English) (French)] /V [(Arabic) (French)] /Rect [10 150 200 240] /P 3 0 R >>".into()),
            (12, "<< /Type /XObject /Subtype /Form /BBox [0 0 14 14] /Length 0 >>\nstream\n\nendstream".into()),
        ];
        let mut b = b"%PDF-1.7\n".to_vec();
        let mut offs = Vec::new();
        for (num, o) in &objs {
            offs.push(b.len());
            b.extend_from_slice(format!("{num} 0 obj\n{o}\nendobj\n").as_bytes());
        }
        let x = b.len();
        b.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", objs.len() + 1).as_bytes());
        for o in offs {
            b.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
        }
        b.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF", objs.len() + 1).as_bytes(),
        );
        b
    }

    #[test]
    fn fields_are_read_with_what_they_inherit() {
        let f = PdfFile::open(form_pdf()).unwrap();
        let fields = read_fields(&f).unwrap();
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["person.name", "agree", "size", "city", "langs"]);
        let name = &fields[0];
        assert_eq!(name.kind, FieldKind::Text);
        assert_eq!(name.value, FieldValue::Text("Ali".into()));
        assert_eq!((name.font_size, name.color, name.align, name.max_len), (11.0, [0.0, 0.0, 1.0], 2, Some(40)));
        assert_eq!(name.tooltip, "Full name");
        assert_eq!(fields[1].kind, FieldKind::Check);
        assert_eq!(fields[1].widgets[0].on.as_deref(), Some("Yes"));
        assert_eq!(fields[1].value, FieldValue::State(None));
        let size = &fields[2];
        assert_eq!(size.kind, FieldKind::Radio);
        assert!(size.no_toggle_off);
        assert_eq!(size.widgets.iter().map(|w| w.on.clone().unwrap()).collect::<Vec<_>>(), ["S", "L"]);
        assert_eq!(size.widgets[0].page, 0, "found through the page's annotations");
        assert_eq!(size.value, FieldValue::State(Some("L".into())));
        assert_eq!(fields[3].kind, FieldKind::Combo);
        assert_eq!(fields[3].options[1], ("ALX".into(), "Alexandria".into()));
        assert_eq!(fields[3].value, FieldValue::Choice(vec!["ALX".into()]));
        assert!(fields[4].multi_select);
        assert_eq!(fields[4].value, FieldValue::Choice(vec!["Arabic".into(), "French".into()]));
    }

    #[test]
    fn default_appearances_parse() {
        assert_eq!(parse_da(b"/Helv 0 Tf 0 g"), (0.0, [0.0; 3]));
        assert_eq!(parse_da(b"0 0 1 rg /F1 9 Tf"), (9.0, [0.0, 0.0, 1.0]));
        assert_eq!(parse_da(b"/Cour 12 Tf 0 0 0 1 k").1, [0.0; 3]);
    }
}
