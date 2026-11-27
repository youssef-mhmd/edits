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

#[derive(Debug, Clone)]
struct Entry {
    /// Every version this annotation has had; `cur` is the one in effect.
    versions: Vec<Annot>,
    cur: usize,
    alive: bool,
    /// Its object in the base file and the version the base holds.
    base: Option<(Ref, usize)>,
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

#[derive(Debug, Clone, Copy)]
enum Command {
    Add(AnnotId),
    Remove(AnnotId),
    Change { id: AnnotId, from: usize, to: usize },
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
}

fn open_base(path: &Path) -> Result<PdfFile<Arc<FileSource>>, PdfError> {
    let f = PdfFile::open(Arc::new(FileSource::open(path)?))?;
    if f.is_encrypted() {
        return Err(PdfError::Encrypted);
    }
    Ok(f)
}

impl Session {
    /// Opens `path` for editing. Fails for encrypted files and for files
    /// whose structure edits cannot read.
    pub fn open(path: &Path) -> Result<Self, PdfError> {
        Ok(Session {
            path: path.to_path_buf(),
            base: open_base(path)?,
            entries: BTreeMap::new(),
            imported: HashSet::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            next_id: 0,
            next_seq: 0,
            saved_at: 0,
            assigned: HashMap::new(),
            tail: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The base file's page count, to check against the renderer's.
    pub fn page_count(&self) -> Result<u32, PdfError> {
        Ok(self.base.pages()?.len() as u32)
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

    /// Reads the annotations of `page` from the base file, once.
    pub fn import(&mut self, page: u32) -> Result<(), PdfError> {
        if !self.imported.insert(page) {
            return Ok(());
        }
        let (_, _, annots) = self.base.page_annots(page)?;
        for o in annots {
            let Some(r) = o.as_ref() else { continue };
            let Ok(obj) = self.base.get(r) else { continue };
            let Some(a) = obj.as_dict().and_then(|d| Annot::from_dict(page, d)) else { continue };
            self.next_id += 1;
            self.entries.insert(self.next_id, Entry { versions: vec![a], cur: 0, alive: true, base: Some((r, 0)) });
        }
        Ok(())
    }

    /// The annotations on `page` now (import it first).
    pub fn list(&self, page: u32) -> Vec<Listed> {
        self.entries
            .iter()
            .filter(|(_, e)| e.alive && e.annot().page == page)
            .map(|(id, e)| Listed { id: *id, annot: e.annot().clone() })
            .collect()
    }

    pub fn get(&self, id: AnnotId) -> Option<&Annot> {
        self.entries.get(&id).filter(|e| e.alive).map(Entry::annot)
    }

    fn push(&mut self, c: Command) {
        self.next_seq += 1;
        self.undo.push((self.next_seq, c));
        self.redo.clear();
    }

    /// Adds an annotation; returns its id and the page to draw again.
    pub fn add(&mut self, a: Annot) -> Result<(AnnotId, u32), EditError> {
        let page = a.page;
        self.next_id += 1;
        let id = self.next_id;
        self.entries.insert(id, Entry { versions: vec![a], cur: 0, alive: true, base: None });
        self.push(Command::Add(id));
        self.rebuild()?;
        Ok((id, page))
    }

    /// Deletes an annotation; returns the page to draw again.
    pub fn remove(&mut self, id: AnnotId) -> Result<u32, EditError> {
        let e = self.entries.get_mut(&id).filter(|e| e.alive).ok_or(EditError::Unknown(id))?;
        e.alive = false;
        let page = e.annot().page;
        self.push(Command::Remove(id));
        self.rebuild()?;
        Ok(page)
    }

    /// Replaces an annotation with a changed version; returns the pages to
    /// draw again (it may have moved between pages).
    pub fn change(&mut self, id: AnnotId, a: Annot) -> Result<Vec<u32>, EditError> {
        let e = self.entries.get_mut(&id).filter(|e| e.alive).ok_or(EditError::Unknown(id))?;
        if *e.annot() == a {
            return Ok(Vec::new());
        }
        let pages = pages_of(&[e.annot().page, a.page]);
        let from = e.cur;
        e.versions.push(a);
        e.cur = e.versions.len() - 1;
        let to = e.cur;
        self.push(Command::Change { id, from, to });
        self.rebuild()?;
        Ok(pages)
    }

    /// Undoes the last command; returns the pages to draw again.
    pub fn undo(&mut self) -> Result<Vec<u32>, EditError> {
        let Some((seq, c)) = self.undo.pop() else { return Ok(Vec::new()) };
        self.redo.push((seq, c));
        let pages = self.apply(c, true);
        self.rebuild()?;
        Ok(pages)
    }

    pub fn redo(&mut self) -> Result<Vec<u32>, EditError> {
        let Some((seq, c)) = self.redo.pop() else { return Ok(Vec::new()) };
        self.undo.push((seq, c));
        let pages = self.apply(c, false);
        self.rebuild()?;
        Ok(pages)
    }

    fn apply(&mut self, c: Command, backwards: bool) -> Vec<u32> {
        match c {
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
        let pages: BTreeSet<u32> = self.entries.values().filter(|e| e.changed()).map(|e| e.annot().page).collect();
        // A changed page also loses the annotations moved away from it.
        let mut pages = pages;
        for e in self.entries.values() {
            if let Some((_, v)) = e.base
                && e.versions[v].page != e.annot().page
            {
                pages.insert(e.versions[v].page);
            }
        }
        let mut objects = Vec::new();
        let mut assigned = HashMap::new();
        let mut next = self.base.size;
        for page in pages {
            let (page_ref, mut page_dict, annots) = self.base.page_annots(page)?;
            // Base annotations now deleted or on another page leave this page.
            let leaving: HashSet<Ref> = self
                .entries
                .values()
                .filter_map(|e| {
                    let (r, _) = e.base?;
                    (!e.alive || e.annot().page != page).then_some(r)
                })
                .collect();
            let mut list: Vec<Obj> = annots
                .into_iter()
                .filter(|o| !o.as_ref().is_some_and(|r| leaving.contains(&r) || self.removes_popup(r, &leaving)))
                .collect();
            for (id, e) in &self.entries {
                let a = e.annot();
                if !e.alive || a.page != page || !e.changed() {
                    continue;
                }
                let (r, base_dict) = match e.base {
                    Some((r, _)) => (r, self.base.get(r)?.as_dict().cloned()),
                    None => {
                        let r = (next, 0);
                        next += 1;
                        assigned.insert(*id, r);
                        (r, None)
                    }
                };
                // An annotation that came from another page joins this one's list.
                if !list.iter().any(|o| o.as_ref() == Some(r)) {
                    list.push(Obj::Ref(r));
                }
                let ap = if a.own_ap && a.kind.drawable() {
                    let ar = (next, 0);
                    next += 1;
                    objects.push((ar, appearance(a)));
                    Some(ar)
                } else {
                    None
                };
                objects.push((r, Obj::Dict(annot_dict(a, base_dict, page_ref, ap))));
            }
            page_dict.set(b"Annots", Obj::Array(list));
            objects.push((page_ref, Obj::Dict(page_dict)));
        }
        Ok((objects, assigned))
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
        let len = result?;
        self.base = open_base(target)?;
        self.path = target.to_path_buf();
        for (id, e) in &mut self.entries {
            if e.alive {
                let r = self.assigned.get(id).copied().or(e.base.map(|b| b.0));
                e.base = r.map(|r| (r, e.cur));
            } else {
                e.base = None;
            }
        }
        self.assigned.clear();
        self.tail = None;
        self.saved_at = self.at();
        Ok(len)
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

/// The appearance stream of `a`, as edits draws it.
fn appearance(a: &Annot) -> Obj {
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
    let mut form = Dict::default();
    form.set(b"Type", Obj::name("XObject"));
    form.set(b"Subtype", Obj::name("Form"));
    form.set(b"BBox", reals(&a.rect));
    form.set(b"Resources", Obj::Dict(res));
    Obj::Stream(form, crate::ap::content(a).into_bytes())
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
    if a.kind.drawable() {
        d.set(b"C", reals(&a.color));
        d.set(b"CA", Obj::Real(a.opacity as f64));
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
    if a.kind.drawable() && !a.kind.is_markup() {
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
    fn encrypted_files_are_refused() {
        let path = temp_pdf("enc.pdf");
        let mut b = std::fs::read(&path).unwrap();
        let t = b.windows(7).rposition(|w| w == b"/Root 1").unwrap();
        b.splice(t..t, b"/Encrypt 9 0 R ".iter().copied());
        std::fs::write(&path, b).unwrap();
        assert!(matches!(Session::open(&path), Err(PdfError::Encrypted)));
    }
}
