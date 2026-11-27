//! Incremental save (PRD 8.4): the original bytes stay as they are and the
//! changed objects are appended after them, with a new cross-reference
//! section. Fast whatever the file size, and other readers see exactly the
//! old file plus the annotations.
//!
//! The result is written to a separate file; the caller replaces the
//! original with it, so a crash can never leave a half-written document.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use lopdf::{Dictionary, Document, IncrementalDocument, Object, ObjectId, Stream, StringFormat};

use crate::model::{Annot, AnnotId, ObjRef};

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The file is encrypted; appending to it would need its encryption.
    /// Save with a full rewrite instead.
    #[error("the file is encrypted")]
    Encrypted,
    #[error("the file could not be read for saving: {0}")]
    Read(String),
    #[error("page {0} is not in the file")]
    NoPage(u32),
    #[error("could not write: {0}")]
    Write(#[from] std::io::Error),
    #[error("could not build the update: {0}")]
    Pdf(String),
}

impl From<lopdf::Error> for SaveError {
    fn from(e: lopdf::Error) -> Self {
        SaveError::Pdf(e.to_string())
    }
}

/// What changes on one page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageChange {
    pub page: u32,
    /// Annotation objects to take off the page.
    pub remove: Vec<ObjRef>,
    /// Annotations to write, in order.
    pub add: Vec<(AnnotId, Annot)>,
}

impl PageChange {
    pub fn new(page: u32) -> Self {
        Self { page, remove: Vec::new(), add: Vec::new() }
    }
}

/// What a save wrote.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SaveOutcome {
    /// The object each new annotation got.
    pub objs: HashMap<AnnotId, ObjRef>,
    /// For each changed page, its annotation list as saved (None for an
    /// annotation written inline rather than as its own object).
    pub pages: HashMap<u32, Vec<Option<ObjRef>>>,
}

/// Reads `src`, applies `changes`, and writes the original plus an
/// incremental update to `dest`.
pub fn save_incremental(src: &Path, dest: &Path, changes: &[PageChange]) -> Result<SaveOutcome, SaveError> {
    let bytes = std::fs::read(src).map_err(|e| SaveError::Read(e.to_string()))?;
    let doc = Document::load_mem(&bytes).map_err(|e| {
        if bytes.windows(8).any(|w| w == b"/Encrypt") { SaveError::Encrypted } else { SaveError::Read(e.to_string()) }
    })?;
    if doc.is_encrypted() || doc.trailer.get(b"Encrypt").is_ok() {
        return Err(SaveError::Encrypted);
    }
    let pages = doc.get_pages();
    let mut inc = IncrementalDocument::create_from(bytes, doc);
    let mut outcome = SaveOutcome::default();

    for change in changes {
        let page_id = *pages.get(&(change.page + 1)).ok_or(SaveError::NoPage(change.page))?;
        let existing = annots_of(inc.get_prev_documents(), page_id)?;
        let mut order: Vec<Object> = existing
            .into_iter()
            .filter(|o| !matches!(o, Object::Reference(id) if change.remove.contains(&(id.0, id.1))))
            .collect();
        for (id, a) in &change.add {
            let obj = write_annot(&mut inc.new_document, a, page_id);
            outcome.objs.insert(*id, (obj.0, obj.1));
            order.push(Object::Reference(obj));
        }
        outcome.pages.insert(
            change.page,
            order.iter().map(|o| if let Object::Reference(id) = o { Some((id.0, id.1)) } else { None }).collect(),
        );
        inc.opt_clone_object_to_new_document(page_id)?;
        let page = inc.new_document.get_object_mut(page_id)?.as_dict_mut()?;
        page.set("Annots", Object::Array(order));
    }

    let mut out = std::io::BufWriter::new(std::fs::File::create(dest)?);
    inc.save_to(&mut out)?;
    let file = out.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    Ok(outcome)
}

/// The page's /Annots entries (references or inline dictionaries).
fn annots_of(doc: &Document, page_id: ObjectId) -> Result<Vec<Object>, SaveError> {
    let page = doc.get_object(page_id)?.as_dict()?;
    let Ok(annots) = page.get(b"Annots") else { return Ok(Vec::new()) };
    let (_, annots) = doc.dereference(annots)?;
    Ok(annots.as_array().cloned().unwrap_or_default())
}

/// A PDF text string: plain bytes for ASCII, UTF-16BE with a byte order mark otherwise.
fn text(s: &str) -> Object {
    if s.is_ascii() {
        Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
    } else {
        let mut b = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            b.extend_from_slice(&u.to_be_bytes());
        }
        Object::String(b, StringFormat::Hexadecimal)
    }
}

fn reals(v: &[f32]) -> Object {
    Object::Array(v.iter().map(|x| Object::Real(if x.is_finite() { *x } else { 0.0 })).collect())
}

/// Adds the annotation and its appearance stream to `doc`; returns its object id.
fn write_annot(doc: &mut Document, a: &Annot, page: ObjectId) -> ObjectId {
    let mut gs = Dictionary::new();
    gs.set("Type", Object::Name(b"ExtGState".to_vec()));
    for (k, v) in crate::ap::graphics_state(a) {
        gs.set(
            k,
            match v {
                crate::ap::GsValue::Num(x) => Object::Real(x),
                crate::ap::GsValue::Name(n) => Object::Name(n.as_bytes().to_vec()),
            },
        );
    }
    let mut ext = Dictionary::new();
    ext.set("GS0", Object::Dictionary(gs));
    let mut resources = Dictionary::new();
    resources.set("ExtGState", Object::Dictionary(ext));

    let mut form = Dictionary::new();
    form.set("Type", Object::Name(b"XObject".to_vec()));
    form.set("Subtype", Object::Name(b"Form".to_vec()));
    form.set("BBox", reals(&a.rect));
    form.set("Resources", Object::Dictionary(resources));
    let ap_id = doc.add_object(Stream::new(form, crate::ap::content(a).into_bytes()));
    let mut ap = Dictionary::new();
    ap.set("N", Object::Reference(ap_id));

    let mut d = Dictionary::new();
    d.set("Type", Object::Name(b"Annot".to_vec()));
    d.set("Subtype", Object::Name(a.kind.subtype().as_bytes().to_vec()));
    d.set("Rect", reals(&a.rect));
    d.set("QuadPoints", reals(&a.quads.concat()));
    d.set("C", reals(&a.color));
    d.set("CA", Object::Real(a.opacity));
    // Print (bit 3), as other readers do for markup.
    d.set("F", Object::Integer(4));
    d.set("P", Object::Reference(page));
    d.set("AP", Object::Dictionary(ap));
    if !a.contents.is_empty() {
        d.set("Contents", text(&a.contents));
    }
    if !a.author.is_empty() {
        d.set("T", text(&a.author));
    }
    if !a.modified.is_empty() {
        d.set("M", Object::String(a.modified.as_bytes().to_vec(), StringFormat::Literal));
        d.set("CreationDate", Object::String(a.modified.as_bytes().to_vec(), StringFormat::Literal));
    }
    doc.add_object(d)
}

/// Where a save of `path` is written before it replaces `path`, so a crash
/// leaves either the old file or the new one, never half of one.
pub fn temp_path_for(path: &Path) -> std::path::PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "document.pdf".into());
    path.with_file_name(format!(".{name}.edits-saving"))
}

/// Replaces `dest` with `tmp`.
pub fn replace(tmp: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::rename(tmp, dest)
}

/// Writes `data` to `path` through a temporary file.
pub fn write_safely(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = temp_path_for(path);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    replace(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}
