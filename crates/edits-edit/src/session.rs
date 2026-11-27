//! A document being edited (PRD 8.4): its file as last saved (the base),
//! the annotations edits knows about, and the undo history.
//!
//! Every change is a command. After each one the whole incremental update
//! (the bytes that saving will append to the base) is rebuilt; PDFium reads
//! base plus update, so what is on screen is exactly what will be saved.
//! Saving copies the base beside the target, appends the update, and
//! renames the copy over the target.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::model::{Annot, AnnotId};
use crate::pdf::object::encode_text;
use crate::pdf::{Dict, FileSource, Obj, PdfError, PdfFile, Ref, write};

mod decor;
mod fields;
mod flatten;
mod pages;
pub use decor::Decor;
pub use fields::{FieldView, WidgetView};
pub use pages::{PageSlot, PageSrc};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Entry {
    /// Every version this annotation has had; `cur` is the one in effect.
    versions: Vec<Annot>,
    cur: usize,
    alive: bool,
    /// Its object in the base file and the version the base holds.
    base: Option<(Ref, usize)>,
    /// For a copy of an annotation from the file (a duplicated page): the
    /// original, whose dictionary the copy starts from.
    #[serde(default)]
    template: Option<Ref>,
}

impl Entry {
    fn annot(&self) -> &Annot {
        &self.versions[self.cur]
    }

    /// Differs from what the base file holds.
    fn changed(&self) -> bool {
        match (self.alive, self.base) {
            (true, None) | (false, Some(_)) => true,
            (true, Some((_, v))) => v != self.cur,
            (false, None) => false,
        }
    }
}

/// What an update writes, and the object each new annotation gets.
type Planned = (Vec<(Ref, Obj)>, HashMap<AnnotId, Ref>);

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
enum Command {
    Add(AnnotId),
    Remove(AnnotId),
    Change {
        id: AnnotId,
        from: usize,
        to: usize,
    },
    /// A form field's value changed (indexes into its versions).
    Field {
        obj: Ref,
        from: usize,
        to: usize,
    },
    /// Annotations and fields drawn into the pages (or no longer).
    Flatten {
        on: bool,
    },
    /// The page list changed (indexes into `Session::orders`).
    Pages {
        from: usize,
        to: usize,
    },
}

/// An annotation as the interface sees it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Listed {
    pub id: AnnotId,
    #[serde(flatten)]
    pub annot: Annot,
}

/// Undo and redo state, for the interface.
#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
pub struct Status {
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error(transparent)]
    Pdf(#[from] PdfError),
    #[error("could not write: {0}")]
    Io(#[from] std::io::Error),
    #[error("no annotation {0}")]
    Unknown(AnnotId),
    #[error("{0}")]
    Pages(String),
}

/// Unsaved work, written after every change so that it survives a crash
/// (PRD 8.4). Only valid for the file exactly as it was: `stamp`.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Recovery {
    pub path: PathBuf,
    /// File size and modification time (Unix ms) of the base.
    pub stamp: (u64, u128),
    /// When this was written (Unix ms).
    pub written: u128,
    entries: BTreeMap<AnnotId, Entry>,
    imported: HashSet<u32>,
    undo: Vec<(u64, Command)>,
    redo: Vec<(u64, Command)>,
    next_id: AnnotId,
    next_seq: u64,
    saved_at: u64,
    #[serde(default)]
    orders: Vec<Vec<PageSlot>>,
    #[serde(default)]
    order_at: usize,
    #[serde(default)]
    next_key: u32,
    #[serde(default)]
    images: Vec<Arc<crate::image::PdfImage>>,
    /// Other files pages were taken from, and the pages copied, in order.
    #[serde(default)]
    foreign: Vec<(PathBuf, Vec<u32>)>,
    #[serde(default)]
    decors: Vec<Decor>,
    #[serde(default)]
    field_values: Vec<(Ref, fields::FieldEntry)>,
    #[serde(default)]
    flattened: bool,
}

fn stamp_of(path: &Path) -> Option<(u64, u128)> {
    let m = std::fs::metadata(path).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    Some((m.len(), t))
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

pub struct Session {
    path: PathBuf,
    base: PdfFile<Arc<FileSource>>,
    entries: BTreeMap<AnnotId, Entry>,
    imported: HashSet<u32>,
    undo: Vec<(u64, Command)>,
    redo: Vec<(u64, Command)>,
    next_id: AnnotId,
    next_seq: u64,
    saved_at: u64,
    /// Objects new annotations got in the last update built.
    assigned: HashMap<AnnotId, Ref>,
    /// The update to append to the base; None when there is nothing to save.
    tail: Option<Arc<Vec<u8>>>,
    /// The base file's page count.
    base_pages: u32,
    /// The password the file was opened with (to read it again after saving).
    password: String,
    /// Every page list the document has had; `orders[order_at]` is now.
    orders: Vec<Vec<PageSlot>>,
    order_at: usize,
    /// Page key -> index now.
    pos: HashMap<u32, u32>,
    next_key: u32,
    /// Pictures of inserted picture pages.
    images: Vec<Arc<crate::image::PdfImage>>,
    foreign: Vec<pages::Foreign>,
    /// Headers, footers and watermarks added to pages.
    decors: Vec<Decor>,
    /// Form fields as read from the base file (read when first asked for).
    fields: Option<Vec<crate::forms::Field>>,
    /// Values given to fields, with their history.
    field_values: BTreeMap<Ref, fields::FieldEntry>,
    /// Annotations and form fields are drawn into the pages.
    flattened: bool,
    layout_changed: bool,
}

/// Opens the base file; a protected one with `password` (empty for files
/// that only restrict permissions).
fn open_base(path: &Path, password: &[u8]) -> Result<PdfFile<Arc<FileSource>>, PdfError> {
    PdfFile::open_with_password(Arc::new(FileSource::open(path)?), password)
}

impl Session {
    /// Opens `path` for editing. Fails for encrypted files and for files
    /// whose structure edits cannot read.
    pub fn open(path: &Path) -> Result<Self, PdfError> {
        Self::open_with_password(path, "")
    }

    /// Opens a password-protected file for editing.
    pub fn open_with_password(path: &Path, password: &str) -> Result<Self, PdfError> {
        let base = open_base(path, password.as_bytes())?;
        let n = base.pages()?.len() as u32;
        let mut s = Session {
            path: path.to_path_buf(),
            base,
            entries: BTreeMap::new(),
            imported: HashSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 0,
            next_seq: 0,
            saved_at: 0,
            assigned: HashMap::new(),
            tail: None,
            base_pages: n,
            password: password.to_string(),
            orders: vec![pages::identity(n)],
            order_at: 0,
            pos: HashMap::new(),
            next_key: n,
            images: Vec::new(),
            foreign: Vec::new(),
            decors: Vec::new(),
            fields: None,
            field_values: BTreeMap::new(),
            flattened: false,
            layout_changed: false,
        };
        s.reindex();
        Ok(s)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The document's page count now (at first the base file's, to check
    /// against the renderer's).
    pub fn page_count(&self) -> Result<u32, PdfError> {
        Ok(self.order().len() as u32)
    }

    /// The base file, shared for reading by the renderer.
    pub fn base_source(&self) -> Result<Arc<FileSource>, PdfError> {
        Ok(Arc::new(FileSource::open(&self.path)?))
    }

    /// The update to show on top of the base; None when there is none.
    pub fn tail(&self) -> Option<Arc<Vec<u8>>> {
        self.tail.clone()
    }

    fn at(&self) -> u64 {
        self.undo.last().map_or(0, |(s, _)| *s)
    }

    pub fn status(&self) -> Status {
        Status { dirty: self.at() != self.saved_at, can_undo: !self.undo.is_empty(), can_redo: !self.redo.is_empty() }
    }

    /// The unsaved work, for a recovery file; None when there is none.
    pub fn recovery(&self) -> Option<Recovery> {
        if !self.status().dirty {
            return None;
        }
        Some(Recovery {
            path: self.path.clone(),
            stamp: stamp_of(&self.path)?,
            written: now_ms(),
            entries: self.entries.clone(),
            imported: self.imported.clone(),
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            next_id: self.next_id,
            next_seq: self.next_seq,
            saved_at: self.saved_at,
            orders: self.orders.clone(),
            order_at: self.order_at,
            next_key: self.next_key,
            images: self.images.clone(),
            foreign: self.foreign.iter().map(|f| (f.path.clone(), f.copied.clone())).collect(),
            decors: self.decors.clone(),
            field_values: fields::to_list(&self.field_values),
            flattened: self.flattened,
        })
    }

    /// True if `r` was written for this file as it is now.
    pub fn can_restore(&self, r: &Recovery) -> bool {
        stamp_of(&self.path) == Some(r.stamp)
    }

    /// Takes back unsaved work from a recovery file. Returns the pages it touches.
    pub fn restore(&mut self, r: Recovery) -> Result<Vec<u32>, EditError> {
        if !self.can_restore(&r) {
            return Err(EditError::Io(std::io::Error::other("the file has changed since")));
        }
        self.entries = r.entries;
        self.imported = r.imported;
        self.undo = r.undo;
        self.redo = r.redo;
        self.next_id = r.next_id;
        self.next_seq = r.next_seq;
        self.saved_at = r.saved_at;
        self.field_values = fields::from_list(r.field_values);
        self.flattened = r.flattened;
        if self.flattened {
            self.layout_changed = true;
        }
        if !self.field_values.is_empty() {
            self.base_fields()?;
        }
        if !r.orders.is_empty() {
            // Pages from other files are copied again, in the same order, so
            // they get the same numbers.
            let mut foreign = Vec::new();
            for (path, copied) in &r.foreign {
                let mut f = pages::Foreign::open(path)?;
                for p in copied {
                    f.copy_page(*p)?;
                }
                foreign.push(f);
            }
            self.foreign = foreign;
            self.orders = r.orders;
            self.order_at = r.order_at.min(self.orders.len() - 1);
            self.next_key = r.next_key;
            self.images = r.images;
            self.decors = r.decors;
            self.reindex();
            self.layout_changed = true;
        }
        self.rebuild()?;
        let pages: BTreeSet<u32> =
            self.entries.values().flat_map(|e| e.versions.iter().filter_map(|a| self.index_of(a.page))).collect();
        Ok(pages.into_iter().collect())
    }

    /// Reads the annotations of the page at `page` from the base file, once.
    /// (Pages inserted from elsewhere have none to read.)
    pub fn import(&mut self, page: u32) -> Result<(), PdfError> {
        let Some(slot) = self.order().get(page as usize) else {
            return Err(PdfError::Damaged(format!("no page {page}")));
        };
        let key = slot.key;
        if slot.src != PageSrc::Base(key) || !self.imported.insert(key) {
            return Ok(());
        }
        let (_, _, annots) = self.base.page_annots(key)?;
        for o in annots {
            let Some(r) = o.as_ref() else { continue };
            let Ok(obj) = self.base.get(r) else { continue };
            let Some(a) = obj.as_dict().and_then(|d| Annot::from_dict(key, d)) else { continue };
            self.next_id += 1;
            self.entries.insert(
                self.next_id,
                Entry { versions: vec![a], cur: 0, alive: true, base: Some((r, 0)), template: None },
            );
        }
        Ok(())
    }

    /// `a` with its page given as the index it is at now.
    fn shown(&self, a: &Annot) -> Option<Annot> {
        let mut a = a.clone();
        a.page = self.index_of(a.page)?;
        Some(a)
    }

    /// Draws every annotation and form field into its page (undoable until saved).
    pub fn flatten(&mut self) -> Result<(), EditError> {
        if self.flattened {
            return Ok(());
        }
        self.flattened = true;
        self.layout_changed = true;
        self.push(Command::Flatten { on: true });
        if let Err(e) = self.rebuild() {
            self.flattened = false;
            self.undo.pop();
            return Err(e.into());
        }
        Ok(())
    }

    pub fn is_flattened(&self) -> bool {
        self.flattened
    }

    /// The annotations on the page at `page` now (import it first).
    pub fn list(&self, page: u32) -> Vec<Listed> {
        if self.flattened {
            return Vec::new();
        }
        let Some(key) = self.key_of(page) else { return Vec::new() };
        self.entries
            .iter()
            .filter(|(_, e)| e.alive && e.annot().page == key)
            .filter_map(|(id, e)| Some(Listed { id: *id, annot: self.shown(e.annot())? }))
            .collect()
    }

    /// An annotation, with its page as an index.
    pub fn get(&self, id: AnnotId) -> Option<Annot> {
        self.entries.get(&id).filter(|e| e.alive).and_then(|e| self.shown(e.annot()))
    }

    /// Page keys to indexes, for pages still in the document.
    fn indexes(&self, keys: &[u32]) -> Vec<u32> {
        pages_of(&keys.iter().filter_map(|k| self.index_of(*k)).collect::<Vec<_>>())
    }

    fn keyed(&self, mut a: Annot) -> Result<Annot, EditError> {
        a.page = self.key_of(a.page).ok_or(EditError::Pages(format!("there is no page {}", a.page + 1)))?;
        Ok(a)
    }

    fn push(&mut self, c: Command) {
        self.next_seq += 1;
        self.undo.push((self.next_seq, c));
        self.redo.clear();
    }

    /// Adds an annotation; returns its id and the page to draw again.
    pub fn add(&mut self, a: Annot) -> Result<(AnnotId, u32), EditError> {
        let page = a.page;
        let a = self.keyed(a)?;
        self.next_id += 1;
        let id = self.next_id;
        self.entries.insert(id, Entry { versions: vec![a], cur: 0, alive: true, base: None, template: None });
        self.push(Command::Add(id));
        self.rebuild()?;
        Ok((id, page))
    }

    /// Deletes an annotation; returns the page to draw again.
    pub fn remove(&mut self, id: AnnotId) -> Result<u32, EditError> {
        let e = self.entries.get_mut(&id).filter(|e| e.alive).ok_or(EditError::Unknown(id))?;
        e.alive = false;
        let key = e.annot().page;
        self.push(Command::Remove(id));
        self.rebuild()?;
        Ok(self.index_of(key).unwrap_or(0))
    }

    /// Replaces an annotation with a changed version; returns the pages to
    /// draw again (it may have moved between pages).
    pub fn change(&mut self, id: AnnotId, a: Annot) -> Result<Vec<u32>, EditError> {
        let a = self.keyed(a)?;
        let e = self.entries.get_mut(&id).filter(|e| e.alive).ok_or(EditError::Unknown(id))?;
        if *e.annot() == a {
            return Ok(Vec::new());
        }
        let keys = [e.annot().page, a.page];
        let from = e.cur;
        e.versions.push(a);
        e.cur = e.versions.len() - 1;
        let to = e.cur;
        self.push(Command::Change { id, from, to });
        self.rebuild()?;
        Ok(self.indexes(&keys))
    }

    /// Undoes the last command; returns the pages to draw again.
    pub fn undo(&mut self) -> Result<Vec<u32>, EditError> {
        let Some((seq, c)) = self.undo.pop() else { return Ok(Vec::new()) };
        self.redo.push((seq, c));
        let keys = self.apply(c, true);
        self.rebuild()?;
        Ok(self.indexes(&keys))
    }

    pub fn redo(&mut self) -> Result<Vec<u32>, EditError> {
        let Some((seq, c)) = self.redo.pop() else { return Ok(Vec::new()) };
        self.undo.push((seq, c));
        let keys = self.apply(c, false);
        self.rebuild()?;
        Ok(self.indexes(&keys))
    }

    /// Applies or reverts `c`; returns the page keys to draw again.
    fn apply(&mut self, c: Command, backwards: bool) -> Vec<u32> {
        match c {
            Command::Field { obj, from, to } => {
                let Some(e) = self.field_values.get_mut(&obj) else { return Vec::new() };
                e.cur = if backwards { from } else { to };
                // Keys of base pages are their indexes in the base file.
                self.fields
                    .iter()
                    .flatten()
                    .filter(|f| f.obj == obj)
                    .flat_map(|f| f.widgets.iter().map(|w| w.page))
                    .collect()
            }
            Command::Flatten { on } => {
                self.flattened = on != backwards;
                self.layout_changed = true;
                Vec::new()
            }
            Command::Pages { from, to } => {
                self.order_at = if backwards { from } else { to };
                self.reindex();
                self.layout_changed = true;
                Vec::new()
            }
            Command::Add(id) | Command::Remove(id) => {
                let adding = matches!(c, Command::Add(_)) != backwards;
                let Some(e) = self.entries.get_mut(&id) else { return Vec::new() };
                e.alive = adding;
                vec![e.annot().page]
            }
            Command::Change { id, from, to } => {
                let Some(e) = self.entries.get_mut(&id) else { return Vec::new() };
                let before = e.annot().page;
                e.cur = if backwards { from } else { to };
                pages_of(&[before, e.annot().page])
            }
        }
    }

    /// Rebuilds the update from the current state.
    fn rebuild(&mut self) -> Result<(), PdfError> {
        let (objects, assigned) = self.objects()?;
        self.assigned = assigned;
        self.tail = if objects.is_empty() { None } else { Some(Arc::new(write::update(&self.base, &objects))) };
        Ok(())
    }

    /// The objects an update must write, and the objects new annotations get.
    fn objects(&self) -> Result<Planned, PdfError> {
        self.objects_for(self.order(), false)
    }

    /// True if `r` is the pop-up window of an annotation that is leaving.
    fn removes_popup(&self, r: Ref, leaving: &HashSet<Ref>) -> bool {
        if leaving.is_empty() {
            return false;
        }
        self.base
            .get(r)
            .ok()
            .and_then(|o| o.as_dict().cloned())
            .filter(|d| d.name(b"Subtype") == Some(b"Popup"))
            .and_then(|d| d.get(b"Parent").and_then(Obj::as_ref))
            .is_some_and(|p| leaving.contains(&p))
    }

    /// Writes the document to `target` (its own file, or a new one for
    /// Save As) and makes that the base. Returns the bytes written.
    pub fn save(&mut self, target: &Path) -> Result<u64, EditError> {
        self.rebuild()?;
        if self.pages_changed() {
            // Deleted pages: the whole file is written anew so that what
            // they showed is really gone. Otherwise the update is appended.
            // A damaged file that cannot be written anew is appended to.
            let whole = if self.drops_base_pages() {
                self.write_whole(self.tail.clone().unwrap_or_default(), target).ok()
            } else {
                None
            };
            let len = match whole {
                Some(len) => len,
                None => self.append_to(target)?,
            };
            // Page keys, objects and history all refer to the old file:
            // start again from the saved one.
            *self = Session::open_with_password(target, &self.password)?;
            self.layout_changed = true;
            return Ok(len);
        }
        let len = self.append_to(target)?;
        self.base = open_base(target, self.password.as_bytes())?;
        self.path = target.to_path_buf();
        for (id, e) in &mut self.entries {
            if e.alive {
                let r = self.assigned.get(id).copied().or(e.base.map(|b| b.0));
                e.base = r.map(|r| (r, e.cur));
                e.template = None;
            } else {
                e.base = None;
            }
        }
        self.assigned.clear();
        for e in self.field_values.values_mut() {
            e.saved = e.cur;
        }
        self.tail = None;
        self.saved_at = self.at();
        Ok(len)
    }

    /// Writes base plus update to `target` through a temporary file.
    fn append_to(&self, target: &Path) -> Result<u64, EditError> {
        let tmp = temp_path_for(target);
        let result = (|| -> Result<u64, EditError> {
            std::fs::copy(&self.path, &tmp)?;
            let mut f = std::fs::OpenOptions::new().append(true).open(&tmp)?;
            if let Some(t) = &self.tail {
                f.write_all(t)?;
            }
            f.sync_all()?;
            let len = f.metadata()?.len();
            drop(f);
            std::fs::rename(&tmp, target)?;
            Ok(len)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
}

fn pages_of(p: &[u32]) -> Vec<u32> {
    let mut v = p.to_vec();
    v.sort_unstable();
    v.dedup();
    v
}

/// Where a save of `path` is written before it replaces `path`, so a crash
/// leaves either the old file or the new one, never half of one.
pub fn temp_path_for(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "document.pdf".into());
    path.with_file_name(format!(".{name}.edits-saving"))
}

fn reals(v: &[f32]) -> Obj {
    Obj::Array(v.iter().map(|x| Obj::Real(*x as f64)).collect())
}

fn deflate(d: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = z.write_all(d);
    z.finish().unwrap_or_default()
}

/// The objects of an embedded subset font; returns the Type0 font.
fn font_objects(f: &edits_text::PdfFont, alloc: &mut dyn FnMut(Obj) -> Ref) -> Ref {
    let ft = edits_text::font();
    let k = 1000.0 / ft.units_per_em;
    let mut file = Dict::default();
    file.set(b"Length1", Obj::Int(f.program.len() as i64));
    file.set(b"Filter", Obj::name("FlateDecode"));
    let file = alloc(Obj::Stream(file, deflate(&f.program)));
    let base = Obj::name(&f.base_font);
    let mut desc = Dict::default();
    desc.set(b"Type", Obj::name("FontDescriptor"));
    desc.set(b"FontName", base.clone());
    desc.set(b"Flags", Obj::Int(4));
    desc.set(b"FontBBox", reals(&ft.bbox.map(|v| v * k)));
    desc.set(b"ItalicAngle", Obj::Int(0));
    desc.set(b"Ascent", Obj::Real((ft.ascent * k) as f64));
    desc.set(b"Descent", Obj::Real((ft.descent * k) as f64));
    desc.set(b"CapHeight", Obj::Real((ft.cap_height * k) as f64));
    desc.set(b"StemV", Obj::Int(80));
    desc.set(b"FontFile2", Obj::Ref(file));
    let desc = alloc(Obj::Dict(desc));
    let mut info = Dict::default();
    info.set(b"Registry", Obj::Str(b"Adobe".to_vec()));
    info.set(b"Ordering", Obj::Str(b"Identity".to_vec()));
    info.set(b"Supplement", Obj::Int(0));
    let mut w = Vec::new();
    for (g, width) in &f.widths {
        w.push(Obj::Int(*g as i64));
        w.push(Obj::Array(vec![Obj::Real(*width as f64)]));
    }
    let mut cid = Dict::default();
    cid.set(b"Type", Obj::name("Font"));
    cid.set(b"Subtype", Obj::name("CIDFontType2"));
    cid.set(b"BaseFont", base.clone());
    cid.set(b"CIDSystemInfo", Obj::Dict(info));
    cid.set(b"FontDescriptor", Obj::Ref(desc));
    cid.set(b"W", Obj::Array(w));
    cid.set(b"DW", Obj::Int(1000));
    cid.set(b"CIDToGIDMap", Obj::name("Identity"));
    let cid = alloc(Obj::Dict(cid));
    let tu = alloc(Obj::Stream(Dict::default(), f.to_unicode.clone()));
    let mut t0 = Dict::default();
    t0.set(b"Type", Obj::name("Font"));
    t0.set(b"Subtype", Obj::name("Type0"));
    t0.set(b"BaseFont", base);
    t0.set(b"Encoding", Obj::name("Identity-H"));
    t0.set(b"DescendantFonts", Obj::Array(vec![Obj::Ref(cid)]));
    t0.set(b"ToUnicode", Obj::Ref(tu));
    alloc(Obj::Dict(t0))
}

/// An image XObject (and its soft mask); returns the image.
fn image_objects(img: &crate::image::PdfImage, alloc: &mut dyn FnMut(Obj) -> Ref) -> Ref {
    let mut d = Dict::default();
    d.set(b"Type", Obj::name("XObject"));
    d.set(b"Subtype", Obj::name("Image"));
    d.set(b"Width", Obj::Int(img.width as i64));
    d.set(b"Height", Obj::Int(img.height as i64));
    d.set(b"BitsPerComponent", Obj::Int(8));
    if let Some(alpha) = &img.alpha {
        let mut m = d.clone();
        m.set(b"ColorSpace", Obj::name("DeviceGray"));
        m.set(b"Filter", Obj::name("FlateDecode"));
        let mask = alloc(Obj::Stream(m, alpha.clone()));
        d.set(b"SMask", Obj::Ref(mask));
    }
    d.set(b"ColorSpace", Obj::name(&img.color_space));
    d.set(b"Filter", Obj::name(&img.filter));
    alloc(Obj::Stream(d, img.data.clone()))
}

/// The appearance stream of `a`, as edits draws it. Fonts and pictures it
/// uses are added through `alloc`, which returns their object references.
fn appearance(a: &Annot, alloc: &mut dyn FnMut(Obj) -> Ref) -> Obj {
    use crate::model::{Kind, TEXT_PAD};
    let mut gs = Dict::default();
    gs.set(b"Type", Obj::name("ExtGState"));
    for (k, v) in crate::ap::graphics_state(a) {
        gs.set(
            k.as_bytes(),
            match v {
                crate::ap::GsValue::Num(x) => Obj::Real(x as f64),
                crate::ap::GsValue::Name(n) => Obj::name(n),
            },
        );
    }
    let mut ext = Dict::default();
    ext.set(b"GS0", Obj::Dict(gs));
    let mut res = Dict::default();
    res.set(b"ExtGState", Obj::Dict(ext));
    let mut content = crate::ap::content(a);
    let [l, b, r, t] = a.rect;
    // Text: a text box's contents, or a stamp's word scaled to its frame.
    let text = match a.kind {
        Kind::FreeText if !a.contents.trim().is_empty() => {
            let lay = a.text_layout();
            Some((lay, l + TEXT_PAD, t - TEXT_PAD, r - l - 2.0 * TEXT_PAD))
        }
        Kind::Stamp if a.image.is_none() && !a.label.is_empty() => {
            // Fit the ink (not the font's line box) inside the frame, centred.
            let one = edits_text::layout(&a.label.replace('\n', " "), 10.0, 0.0);
            one.ink().map(|ink| {
                let pad = (t - b) * crate::model::STAMP_PAD;
                let (iw, ih) = ((ink[2] - ink[0]).max(0.1), (ink[3] - ink[1]).max(0.1));
                let k = ((r - l - 2.0 * pad) / iw).min((t - b - 2.0 * pad) / ih);
                let lay = edits_text::layout(&a.label.replace('\n', " "), (10.0 * k).max(0.5), 0.0);
                // Line start and baseline that put the scaled ink box in the middle.
                let x = (l + r) / 2.0 - (ink[0] + ink[2]) / 2.0 * k;
                let baseline = (b + t) / 2.0 - (ink[1] + ink[3]) / 2.0 * k;
                let top = baseline + lay.first_baseline();
                let w = lay.lines.first().map_or(0.0, |x| x.width);
                (lay, x, top, w)
            })
        }
        _ => None,
    };
    if let Some((lay, left, top, width)) = text {
        let pt = edits_text::pdf_text(&[(&lay, left, top, width, a.color)], "F0");
        let font = font_objects(&pt.font, alloc);
        let mut fonts = Dict::default();
        fonts.set(b"F0", Obj::Ref(font));
        res.set(b"Font", Obj::Dict(fonts));
        content += &format!(
            "q {} {} {} {} re W n\n{}Q\n",
            crate::pdf::object::real(l as f64),
            crate::pdf::object::real(b as f64),
            crate::pdf::object::real((r - l) as f64),
            crate::pdf::object::real((t - b) as f64),
            pt.ops
        );
    }
    if let Some(img) = a.image.as_deref().filter(|_| a.kind == Kind::Stamp) {
        let im = image_objects(img, alloc);
        let mut xo = Dict::default();
        xo.set(b"Im0", Obj::Ref(im));
        res.set(b"XObject", Obj::Dict(xo));
        content += &format!(
            "q {} 0 0 {} {} {} cm /Im0 Do Q\n",
            crate::pdf::object::real((r - l) as f64),
            crate::pdf::object::real((t - b) as f64),
            crate::pdf::object::real(l as f64),
            crate::pdf::object::real(b as f64)
        );
    }
    let mut form = Dict::default();
    form.set(b"Type", Obj::name("XObject"));
    form.set(b"Subtype", Obj::name("Form"));
    form.set(b"BBox", reals(&a.rect));
    form.set(b"Resources", Obj::Dict(res));
    Obj::Stream(form, content.into_bytes())
}

/// The annotation dictionary for `a`: the file's own dictionary with the
/// fields edits manages replaced (so keys edits does not know survive), or
/// a new one.
fn annot_dict(a: &Annot, base: Option<Dict>, page: Ref, ap: Option<Ref>) -> Dict {
    let had_base = base.is_some();
    let mut d = base.unwrap_or_default();
    d.set(b"Type", Obj::name("Annot"));
    if a.kind.drawable() {
        d.set(b"Subtype", Obj::name(a.kind.subtype()));
    }
    d.set(b"Rect", reals(&a.rect));
    d.set(b"P", Obj::Ref(page));
    if !had_base {
        // Print (bit 3), as other readers do for comments.
        d.set(b"F", Obj::Int(4));
    }
    match a.kind {
        // A text box's /C is its background; its text colour is in /DA.
        crate::model::Kind::FreeText => {
            match a.fill {
                Some(f) => d.set(b"C", reals(&f)),
                None => d.remove(b"C"),
            }
            let [r, g, b] = a.color;
            let da = format!(
                "/Helv {} Tf {} {} {} rg",
                crate::pdf::object::real(a.font_size as f64),
                crate::pdf::object::real(r as f64),
                crate::pdf::object::real(g as f64),
                crate::pdf::object::real(b as f64)
            );
            d.set(b"DA", Obj::Str(da.into_bytes()));
            let rtl = a.text_layout().lines.first().is_some_and(|l| l.rtl);
            d.set(b"Q", Obj::Int(if rtl { 2 } else { 0 }));
            d.set(b"CA", Obj::Real(a.opacity as f64));
        }
        k if k.drawable() => {
            d.set(b"C", reals(&a.color));
            d.set(b"CA", Obj::Real(a.opacity as f64));
        }
        _ => {}
    }
    match a.kind {
        crate::model::Kind::Note => {
            d.set(b"Name", Obj::name("Comment"));
            if d.get(b"Open").is_none() {
                d.set(b"Open", Obj::Bool(false));
            }
        }
        crate::model::Kind::Stamp if !a.label.is_empty() => {
            // Kept so that edits can draw the stamp again after reopening.
            d.set(b"EditsLabel", encode_text(&a.label));
            d.set(b"Name", Obj::name("Draft"));
        }
        crate::model::Kind::Stamp if a.image.is_some() => d.set(b"Name", Obj::name("Image")),
        _ => {}
    }
    if !a.modified.is_empty() {
        d.set(b"M", Obj::Str(a.modified.as_bytes().to_vec()));
        if !had_base {
            d.set(b"CreationDate", Obj::Str(a.modified.as_bytes().to_vec()));
        }
    }
    match a.author.as_str() {
        "" => d.remove(b"T"),
        t => d.set(b"T", encode_text(t)),
    }
    let old_contents = d.get(b"Contents").and_then(Obj::text).unwrap_or_default();
    if old_contents != a.contents {
        // Rich text would now say something else.
        d.remove(b"RC");
    }
    match a.contents.as_str() {
        "" => d.remove(b"Contents"),
        c => d.set(b"Contents", encode_text(c)),
    }
    if a.kind.is_markup() {
        d.set(b"QuadPoints", reals(&a.quads.concat()));
    }
    if a.kind == crate::model::Kind::Ink {
        d.set(b"InkList", Obj::Array(a.ink.iter().map(|s| reals(&s.concat())).collect()));
    }
    if let Some(l) = a.line {
        d.set(b"L", reals(&l));
        d.set(b"LE", Obj::Array(vec![Obj::name("None"), Obj::name(if a.arrow { "ClosedArrow" } else { "None" })]));
        if a.arrow {
            d.set(b"IC", reals(&a.color));
        }
    }
    if matches!(a.kind, crate::model::Kind::Square | crate::model::Kind::Circle) {
        match a.fill {
            Some(f) => d.set(b"IC", reals(&f)),
            None => d.remove(b"IC"),
        }
    }
    if a.kind.drawable()
        && !a.kind.is_markup()
        && !matches!(a.kind, crate::model::Kind::Note | crate::model::Kind::Stamp)
    {
        let mut bs = Dict::default();
        bs.set(b"W", Obj::Real(a.width as f64));
        d.set(b"BS", Obj::Dict(bs));
        d.remove(b"Border");
    }
    match ap {
        Some(r) => {
            let mut apd = Dict::default();
            apd.set(b"N", Obj::Ref(r));
            d.set(b"AP", Obj::Dict(apd));
            d.remove(b"AS");
        }
        // Kept appearance from another app: it follows /Rect when moved.
        None if a.own_ap => d.remove(b"AP"),
        None => {}
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;

    fn temp_pdf(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("edits-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, crate::pdf::write::tiny_pdf()).unwrap();
        p
    }

    /// The annotations PDF readers would see in base plus update.
    fn seen(s: &Session) -> Vec<Annot> {
        let mut bytes = std::fs::read(s.path()).unwrap();
        if let Some(t) = s.tail() {
            bytes.extend_from_slice(&t);
        }
        let f = PdfFile::open(bytes).unwrap();
        let (_, _, annots) = f.page_annots(0).unwrap();
        annots.iter().filter_map(|o| Annot::from_dict(0, f.resolve(o).unwrap().as_dict()?)).collect()
    }

    fn highlight() -> Annot {
        let mut a = Annot::new(0, Kind::Highlight);
        a.quads = vec![[10.0, 90.0, 60.0, 90.0, 10.0, 80.0, 60.0, 80.0]];
        a.fit_rect();
        a.color = [1.0, 1.0, 0.0];
        a
    }

    #[test]
    fn edits_build_an_update_that_readers_see() {
        let path = temp_pdf("a.pdf");
        let original = std::fs::read(&path).unwrap();
        let mut s = Session::open(&path).unwrap();
        s.import(0).unwrap();
        let listed = s.list(0);
        assert_eq!(listed.len(), 1);
        let square = listed[0].id;
        assert_eq!(listed[0].annot.kind, Kind::Square);
        assert!(!listed[0].annot.own_ap);
        assert!(s.tail().is_none() && !s.status().dirty);

        let (hl, page) = s.add(highlight()).unwrap();
        assert_eq!(page, 0);
        assert_eq!(seen(&s).len(), 2);

        // Move the square from the file: same object, new place.
        let mut moved = s.get(square).unwrap().clone();
        moved.translate(100.0, 0.0);
        s.change(square, moved).unwrap();
        let now = seen(&s);
        assert!(now.iter().any(|a| a.kind == Kind::Square && a.rect[0] == 110.0));

        s.remove(hl).unwrap();
        assert_eq!(seen(&s).len(), 1);
        s.undo().unwrap();
        assert_eq!(seen(&s).len(), 2);
        s.undo().unwrap(); // the move
        assert!(seen(&s).iter().any(|a| a.kind == Kind::Square && a.rect[0] == 10.0));
        s.redo().unwrap();

        // Save: the original bytes are kept and the result reads back.
        s.save(&path).unwrap();
        assert!(!s.status().dirty);
        assert!(s.tail().is_none());
        let saved = std::fs::read(&path).unwrap();
        assert!(saved.starts_with(&original));
        assert_eq!(seen(&s).len(), 2);

        // Undo after saving deletes the saved highlight in the next update.
        s.undo().unwrap(); // the move again
        s.undo().unwrap(); // the remove of hl: hl comes back (it is saved already)
        s.undo().unwrap(); // the add of hl: hl goes
        assert_eq!(seen(&s).len(), 1);
        s.save(&path).unwrap();
        let mut s2 = Session::open(&path).unwrap();
        s2.import(0).unwrap();
        assert_eq!(s2.list(0).len(), 1);
        assert_eq!(s2.list(0)[0].annot.rect[0], 10.0);
    }

    #[test]
    fn broken_protection_is_refused() {
        let path = temp_pdf("enc.pdf");
        let mut b = std::fs::read(&path).unwrap();
        let t = b.windows(7).rposition(|w| w == b"/Root 1").unwrap();
        b.splice(t..t, b"/Encrypt 9 0 R ".iter().copied());
        std::fs::write(&path, b).unwrap();
        assert!(Session::open(&path).is_err());
    }

    #[test]
    fn text_boxes_stamps_notes_and_pictures_embed_what_they_need() {
        let path = temp_pdf("text.pdf");
        let mut s = Session::open(&path).unwrap();
        let mut tb = Annot::new(0, Kind::FreeText);
        tb.rect = [10.0, 0.0, 150.0, 90.0];
        tb.contents = "مرحبا بالعالم
Hello"
            .into();
        tb.font_size = 10.0;
        tb.width = 0.0;
        tb.fit_text();
        assert!(tb.rect[1] > 40.0 && tb.rect[1] < 85.0, "two lines high: {:?}", tb.rect);
        s.add(tb).unwrap();
        let mut st = Annot::new(0, Kind::Stamp);
        st.label = "معتمد APPROVED".into();
        st.font_size = 16.0;
        st.color = [0.8, 0.0, 0.0];
        st.fit_label(20.0, 60.0);
        s.add(st).unwrap();
        let mut note = Annot::new(0, Kind::Note);
        note.rect = [150.0, 70.0, 170.0, 90.0];
        note.contents = "a note".into();
        s.add(note).unwrap();
        let mut png = Vec::new();
        {
            let mut e = png::Encoder::new(&mut png, 1, 1);
            e.set_color(png::ColorType::Rgb);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[0, 128, 255]).unwrap();
        }
        let mut pic = Annot::new(0, Kind::Stamp);
        pic.rect = [100.0, 10.0, 140.0, 30.0];
        pic.image = Some(std::sync::Arc::new(crate::image::PdfImage::from_bytes(&png).unwrap()));
        s.add(pic).unwrap();

        let mut bytes = std::fs::read(s.path()).unwrap();
        bytes.extend_from_slice(&s.tail().unwrap());
        let f = PdfFile::open(bytes).unwrap();
        let (_, _, annots) = f.page_annots(0).unwrap();
        assert_eq!(annots.len(), 5);
        let kinds: Vec<Kind> = annots
            .iter()
            .filter_map(|o| Annot::from_dict(0, f.resolve(o).unwrap().as_dict()?))
            .map(|a| a.kind)
            .collect();
        assert_eq!(kinds, [Kind::Square, Kind::FreeText, Kind::Stamp, Kind::Note, Kind::Stamp]);
        // The text box's appearance uses an embedded subset font with a ToUnicode map.
        let tb = f.resolve(&annots[1]).unwrap();
        let ap = f.resolve(tb.as_dict().unwrap().get(b"AP").unwrap()).unwrap();
        let n = f.resolve(ap.as_dict().unwrap().get(b"N").unwrap()).unwrap();
        let fonts = n.as_dict().unwrap().get(b"Resources").unwrap().as_dict().unwrap().get(b"Font").unwrap();
        let t0 = f.resolve(fonts.as_dict().unwrap().get(b"F0").unwrap()).unwrap();
        let t0 = t0.as_dict().unwrap();
        assert_eq!(t0.name(b"Subtype"), Some(&b"Type0"[..]));
        assert!(t0.get(b"ToUnicode").is_some());
        let stamp = Annot::from_dict(0, f.resolve(&annots[2]).unwrap().as_dict().unwrap()).unwrap();
        assert_eq!(stamp.label, "معتمد APPROVED", "the label survives for redrawing");
    }

    #[test]
    fn unsaved_work_survives_through_a_recovery_file() {
        let path = temp_pdf("recover.pdf");
        let mut s = Session::open(&path).unwrap();
        assert!(s.recovery().is_none(), "nothing to recover when clean");
        s.add(highlight()).unwrap();
        let json = serde_json::to_string(&s.recovery().unwrap()).unwrap();
        let tail = s.tail().unwrap();

        let mut s2 = Session::open(&path).unwrap();
        let r: Recovery = serde_json::from_str(&json).unwrap();
        assert_eq!(s2.restore(r).unwrap(), vec![0]);
        assert_eq!(s2.tail().unwrap(), tail);
        assert!(s2.status().dirty && s2.status().can_undo);
        s2.undo().unwrap();
        assert!(s2.tail().is_none());

        // A file changed since cannot take the old work.
        let r: Recovery = serde_json::from_str(&json).unwrap();
        std::fs::write(&path, [crate::pdf::write::tiny_pdf(), b"\n".to_vec()].concat()).unwrap();
        let mut s3 = Session::open(&path).unwrap();
        assert!(!s3.can_restore(&r));
        assert!(s3.restore(r).is_err());
    }
}
