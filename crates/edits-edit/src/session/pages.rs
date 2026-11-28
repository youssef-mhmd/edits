//! Organising pages (PRD 5.3): the document's pages as a list of slots that
//! commands reorder, turn, crop, delete, copy and insert. Every list the
//! document has had is kept, so undo and redo just pick another one.
//!
//! A slot has a key that never changes; annotations refer to that key, so
//! they follow their page wherever it goes. Pages of the base file start
//! with their index as key. A base page's first slot ("owner") is the page
//! object itself; copies of it are written as new page objects.
//!
//! The update writes the page tree's top node again with the new list, so
//! pages keep their objects (and links and bookmarks to them stay right).
//! Pages from other files are copied in with what they use.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::decor::{self, Decor};
use super::{Command, EditError, Entry, Planned, Session, annot_dict, appearance, image_objects, reals};
use crate::image::PdfImage;
use crate::model::AnnotId;
use crate::pdf::copy::{Copier, place};
use crate::pdf::{Dict, FileSource, Obj, PdfError, PdfFile, Ref, Source};

/// Where a page comes from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PageSrc {
    /// Page `n` of the base file.
    Base(u32),
    /// An empty page, in points.
    Blank { w: f32, h: f32 },
    /// A page showing picture `n` of the session's pictures.
    Image(u32),
    /// Picture `n` centred on a page `w` x `h` points, as large as fits.
    ImageOn { n: u32, w: f32, h: f32 },
    /// Page `page` of another file (`file` indexes the session's list).
    Foreign { file: u32, page: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageSlot {
    pub key: u32,
    pub src: PageSrc,
    /// Quarter turns clockwise added to the page's own rotation.
    #[serde(default)]
    pub turn: u8,
    /// Crop box in user space (left, bottom, right, top).
    #[serde(default)]
    pub crop: Option<[f32; 4]>,
    /// Headers, footers and watermarks added (indexes into `Session::decors`).
    #[serde(default)]
    pub decor: Vec<u32>,
    /// Recognised text added as an invisible layer (index into `Session::ocr`).
    #[serde(default)]
    pub ocr: Option<u32>,
    /// The page's items as changed in this session (index into
    /// `Session::item_lists`); None keeps what the page has.
    #[serde(default)]
    pub items: Option<u32>,
}

impl PageSlot {
    /// True if `other` shows the same page the same way (its content may differ).
    fn same_layout(&self, other: &PageSlot) -> bool {
        self.key == other.key && self.src == other.src && self.turn == other.turn && self.crop == other.crop
    }
}

/// The page keys whose content differs between two lists of the same layout.
fn content_keys(a: &[PageSlot], b: &[PageSlot]) -> Vec<u32> {
    a.iter().zip(b).filter(|(x, y)| x != y).map(|(x, _)| x.key).collect()
}

/// True if the two lists show the same pages in the same places.
pub(super) fn same_layout(a: &[PageSlot], b: &[PageSlot]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same_layout(y))
}

impl PageSlot {
    fn owner(&self) -> bool {
        matches!(self.src, PageSrc::Base(i) if i == self.key)
    }
}

/// The base file's pages, unchanged.
pub(super) fn identity(n: u32) -> Vec<PageSlot> {
    (0..n)
        .map(|i| PageSlot {
            key: i,
            src: PageSrc::Base(i),
            turn: 0,
            crop: None,
            decor: Vec::new(),
            ocr: None,
            items: None,
        })
        .collect()
}

pub(super) fn is_identity(order: &[PageSlot], n: u32) -> bool {
    order.len() == n as usize
        && order.iter().enumerate().all(|(i, s)| {
            s.src == PageSrc::Base(i as u32)
                && s.key == i as u32
                && s.turn == 0
                && s.crop.is_none()
                && s.decor.is_empty()
                && s.ocr.is_none()
                && s.items.is_none()
        })
}

/// Another file pages were taken from, and the copies made of them.
pub(super) struct Foreign {
    pub path: PathBuf,
    file: PdfFile<Arc<FileSource>>,
    copier: Copier,
    /// Page index -> its page object among the copies.
    pages: HashMap<u32, Ref>,
    /// Pages in the order they were copied (to copy them again identically
    /// after a crash).
    pub copied: Vec<u32>,
}

impl Foreign {
    pub(super) fn open(path: &Path) -> Result<Self, PdfError> {
        // Files with a password fail here (Encrypted); permissions-only ones open.
        let file = PdfFile::open(Arc::new(FileSource::open(path)?))?;
        Ok(Foreign {
            path: path.to_path_buf(),
            file,
            copier: Copier::default(),
            pages: HashMap::new(),
            copied: Vec::new(),
        })
    }

    pub(super) fn page_count(&self) -> Result<u32, PdfError> {
        Ok(self.file.pages()?.len() as u32)
    }

    /// Copies page `page` with everything it uses (once).
    pub(super) fn copy_page(&mut self, page: u32) -> Result<(), PdfError> {
        if self.pages.contains_key(&page) {
            return Ok(());
        }
        let refs = self.file.pages()?;
        let r = *refs.get(page as usize).ok_or(PdfError::Damaged(format!("no page {page}")))?;
        let mut dict = page_dict(&self.file, r)?;
        // Its place in the other file's structure tree means nothing here.
        dict.remove(b"StructParents");
        let local = self.copier.reserve();
        let others: HashSet<u32> = refs.iter().map(|p| p.0).collect();
        let mut stop = |x: Ref| {
            if x.0 == r.0 {
                Some(Obj::Ref(local))
            } else if others.contains(&x.0) {
                // Links to pages not copied lead nowhere.
                Some(Obj::Null)
            } else {
                None
            }
        };
        let d = self.copier.copy(&self.file, &Obj::Dict(dict), &mut stop)?;
        self.copier.set(local, d);
        self.pages.insert(page, local);
        self.copied.push(page);
        Ok(())
    }
}

const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// A page's dictionary with the attributes it inherits made its own, and no /Parent.
pub(crate) fn page_dict<S: Source>(f: &PdfFile<S>, page: Ref) -> Result<Dict, PdfError> {
    let mut d = f.get(page)?.as_dict().cloned().unwrap_or_default();
    let mut parent = d.get(b"Parent").and_then(Obj::as_ref);
    let mut seen = HashSet::new();
    while let Some(p) = parent {
        if !seen.insert(p.0) || seen.len() > 64 {
            break;
        }
        let pd = f.get(p)?;
        let Some(pd) = pd.as_dict() else { break };
        for k in INHERITED {
            if d.get(k).is_none()
                && let Some(v) = pd.get(k)
            {
                d.set(k, v.clone());
            }
        }
        parent = pd.get(b"Parent").and_then(Obj::as_ref);
    }
    for k in [&b"MediaBox"[..], b"CropBox", b"Rotate"] {
        // Values written once as references are read now.
        if let Some(Obj::Ref(r)) = d.get(k).cloned() {
            d.set(k, f.get(r)?);
        }
    }
    if d.get(b"MediaBox").and_then(Obj::numbers).is_none_or(|b| b.len() != 4) {
        d.set(b"MediaBox", reals(&[0.0, 0.0, 612.0, 792.0]));
    }
    // Resources as a dictionary of the page's own, with fonts and graphics
    // states too, so that headers and footers can add theirs.
    let mut res = match d.get(b"Resources") {
        Some(r) => f.resolve(r)?.as_dict().cloned().unwrap_or_default(),
        None => Dict::default(),
    };
    for k in [&b"Font"[..], b"ExtGState"] {
        if let Some(Obj::Ref(r)) = res.get(k).cloned() {
            res.set(k, f.get(r)?);
        }
    }
    d.set(b"Resources", Obj::Dict(res));
    d.set(b"Type", Obj::name("Page"));
    d.remove(b"Parent");
    Ok(d)
}

/// Objects being written, numbered from the base file's size up.
pub(super) struct Out {
    pub objects: Vec<(Ref, Obj)>,
    pub next: u32,
}

impl Out {
    pub fn reserve(&mut self) -> Ref {
        let r = (self.next, 0);
        self.next += 1;
        r
    }

    pub fn add(&mut self, o: Obj) -> Ref {
        let r = self.reserve();
        self.objects.push((r, o));
        r
    }
}

/// A picture filling a page of its own shape, at most A4 in size.
fn image_page_size(img: &PdfImage) -> (f32, f32) {
    let (w, h) = (img.width.max(1) as f32, img.height.max(1) as f32);
    let (bw, bh) = if w > h { (842.0, 595.0) } else { (595.0, 842.0) };
    let k = (bw / w).min(bh / h);
    ((w * k).max(1.0), (h * k).max(1.0))
}

/// A new PDF of `pages` empty pages, `w` x `h` points. `margins` (left,
/// top, right, bottom, in points) are kept in the catalog for the guides
/// edits shows when placing items (/EditsMargins).
pub fn blank_pdf(w: f32, h: f32, pages: u32, margins: Option<[f32; 4]>) -> Vec<u8> {
    let r = |v: f32| crate::pdf::object::real(v as f64);
    let n = pages.clamp(1, 10_000);
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i)).collect();
    let mut catalog = String::from("<< /Type /Catalog /Pages 2 0 R");
    if let Some(m) = margins {
        catalog += &format!(" /EditsMargins [{} {} {} {}]", r(m[0]), r(m[1]), r(m[2]), r(m[3]));
    }
    catalog += " >>";
    let mut objs = vec![catalog, format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" "))];
    for _ in 0..n {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << >> >>", r(w), r(h)));
    }
    let mut b = b"%PDF-1.7\n".to_vec();
    let mut offs = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offs.push(b.len());
        b.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let x = b.len();
    b.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", objs.len() + 1).as_bytes());
    for o in offs {
        b.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
    }
    b.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{x}\n%%EOF\n", objs.len() + 1).as_bytes(),
    );
    b
}

/// Writes a new PDF at `target` with one page per picture, in order: on
/// pages of `size` points (turned to suit each picture), or, when None, on
/// pages the picture's shape. Returns the bytes written.
pub fn images_to_pdf(images: Vec<PdfImage>, size: Option<(f32, f32)>, target: &Path) -> Result<u64, EditError> {
    if images.is_empty() {
        return Err(EditError::Pages("no pictures chosen".into()));
    }
    let n = images.len() as u32;
    let base = super::temp_path_for(target).with_extension("edits-base");
    std::fs::write(&base, blank_pdf(595.0, 842.0, 1, None))?;
    let result = (|| {
        let mut s = Session::open(&base)?;
        s.insert_images_on(0, images, size)?;
        s.export(&(0..n).collect::<Vec<_>>(), target)
    })();
    let _ = std::fs::remove_file(&base);
    result
}

impl Session {
    pub(super) fn order(&self) -> &[PageSlot] {
        &self.orders[self.order_at]
    }

    pub(super) fn reindex(&mut self) {
        self.pos = self.orders[self.order_at].iter().enumerate().map(|(i, s)| (s.key, i as u32)).collect();
    }

    /// The slot key of the page now at `index`.
    pub(super) fn key_of(&self, index: u32) -> Option<u32> {
        self.order().get(index as usize).map(|s| s.key)
    }

    /// Where the page with `key` is now, if it is still in the document.
    pub(super) fn index_of(&self, key: u32) -> Option<u32> {
        self.pos.get(&key).copied()
    }

    /// True once after a command changed the page list (the document has
    /// to be shown anew, not just some pages drawn again).
    pub fn take_layout_changed(&mut self) -> bool {
        std::mem::take(&mut self.layout_changed)
    }

    /// Records what going from page list `a` to `b` changes: the layout (the
    /// document is shown anew) or only the content of some pages.
    pub(super) fn note_change(&mut self, a: &[PageSlot], b: &[PageSlot]) {
        if same_layout(a, b) {
            self.redraw.extend(content_keys(a, b));
        } else {
            self.layout_changed = true;
        }
    }

    /// The pages (indexes now) whose content changed since last asked.
    pub fn take_redraw(&mut self) -> Vec<u32> {
        let keys = std::mem::take(&mut self.redraw);
        keys.into_iter().filter_map(|k| self.index_of(k)).collect()
    }

    /// True if the pages differ from the base file's.
    pub fn pages_changed(&self) -> bool {
        self.flattened || !is_identity(self.order(), self.base_pages)
    }

    pub(super) fn set_order(&mut self, new: Vec<PageSlot>) -> Result<(), EditError> {
        if new.is_empty() {
            return Err(EditError::Pages("a document needs at least one page".into()));
        }
        if new == self.order() {
            return Ok(());
        }
        let from = self.order_at;
        let layout = self.layout_changed;
        self.note_change(&self.orders[from].clone(), &new);
        self.orders.push(new);
        self.order_at = self.orders.len() - 1;
        self.reindex();
        self.push(Command::Pages { from, to: self.order_at });
        if let Err(e) = self.rebuild() {
            // Leave things as they were.
            self.undo.pop();
            self.layout_changed = layout;
            self.order_at = from;
            self.orders.pop();
            self.reindex();
            return Err(e.into());
        }
        Ok(())
    }

    /// Changes the page list now without a new undo step (the last step
    /// takes the change).
    pub(super) fn replace_order(&mut self, new: Vec<PageSlot>) -> Result<(), EditError> {
        self.note_change(&self.orders[self.order_at].clone(), &new);
        let old = std::mem::replace(&mut self.orders[self.order_at], new);
        self.reindex();
        if let Err(e) = self.rebuild() {
            self.orders[self.order_at] = old;
            self.reindex();
            return Err(e.into());
        }
        Ok(())
    }

    pub(super) fn checked(&self, pages: &[u32]) -> Result<BTreeSet<usize>, EditError> {
        let n = self.order().len();
        let set: BTreeSet<usize> = pages.iter().map(|p| *p as usize).collect();
        match set.iter().find(|p| **p >= n) {
            Some(p) => Err(EditError::Pages(format!("there is no page {}", p + 1))),
            None => Ok(set),
        }
    }

    fn new_key(&mut self) -> u32 {
        let k = self.next_key;
        self.next_key += 1;
        k
    }

    /// Moves `pages` (indexes) together to just before the page now at
    /// `before` (the page count puts them at the end), keeping their order.
    pub fn move_pages(&mut self, pages: &[u32], before: u32) -> Result<(), EditError> {
        let set = self.checked(pages)?;
        let order = self.order();
        let moving: Vec<PageSlot> = set.iter().map(|i| order[*i].clone()).collect();
        let at = (0..(before as usize).min(order.len())).filter(|i| !set.contains(i)).count();
        let mut rest: Vec<PageSlot> =
            order.iter().enumerate().filter(|(i, _)| !set.contains(i)).map(|(_, s)| s.clone()).collect();
        rest.splice(at..at, moving);
        self.set_order(rest)
    }

    /// Turns `pages` by `quarters` quarter turns clockwise (negative: anticlockwise).
    pub fn rotate_pages(&mut self, pages: &[u32], quarters: i32) -> Result<(), EditError> {
        let set = self.checked(pages)?;
        let mut order = self.order().to_vec();
        for i in set {
            order[i].turn = ((order[i].turn as i32 + quarters).rem_euclid(4)) as u8;
        }
        self.set_order(order)
    }

    pub fn delete_pages(&mut self, pages: &[u32]) -> Result<(), EditError> {
        let set = self.checked(pages)?;
        let order = self.order().iter().enumerate().filter(|(i, _)| !set.contains(i)).map(|(_, s)| s.clone()).collect();
        self.set_order(order)
    }

    /// Copies `pages`, each just after itself, with their annotations.
    pub fn duplicate_pages(&mut self, pages: &[u32]) -> Result<(), EditError> {
        let set = self.checked(pages)?;
        let mut order = Vec::new();
        for (i, s) in self.order().to_vec().into_iter().enumerate() {
            order.push(s.clone());
            if set.contains(&i) {
                if s.owner() {
                    self.import(i as u32)?;
                }
                let key = self.new_key();
                // Its annotations, as new ones made from the originals.
                let copies: Vec<Entry> = self
                    .entries
                    .values()
                    .filter(|e| e.alive && e.annot().page == s.key)
                    .map(|e| {
                        let mut a = e.annot().clone();
                        a.page = key;
                        Entry {
                            versions: vec![a],
                            cur: 0,
                            alive: true,
                            base: None,
                            template: e.base.map(|b| b.0).or(e.template),
                        }
                    })
                    .collect();
                for c in copies {
                    self.next_id += 1;
                    self.entries.insert(self.next_id as AnnotId, c);
                }
                order.push(PageSlot { key, ..s });
            }
        }
        self.set_order(order)
    }

    fn insert(&mut self, at: u32, srcs: Vec<PageSrc>) -> Result<(), EditError> {
        let mut order = self.order().to_vec();
        let at = (at as usize).min(order.len());
        let slots: Vec<PageSlot> = srcs
            .into_iter()
            .map(|src| PageSlot {
                key: self.new_key(),
                src,
                turn: 0,
                crop: None,
                decor: Vec::new(),
                ocr: None,
                items: None,
            })
            .collect();
        order.splice(at..at, slots);
        self.set_order(order)
    }

    /// Inserts `count` empty pages of `w` x `h` points before index `at`.
    pub fn insert_blank(&mut self, at: u32, w: f32, h: f32, count: u32) -> Result<(), EditError> {
        let (w, h) = (w.clamp(36.0, 14400.0), h.clamp(36.0, 14400.0));
        self.insert(at, (0..count.clamp(1, 1000)).map(|_| PageSrc::Blank { w, h }).collect())
    }

    /// Inserts a page for each picture before index `at`.
    pub fn insert_images(&mut self, at: u32, images: Vec<PdfImage>) -> Result<(), EditError> {
        self.insert_images_on(at, images, None)
    }

    /// Inserts pictures as pages before index `at`: on pages of `size`
    /// points (width, height; turned to suit each picture), or, when None,
    /// on pages the picture's shape.
    pub fn insert_images_on(
        &mut self,
        at: u32,
        images: Vec<PdfImage>,
        size: Option<(f32, f32)>,
    ) -> Result<(), EditError> {
        let mut srcs = Vec::new();
        for img in images {
            let landscape = img.width > img.height;
            self.images.push(Arc::new(img));
            let n = self.images.len() as u32 - 1;
            srcs.push(match size {
                None => PageSrc::Image(n),
                Some((a, b)) => {
                    let (short, long) = (a.min(b).clamp(72.0, 14_400.0), a.max(b).clamp(72.0, 14_400.0));
                    let (w, h) = if landscape { (long, short) } else { (short, long) };
                    PageSrc::ImageOn { n, w, h }
                }
            });
        }
        self.insert(at, srcs)
    }

    /// Inserts pages of the PDF at `path` (all of them when `pages` is None)
    /// before index `at`. Returns how many were inserted.
    pub fn insert_pdf(&mut self, at: u32, path: &Path, pages: Option<Vec<u32>>) -> Result<u32, EditError> {
        let file = match self.foreign.iter().position(|f| f.path == path) {
            Some(i) => i,
            None => {
                self.foreign.push(Foreign::open(path)?);
                self.foreign.len() - 1
            }
        };
        let f = &mut self.foreign[file];
        let n = f.page_count()?;
        let pages = pages.unwrap_or_else(|| (0..n).collect());
        if pages.iter().any(|p| *p >= n) {
            return Err(EditError::Pages(format!("{} has only {n} pages", path.display())));
        }
        for p in &pages {
            f.copy_page(*p)?;
        }
        let count = pages.len() as u32;
        self.insert(at, pages.into_iter().map(|page| PageSrc::Foreign { file: file as u32, page }).collect())?;
        Ok(count)
    }

    /// Sets the crop box of each page in `boxes` (index, user-space box, or
    /// None to show the whole page again).
    pub fn crop_pages(&mut self, boxes: &[(u32, Option<[f32; 4]>)]) -> Result<(), EditError> {
        let idx: Vec<u32> = boxes.iter().map(|b| b.0).collect();
        self.checked(&idx)?;
        let mut order = self.order().to_vec();
        for (i, b) in boxes {
            order[*i as usize].crop = b.map(|b| [b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3])]);
        }
        self.set_order(order)
    }

    /// Adds page numbers, headers, footers or a watermark to `pages`.
    pub fn decorate_pages(&mut self, pages: &[u32], d: Decor) -> Result<(), EditError> {
        let set = self.checked(pages)?;
        let d = d.clamped();
        if d.is_empty() {
            return Err(EditError::Pages("there is nothing to add".into()));
        }
        let id = match self.decors.iter().position(|x| *x == d) {
            Some(i) => i as u32,
            None => {
                self.decors.push(d);
                self.decors.len() as u32 - 1
            }
        };
        let mut order = self.order().to_vec();
        for i in set {
            order[i].decor.push(id);
        }
        self.set_order(order)
    }

    /// Takes away the headers, footers and watermarks added to `pages`
    /// since the file was last saved.
    pub fn undecorate_pages(&mut self, pages: &[u32]) -> Result<(), EditError> {
        let set = self.checked(pages)?;
        let mut order = self.order().to_vec();
        for i in set {
            order[i].decor.clear();
        }
        self.set_order(order)
    }

    /// Writes `pages` (indexes, in that order) as a new PDF at `target`,
    /// with annotations and other changes, without bookmarks.
    pub fn export(&self, pages: &[u32], target: &Path) -> Result<u64, EditError> {
        if pages.is_empty() {
            return Err(EditError::Pages("no pages chosen".into()));
        }
        self.checked(pages)?;
        let order: Vec<PageSlot> = pages.iter().map(|i| self.order()[*i as usize].clone()).collect();
        let (objects, _) = self.objects_for(&order, true)?;
        let tail = crate::pdf::write::update(&self.base, &objects);
        self.write_whole(Arc::new(tail), target)
    }

    /// Writes base plus `tail` to `target` as a new file with only what is
    /// in use, through a temporary file.
    pub(super) fn write_whole(&self, tail: Arc<Vec<u8>>, target: &Path) -> Result<u64, EditError> {
        let src = crate::pdf::Layered::new(Arc::new(FileSource::open(&self.path)?), tail);
        let f = PdfFile::open(src)?;
        let tmp = super::temp_path_for(target);
        let result = (|| -> Result<u64, EditError> {
            let file = std::fs::File::create(&tmp)?;
            let mut w = std::io::BufWriter::new(file);
            let n = crate::pdf::copy::rewrite(&f, &mut w)?;
            let file = w.into_inner().map_err(|e| e.into_error())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, target)?;
            Ok(n)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// True if a base page is no longer in the document: saving then writes
    /// the whole file, so the deleted pages' content is really gone.
    pub(super) fn drops_base_pages(&self) -> bool {
        let kept: HashSet<u32> = self.order().iter().filter(|s| s.owner()).map(|s| s.key).collect();
        (0..self.base_pages).any(|i| !kept.contains(&i))
    }

    /// The objects an update writes for the document with pages `order`.
    /// `export`: a new file of these pages only, without bookmarks.
    pub(super) fn objects_for(&self, order: &[PageSlot], export: bool) -> Result<Planned, PdfError> {
        let base_pages = self.base.pages()?;
        let structural = export || self.flattened || !is_identity(order, base_pages.len() as u32);
        // Pages (keys) whose annotations change.
        let mut touched: BTreeSet<u32> =
            self.entries.values().filter(|e| e.changed()).map(|e| e.annot().page).collect();
        for e in self.entries.values() {
            if let Some((_, v)) = e.base
                && e.versions[v].page != e.annot().page
            {
                touched.insert(e.versions[v].page);
            }
        }
        let mut out = Out { objects: Vec::new(), next: self.base.size };
        let mut assigned = HashMap::new();
        let catalog_ref = self.base.trailer.get(b"Root").and_then(Obj::as_ref);
        let catalog = self.base.resolve(self.base.trailer.get(b"Root").unwrap_or(&Obj::Null))?;
        let root_ref = catalog.as_dict().and_then(|d| d.get(b"Pages")).and_then(Obj::as_ref);

        // Pages copied from other files, with what they use.
        let mut starts: HashMap<u32, u32> = HashMap::new();
        if structural {
            for s in order {
                let PageSrc::Foreign { file, .. } = s.src else { continue };
                if starts.contains_key(&file) {
                    continue;
                }
                let f = &self.foreign[file as usize];
                let start = out.next;
                out.next += f.copier.objects.len() as u32;
                starts.insert(file, start);
                let page_objs: HashSet<u32> = f.pages.values().map(|r| r.0).collect();
                for (i, o) in f.copier.objects.iter().enumerate() {
                    if page_objs.contains(&(i as u32)) {
                        continue;
                    }
                    let mut o = o.clone();
                    place(&mut o, start);
                    out.objects.push(((start + i as u32, 0), o));
                }
            }
        }

        let mut kids = Vec::with_capacity(order.len());
        let mut page_refs: HashMap<u32, Ref> = HashMap::new();
        // Pages with headers and footers: where their dictionary is in
        // `out.objects`, and their text laid out.
        let mut decorated: Vec<(usize, decor::PageText)> = Vec::new();
        // Pages with recognised text: where their dictionary is, and the layer.
        let mut recognised: Vec<(usize, u32)> = Vec::new();
        // Pages with items: where their dictionary is, and the items.
        let mut itemed: Vec<(usize, Arc<Vec<super::Item>>)> = Vec::new();
        let mut foreign_seen: HashSet<(u32, u32)> = HashSet::new();
        for s in order {
            // The page object, its dictionary, its annotations now, and whether to write it.
            let (page_ref, mut dict, mut list, write) = match &s.src {
                PageSrc::Base(i) if s.owner() => {
                    let r = *base_pages.get(*i as usize).ok_or(PdfError::Damaged(format!("no page {i}")))?;
                    let (_, raw, annots) = self.base.page_annots(*i)?;
                    let leaving: HashSet<Ref> = self
                        .entries
                        .values()
                        .filter_map(|e| {
                            let (r, _) = e.base?;
                            (!e.alive || e.annot().page != s.key).then_some(r)
                        })
                        .collect();
                    let list: Vec<Obj> = annots
                        .into_iter()
                        .filter(|o| {
                            !o.as_ref().is_some_and(|r| leaving.contains(&r) || self.removes_popup(r, &leaving))
                        })
                        .collect();
                    if !structural {
                        (r, raw, list, touched.contains(&s.key))
                    } else {
                        let under_root = raw.get(b"Parent").and_then(Obj::as_ref) == root_ref;
                        let write = !under_root
                            || s.turn != 0
                            || s.crop.is_some()
                            || !s.decor.is_empty()
                            || s.ocr.is_some()
                            || s.items.is_some()
                            || self.flattened
                            || touched.contains(&s.key);
                        let dict = if write { page_dict(&self.base, r)? } else { raw };
                        (r, dict, list, write)
                    }
                }
                PageSrc::Base(i) => {
                    // A copy of a page: its comments come as copied entries;
                    // its links are copied here.
                    let r = *base_pages.get(*i as usize).ok_or(PdfError::Damaged(format!("no page {i}")))?;
                    let mut d = page_dict(&self.base, r)?;
                    let annots = match d.get(b"Annots") {
                        Some(a) => self.base.resolve(a)?.as_array().map(<[Obj]>::to_vec).unwrap_or_default(),
                        None => Vec::new(),
                    };
                    d.remove(b"Annots");
                    d.remove(b"StructParents");
                    let me = out.reserve();
                    let mut links = Vec::new();
                    for a in annots {
                        let Ok(Obj::Dict(mut l)) = self.base.resolve(&a) else { continue };
                        if l.name(b"Subtype") != Some(b"Link") {
                            continue;
                        }
                        l.set(b"P", Obj::Ref(me));
                        l.remove(b"StructParent");
                        links.push(Obj::Ref(out.add(Obj::Dict(l))));
                    }
                    (me, d, links, true)
                }
                PageSrc::Blank { w, h } => {
                    let mut d = Dict::default();
                    d.set(b"Type", Obj::name("Page"));
                    d.set(b"MediaBox", reals(&[0.0, 0.0, *w, *h]));
                    d.set(b"Resources", Obj::Dict(Dict::default()));
                    (out.reserve(), d, Vec::new(), true)
                }
                PageSrc::Image(n) | PageSrc::ImageOn { n, .. } => {
                    let img = self.images.get(*n as usize).ok_or(PdfError::Damaged("picture missing".into()))?;
                    let (pw, ph) = match s.src {
                        PageSrc::ImageOn { w, h, .. } => (w, h),
                        _ => image_page_size(img),
                    };
                    // The picture as large as fits, centred.
                    let k = (pw / img.width.max(1) as f32).min(ph / img.height.max(1) as f32);
                    let (w, h) = (img.width.max(1) as f32 * k, img.height.max(1) as f32 * k);
                    let (x, y) = ((pw - w) / 2.0, (ph - h) / 2.0);
                    let im = image_objects(img, &mut |o| out.add(o));
                    let mut xo = Dict::default();
                    xo.set(b"Im0", Obj::Ref(im));
                    let mut res = Dict::default();
                    res.set(b"XObject", Obj::Dict(xo));
                    let r = |v: f32| crate::pdf::object::real(v as f64);
                    let ops = format!("q {} 0 0 {} {} {} cm /Im0 Do Q", r(w), r(h), r(x), r(y));
                    let content = out.add(Obj::Stream(Dict::default(), ops.into_bytes()));
                    let mut d = Dict::default();
                    d.set(b"Type", Obj::name("Page"));
                    d.set(b"MediaBox", reals(&[0.0, 0.0, pw, ph]));
                    d.set(b"Resources", Obj::Dict(res));
                    d.set(b"Contents", Obj::Ref(content));
                    (out.reserve(), d, Vec::new(), true)
                }
                PageSrc::Foreign { file, page } => {
                    let f = &self.foreign[*file as usize];
                    let local = *f.pages.get(page).ok_or(PdfError::Damaged("copied page missing".into()))?;
                    let start = starts[file];
                    let mut d = f.copier.objects[local.0 as usize].clone();
                    place(&mut d, start);
                    let mut d = d.as_dict().cloned().unwrap_or_default();
                    let mut list = match d.get(b"Annots") {
                        Some(Obj::Array(a)) => a.clone(),
                        _ => Vec::new(),
                    };
                    let first = foreign_seen.insert((*file, *page));
                    let r = if first { (start + local.0, 0) } else { out.reserve() };
                    if !first {
                        // A second copy of the same page gets no annotations of its own.
                        list.clear();
                    }
                    d.remove(b"Annots");
                    (r, d, list, true)
                }
            };
            page_refs.insert(s.key, page_ref);
            kids.push(Obj::Ref(page_ref));
            if !write {
                continue;
            }
            // Annotations that change on this page.
            for (id, e) in &self.entries {
                let a = e.annot();
                if !e.alive || a.page != s.key || !e.changed() {
                    continue;
                }
                let (r, base_dict) = match (e.base, e.template) {
                    (Some((r, _)), _) => (r, self.base.get(r)?.as_dict().cloned()),
                    (None, t) => {
                        let r = out.reserve();
                        assigned.insert(*id, r);
                        let d = match t {
                            Some(t) => self.base.get(t)?.as_dict().cloned().map(|mut d| {
                                for k in [&b"Popup"[..], b"Parent", b"NM", b"StructParent"] {
                                    d.remove(k);
                                }
                                d
                            }),
                            None => None,
                        };
                        (r, d)
                    }
                };
                if !list.iter().any(|o| o.as_ref() == Some(r)) {
                    list.push(Obj::Ref(r));
                }
                let ap = if a.own_ap && a.drawable() {
                    let form = appearance(a, &mut |o| out.add(o));
                    Some(out.add(form))
                } else {
                    None
                };
                out.objects.push((r, Obj::Dict(annot_dict(a, base_dict, page_ref, ap))));
            }
            if !list.is_empty() || dict.get(b"Annots").is_some() {
                dict.set(b"Annots", Obj::Array(list));
            }
            if structural {
                if let Some(root) = root_ref {
                    dict.set(b"Parent", Obj::Ref(root));
                }
                let own = dict.get(b"Rotate").and_then(Obj::as_int).unwrap_or(0);
                dict.set(b"Rotate", Obj::Int((own + 90 * s.turn as i64).rem_euclid(360)));
                match s.crop {
                    Some(c) => dict.set(b"CropBox", reals(&c)),
                    // New pages must not take a crop box from the tree they join.
                    None if dict.get(b"CropBox").is_none() => {
                        if let Some(m) = dict.get(b"MediaBox").cloned() {
                            dict.set(b"CropBox", m);
                        }
                    }
                    None => {}
                }
            }
            if structural && !s.decor.is_empty() {
                let num = |k: &[u8]| dict.get(k).and_then(Obj::numbers).filter(|b| b.len() == 4);
                let b = s.crop.map(|c| c.to_vec()).or_else(|| num(b"CropBox")).or_else(|| num(b"MediaBox"));
                if let Some(b) = b {
                    let frame = decor::Frame {
                        bx: [b[0].min(b[2]), b[1].min(b[3]), b[0].max(b[2]), b[1].max(b[3])],
                        rotate: dict.get(b"Rotate").and_then(Obj::as_int).unwrap_or(0).rem_euclid(360),
                    };
                    let ds: Vec<(usize, &Decor)> =
                        s.decor.iter().filter_map(|k| self.decors.get(*k as usize).map(|d| (*k as usize, d))).collect();
                    let index = kids.len() - 1;
                    decorated.push((out.objects.len(), decor::lay_out(&ds, frame, index, order.len())));
                }
            }
            if structural && let Some(k) = s.ocr {
                recognised.push((out.objects.len(), k));
            }
            if structural && let Some(k) = s.items {
                if matches!(s.src, PageSrc::Base(_)) {
                    super::items::strip_saved(&self.base, &mut dict);
                }
                if let Some(list) = self.item_lists.get(k as usize).filter(|l| !l.is_empty()) {
                    itemed.push((out.objects.len(), list.clone()));
                }
            }
            out.objects.push((page_ref, Obj::Dict(dict)));
        }
        if !recognised.is_empty() {
            self.draw_ocr(&recognised, &mut out);
        }
        if !itemed.is_empty() {
            self.draw_items(&itemed, &mut out);
        }
        if !decorated.is_empty() {
            self.draw_decor(&decorated, &mut out);
        }

        // The catalog, when something changes it.
        let mut cat: Option<Dict> = None;
        let page_list: Vec<Ref> = kids.iter().filter_map(Obj::as_ref).collect();
        if structural && let Some(root) = root_ref {
            let mut d = self.base.get(root)?.as_dict().cloned().unwrap_or_default();
            d.set(b"Type", Obj::name("Pages"));
            d.set(b"Kids", Obj::Array(kids));
            d.set(b"Count", Obj::Int(order.len() as i64));
            d.remove(b"Parent");
            out.objects.push((root, Obj::Dict(d)));
            if export {
                // A new file of some pages: no bookmarks or document-wide structure.
                let c = cat.get_or_insert_with(|| catalog.as_dict().cloned().unwrap_or_default());
                for k in [&b"Outlines"[..], b"StructTreeRoot", b"MarkInfo", b"PageLabels", b"OpenAction", b"PageMode"] {
                    c.remove(k);
                }
            } else {
                self.retarget_outline(order, &base_pages, &page_refs, &mut out)?;
                // Page labels are given by position: after pages move they
                // would name the wrong pages, so readers number pages plainly.
                let moved = order.len() != base_pages.len()
                    || order.iter().enumerate().any(|(i, s)| s.key != i as u32 || !s.owner());
                if moved && catalog.as_dict().is_some_and(|c| c.get(b"PageLabels").is_some()) {
                    cat.get_or_insert_with(|| catalog.as_dict().cloned().unwrap_or_default()).remove(b"PageLabels");
                }
            }
        }
        self.field_objects(&mut out, &mut cat, catalog.as_dict())?;
        if self.flattened {
            self.flatten_pages(&mut out, &page_list)?;
            // No fields are left to fill.
            if catalog.as_dict().is_some_and(|c| c.get(b"AcroForm").is_some()) {
                cat.get_or_insert_with(|| catalog.as_dict().cloned().unwrap_or_default()).remove(b"AcroForm");
            }
        }
        if let (Some(cr), Some(c)) = (catalog_ref, cat) {
            out.objects.push((cr, Obj::Dict(c)));
        }
        Ok((out.objects, assigned))
    }

    /// Adds the invisible text layers: one font for all of them, the page's
    /// own content wrapped in q/Q, then the words.
    fn draw_ocr(&self, recognised: &[(usize, u32)], out: &mut Out) {
        let font = super::ocr::font_objects(out);
        let open = out.add(Obj::Stream(Dict::default(), b"q\n".to_vec()));
        for (at, k) in recognised {
            let Some(layer) = self.ocr.get(*k as usize) else { continue };
            let content = out.add(Obj::Stream(Dict::default(), super::ocr::content(layer)));
            let Obj::Dict(dict) = &mut out.objects[*at].1 else { continue };
            let mut contents = vec![Obj::Ref(open)];
            match dict.get(b"Contents") {
                Some(Obj::Array(a)) => contents.extend(a.iter().cloned()),
                Some(c @ Obj::Ref(_)) => contents.push(c.clone()),
                _ => {}
            }
            contents.push(Obj::Ref(content));
            dict.set(b"Contents", Obj::Array(contents));
            let mut res = dict.get(b"Resources").and_then(Obj::as_dict).cloned().unwrap_or_default();
            let mut fonts = res.get(b"Font").and_then(Obj::as_dict).cloned().unwrap_or_default();
            fonts.set(super::ocr::FONT.as_bytes(), Obj::Ref(font));
            res.set(b"Font", Obj::Dict(fonts));
            dict.set(b"Resources", Obj::Dict(res));
        }
    }

    /// Adds the header and footer text to the pages in `decorated`: one
    /// font for all of them, the page's own content wrapped in q/Q, then
    /// the text drawn on top.
    fn draw_decor(&self, decorated: &[(usize, decor::PageText)], out: &mut Out) {
        fn one(b: &decor::Block) -> (&edits_text::Layout, f32, f32, f32, [f32; 3]) {
            (&b.0, b.1, b.2, b.3, b.4)
        }
        let mut parts = Vec::new();
        for (_, p) in decorated {
            parts.push(p.plain.iter().map(one).collect::<Vec<_>>());
            for (b, _, _) in &p.marks {
                parts.push(vec![one(b)]);
            }
        }
        let refs: Vec<&edits_text::Blocks> = parts.iter().map(|p| p.as_slice()).collect();
        let (ops, font) = edits_text::pdf_text_parts(&refs, "EditsF0");
        let font = super::font_objects(&font, &mut |o| out.add(o));
        let open = out.add(Obj::Stream(Dict::default(), b"q\n".to_vec()));
        let mut ops = ops.into_iter();
        for (at, p) in decorated {
            let n = 1 + p.marks.len();
            let mine: Vec<String> = ops.by_ref().take(n).collect();
            let content = out.add(Obj::Stream(Dict::default(), decor::content(p, mine.into_iter())));
            let Obj::Dict(dict) = &mut out.objects[*at].1 else { continue };
            let mut contents = vec![Obj::Ref(open)];
            match dict.get(b"Contents") {
                Some(Obj::Array(a)) => contents.extend(a.iter().cloned()),
                Some(c @ Obj::Ref(_)) => contents.push(c.clone()),
                _ => {}
            }
            contents.push(Obj::Ref(content));
            dict.set(b"Contents", Obj::Array(contents));
            let mut res = dict.get(b"Resources").and_then(Obj::as_dict).cloned().unwrap_or_default();
            let mut fonts = res.get(b"Font").and_then(Obj::as_dict).cloned().unwrap_or_default();
            fonts.set(b"EditsF0", Obj::Ref(font));
            res.set(b"Font", Obj::Dict(fonts));
            if !p.marks.is_empty() {
                let mut gs = res.get(b"ExtGState").and_then(Obj::as_dict).cloned().unwrap_or_default();
                for (_, _, k) in &p.marks {
                    let op = self.decors[*k].watermark_opacity;
                    gs.set(format!("EditsGS{k}").as_bytes(), Obj::Dict(decor::opacity_state(op)));
                }
                res.set(b"ExtGState", Obj::Dict(gs));
            }
            dict.set(b"Resources", Obj::Dict(res));
        }
    }

    /// Bookmarks to pages that were deleted go to the nearest page left.
    fn retarget_outline(
        &self,
        order: &[PageSlot],
        base_pages: &[Ref],
        page_refs: &HashMap<u32, Ref>,
        out: &mut Out,
    ) -> Result<(), PdfError> {
        let kept: BTreeSet<u32> = order.iter().filter(|s| s.owner()).map(|s| s.key).collect();
        if kept.len() == base_pages.len() {
            return Ok(());
        }
        let dead: HashMap<u32, u32> = base_pages
            .iter()
            .enumerate()
            .filter(|(i, _)| !kept.contains(&(*i as u32)))
            .map(|(i, r)| (r.0, i as u32))
            .collect();
        let survivor = |i: u32| -> Option<Ref> {
            let key = kept.range(i..).next().or_else(|| kept.range(..i).next_back()).copied();
            match key {
                Some(k) => page_refs.get(&k).copied(),
                None => order.first().and_then(|s| page_refs.get(&s.key)).copied(),
            }
        };
        let catalog = self.base.resolve(self.base.trailer.get(b"Root").unwrap_or(&Obj::Null))?;
        let Some(first) = catalog
            .as_dict()
            .and_then(|d| d.get(b"Outlines"))
            .map(|o| self.base.resolve(o))
            .transpose()?
            .and_then(|o| o.as_dict().and_then(|d| d.get(b"First")).and_then(Obj::as_ref))
        else {
            return Ok(());
        };
        let mut stack = vec![first];
        let mut seen = HashSet::new();
        while let Some(r) = stack.pop() {
            if !seen.insert(r.0) || seen.len() > 100_000 {
                continue;
            }
            let item = self.base.get(r)?;
            let Some(d) = item.as_dict() else { continue };
            for k in [&b"Next"[..], b"First"] {
                if let Some(n) = d.get(k).and_then(Obj::as_ref) {
                    stack.push(n);
                }
            }
            let action = match d.get(b"A") {
                Some(a) => self.base.resolve(a)?.as_dict().cloned(),
                None => None,
            };
            let dest = match (d.get(b"Dest"), &action) {
                (Some(x), _) => Some(self.base.resolve(x)?),
                (None, Some(a)) if a.name(b"S") == Some(b"GoTo") => {
                    a.get(b"D").map(|x| self.base.resolve(x)).transpose()?
                }
                _ => None,
            };
            let target = dest.as_ref().and_then(Obj::as_array).and_then(|a| a.first()).and_then(Obj::as_ref);
            let Some(gone) = target.and_then(|t| dead.get(&t.0)) else { continue };
            let Some(to) = survivor(*gone) else { continue };
            let mut d = d.clone();
            d.remove(b"A");
            d.set(b"Dest", Obj::Array(vec![Obj::Ref(to), Obj::name("Fit")]));
            out.objects.push((r, Obj::Dict(d)));
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Annot, Kind};

    /// A PDF of `n` pages in a two-level page tree. Page i is 100 + i
    /// points wide and its content says PAGE-i. Page 1 has a square
    /// annotation; every page has a bookmark. The top node holds the
    /// (inherited) resources.
    pub(crate) fn pages_pdf(n: u32) -> Vec<u8> {
        // 1 catalog, 2 root, 3 outlines, then per page: page, content, bookmark; then 2 middle nodes, 1 annot.
        let page = |i: u32| 4 + 3 * i;
        let content = |i: u32| 5 + 3 * i;
        let mark = |i: u32| 6 + 3 * i;
        let mid = [4 + 3 * n, 5 + 3 * n];
        let annot = 6 + 3 * n;
        let link = 7 + 3 * n;
        let half = n.div_ceil(2);
        let mut objs: Vec<(u32, String)> = Vec::new();
        objs.push((
            1,
            "<< /Type /Catalog /Pages 2 0 R /Outlines 3 0 R /PageLabels << /Nums [0 << /S /r >>] >> >>".into(),
        ));
        objs.push((
            2,
            format!(
                "<< /Type /Pages /Kids [{} 0 R {} 0 R] /Count {n} /Resources << /ProcSet [/PDF /Text] >> >>",
                mid[0], mid[1]
            ),
        ));
        objs.push((3, format!("<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {n} >>", mark(0), mark(n - 1))));
        for i in 0..n {
            let parent = if i < half { mid[0] } else { mid[1] };
            let annots = match i {
                0 => format!(" /Annots [{link} 0 R]"),
                1 => format!(" /Annots [{annot} 0 R]"),
                _ => String::new(),
            };
            objs.push((
                page(i),
                format!(
                    "<< /Type /Page /Parent {parent} 0 R /MediaBox [0 0 {} 100] /Contents {} 0 R{annots} >>",
                    100 + i,
                    content(i)
                ),
            ));
            let text = format!("% PAGE-{i}\n");
            objs.push((content(i), format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len())));
            let mut m = format!("<< /Title (Page {i}) /Parent 3 0 R /Dest [{} 0 R /Fit]", page(i));
            if i > 0 {
                m += &format!(" /Prev {} 0 R", mark(i - 1));
            }
            if i + 1 < n {
                m += &format!(" /Next {} 0 R", mark(i + 1));
            }
            objs.push((mark(i), m + " >>"));
        }
        let kids = |r: std::ops::Range<u32>| r.map(|i| format!("{} 0 R", page(i))).collect::<Vec<_>>().join(" ");
        objs.push((mid[0], format!("<< /Type /Pages /Parent 2 0 R /Kids [{}] /Count {half} >>", kids(0..half))));
        objs.push((mid[1], format!("<< /Type /Pages /Parent 2 0 R /Kids [{}] /Count {} >>", kids(half..n), n - half)));
        objs.push((
            annot,
            format!("<< /Type /Annot /Subtype /Square /Rect [10 10 50 50] /C [1 0 0] /P {} 0 R >>", page(1)),
        ));
        objs.push((
            link,
            format!(
                "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /P {} 0 R /Dest [{} 0 R /Fit] >>",
                page(0),
                page(2)
            ),
        ));
        objs.sort_by_key(|o| o.0);
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

    pub(crate) fn temp(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("edits-pages-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// What readers see: per page, its width, rotation, content marker and annotation count.
    fn seen_bytes(bytes: Vec<u8>) -> Vec<(i64, i64, String, usize)> {
        let f = PdfFile::open(bytes).unwrap();
        f.pages()
            .unwrap()
            .iter()
            .map(|r| {
                let d = page_dict(&f, *r).unwrap();
                let w = d.get(b"MediaBox").and_then(Obj::numbers).unwrap()[2] as i64;
                let rot = d.get(b"Rotate").and_then(Obj::as_int).unwrap_or(0);
                let text = match d.get(b"Contents") {
                    Some(c) => match f.resolve(c).unwrap() {
                        Obj::Stream(_, data) => String::from_utf8_lossy(&data).trim().to_string(),
                        _ => String::new(),
                    },
                    None => String::new(),
                };
                let annots = match d.get(b"Annots") {
                    Some(a) => f.resolve(a).unwrap().as_array().map_or(0, <[Obj]>::len),
                    None => 0,
                };
                assert!(d.get(b"Resources").is_some(), "every page has resources");
                (w, rot, text, annots)
            })
            .collect()
    }

    fn seen(s: &Session) -> Vec<(i64, i64, String, usize)> {
        let mut bytes = std::fs::read(s.path()).unwrap();
        if let Some(t) = s.tail() {
            bytes.extend_from_slice(&t);
        }
        seen_bytes(bytes)
    }

    fn widths(v: &[(i64, i64, String, usize)]) -> Vec<i64> {
        v.iter().map(|p| p.0).collect()
    }

    #[test]
    fn pages_move_turn_and_copy_with_their_annotations() {
        let path = temp("move.pdf", &pages_pdf(5));
        let mut s = Session::open(&path).unwrap();
        assert_eq!(widths(&seen(&s)), [100, 101, 102, 103, 104]);

        s.move_pages(&[0, 1], 4).unwrap();
        assert!(s.take_layout_changed());
        assert_eq!(widths(&seen(&s)), [102, 103, 100, 101, 104]);
        // The annotation went with its page, and is listed there.
        s.import(3).unwrap();
        assert_eq!(s.list(3).len(), 1);
        assert_eq!(s.list(3)[0].annot.page, 3);
        assert_eq!(seen(&s)[3].3, 1);

        s.move_pages(&[4], 0).unwrap();
        assert_eq!(widths(&seen(&s)), [104, 102, 103, 100, 101]);
        s.rotate_pages(&[0], 1).unwrap();
        s.rotate_pages(&[0], 2).unwrap();
        assert_eq!(seen(&s)[0].1, 270);

        // A copy is a page of its own, with its own copy of the annotation.
        s.duplicate_pages(&[4]).unwrap();
        let now = seen(&s);
        assert_eq!(widths(&now), [104, 102, 103, 100, 101, 101]);
        assert_eq!(now[5].2, "% PAGE-1");
        assert_eq!((now[4].3, now[5].3), (1, 1));
        s.import(5).unwrap();
        let copy = s.list(5);
        assert_eq!(copy.len(), 1);
        // Changing the copy leaves the original alone.
        let mut a = copy[0].annot.clone();
        a.translate(20.0, 0.0);
        s.change(copy[0].id, a).unwrap();
        assert_eq!(s.list(4)[0].annot.rect[0], 10.0);
        assert_eq!(s.list(5)[0].annot.rect[0], 30.0);

        // Undo walks back through all of it.
        for _ in 0..5 {
            s.undo().unwrap();
        }
        assert_eq!(widths(&seen(&s)), [102, 103, 100, 101, 104]);
        s.undo().unwrap();
        assert!(s.tail().is_none(), "back to the file as it was");
        s.redo().unwrap();
        assert_eq!(widths(&seen(&s)), [102, 103, 100, 101, 104]);

        // Saved without deleting: appended, and read back as shown.
        let shown = seen(&s);
        s.save(&path).unwrap();
        assert!(!s.status().dirty && !s.status().can_undo);
        let saved = seen_bytes(std::fs::read(&path).unwrap());
        assert_eq!(saved, shown);
        assert!(std::fs::read(&path).unwrap().starts_with(&pages_pdf(5)));
    }

    #[test]
    fn deleting_pages_removes_them_from_the_saved_file() {
        let path = temp("delete.pdf", &pages_pdf(4));
        let mut s = Session::open(&path).unwrap();
        s.delete_pages(&[1, 2]).unwrap();
        assert_eq!(widths(&seen(&s)), [100, 103]);
        assert!(s.delete_pages(&[0, 1]).is_err(), "the last page cannot go");
        s.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(widths(&seen_bytes(bytes.clone())), [100, 103]);
        let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"PAGE-0") && has(b"PAGE-3"));
        assert!(!has(b"PAGE-1") && !has(b"PAGE-2"), "deleted pages' content is gone");

        // Their bookmarks now lead to the nearest page left.
        let f = PdfFile::open(bytes).unwrap();
        let pages = f.pages().unwrap();
        let cat = f.resolve(f.trailer.get(b"Root").unwrap()).unwrap();
        let outlines = f.resolve(cat.as_dict().unwrap().get(b"Outlines").unwrap()).unwrap();
        let mut item = outlines.as_dict().unwrap().get(b"First").and_then(Obj::as_ref);
        let mut targets = Vec::new();
        while let Some(r) = item {
            let d = f.get(r).unwrap();
            let d = d.as_dict().unwrap();
            let dest = d.get(b"Dest").and_then(Obj::as_array).unwrap();
            targets.push(pages.iter().position(|p| Some(*p) == dest[0].as_ref()));
            item = d.get(b"Next").and_then(Obj::as_ref);
        }
        assert_eq!(targets, [Some(0), Some(1), Some(1), Some(1)]);
    }

    #[test]
    fn pages_come_in_from_other_files_pictures_and_nothing() {
        let path = temp("insert.pdf", &pages_pdf(2));
        let other = temp("other.pdf", &pages_pdf(3));
        let mut s = Session::open(&path).unwrap();
        assert_eq!(s.insert_pdf(1, &other, Some(vec![2, 1])).unwrap(), 2);
        assert_eq!(widths(&seen(&s)), [100, 102, 101, 101]);
        let now = seen(&s);
        assert_eq!(now[1].2, "% PAGE-2");
        assert_eq!(now[2].3, 1, "the other file's annotation came along");

        s.insert_blank(0, 300.0, 400.0, 2).unwrap();
        let mut png = Vec::new();
        {
            let mut e = png::Encoder::new(&mut png, 2, 1);
            e.set_color(png::ColorType::Rgb);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[0, 128, 255, 255, 0, 0]).unwrap();
        }
        let img = crate::image::PdfImage::from_bytes(&png).unwrap();
        s.insert_images(6, vec![img]).unwrap();
        let now = seen(&s);
        assert_eq!(widths(&now), [300, 300, 100, 102, 101, 101, 842]);

        // Annotations on new pages work like any other.
        let mut a = Annot::new(0, Kind::Square);
        a.rect = [10.0, 10.0, 40.0, 40.0];
        s.add(a).unwrap();
        assert_eq!(seen(&s)[0].3, 1);

        // Survives a crash.
        let json = serde_json::to_string(&s.recovery().unwrap()).unwrap();
        let shown = seen(&s);
        let mut s2 = Session::open(&path).unwrap();
        s2.restore(serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(seen(&s2), shown);

        // Some pages as a new file.
        let out = path.with_file_name("part.pdf");
        s.export(&[3, 0], &out).unwrap();
        let part = std::fs::read(&out).unwrap();
        assert_eq!(widths(&seen_bytes(part.clone())), [102, 300]);
        let f = PdfFile::open(part).unwrap();
        let cat = f.resolve(f.trailer.get(b"Root").unwrap()).unwrap();
        assert!(cat.as_dict().unwrap().get(b"Outlines").is_none());

        s.save(&path).unwrap();
        assert_eq!(widths(&seen_bytes(std::fs::read(&path).unwrap())), [300, 300, 100, 102, 101, 101, 842]);
        assert_eq!(s.page_count().unwrap(), 7);
    }

    #[test]
    fn copies_keep_links_and_reordering_drops_page_labels() {
        let path = temp("links.pdf", &pages_pdf(3));
        let mut s = Session::open(&path).unwrap();
        let catalog = |s: &Session| {
            let mut bytes = std::fs::read(s.path()).unwrap();
            if let Some(t) = s.tail() {
                bytes.extend_from_slice(&t);
            }
            let f = PdfFile::open(bytes).unwrap();
            let c = f.resolve(f.trailer.get(b"Root").unwrap()).unwrap().as_dict().cloned().unwrap();
            (f, c)
        };
        s.rotate_pages(&[2], 1).unwrap();
        assert!(catalog(&s).1.get(b"PageLabels").is_some(), "turning a page keeps labels");
        s.duplicate_pages(&[0]).unwrap();
        let (f, c) = catalog(&s);
        assert!(c.get(b"PageLabels").is_none());
        let pages = f.pages().unwrap();
        let link_of = |i: usize| {
            let (_, _, annots) = f.page_annots(i as u32).unwrap();
            let l = f.resolve(&annots[0]).unwrap();
            let l = l.as_dict().unwrap();
            assert_eq!(l.get(b"P").and_then(Obj::as_ref), Some(pages[i]));
            l.get(b"Dest").and_then(Obj::as_array).unwrap()[0].as_ref()
        };
        assert_eq!(link_of(0), Some(pages[3]));
        assert_eq!(link_of(1), Some(pages[3]), "the copy links to the same page");
    }

    #[test]
    fn numbers_and_watermarks_are_drawn_on_top() {
        let path = temp("numbers.pdf", &pages_pdf(3));
        let mut s = Session::open(&path).unwrap();
        s.rotate_pages(&[1], 1).unwrap();
        let mut d: Decor = serde_json::from_str("{}").unwrap();
        d.footer[1] = "صفحة {n} من {total}".into();
        d.watermark = "DRAFT".into();
        s.decorate_pages(&[0, 1, 2], d).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&s.tail().unwrap());
        let f = PdfFile::open(bytes).unwrap();
        let pages = f.pages().unwrap();
        let d = page_dict(&f, pages[1]).unwrap();
        let contents = d.get(b"Contents").and_then(Obj::as_array).unwrap().to_vec();
        assert_eq!(contents.len(), 3, "q, the page, then the additions");
        let Obj::Stream(_, first) = f.resolve(&contents[0]).unwrap() else { panic!() };
        assert_eq!(first, b"q\n");
        let Obj::Stream(_, last) = f.resolve(&contents[2]).unwrap() else { panic!() };
        let last = String::from_utf8(last).unwrap();
        assert!(last.starts_with("Q\nq\n") && last.contains("/EditsF0") && last.contains("/EditsGS0 gs"));
        let res = d.get(b"Resources").and_then(Obj::as_dict).unwrap();
        assert!(res.get(b"Font").and_then(Obj::as_dict).unwrap().get(b"EditsF0").is_some());
        assert!(res.get(b"ProcSet").is_some(), "the page's own resources are kept");
        // Taken off again before saving: no trace of them left.
        s.undecorate_pages(&[0, 1, 2]).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&s.tail().unwrap());
        assert!(!bytes.windows(7).any(|w| w == b"EditsF0"));
    }

    #[test]
    fn crop_boxes_are_set_and_cleared() {
        let path = temp("crop.pdf", &pages_pdf(2));
        let mut s = Session::open(&path).unwrap();
        s.crop_pages(&[(1, Some([60.0, 90.0, 10.0, 20.0]))]).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&s.tail().unwrap());
        let f = PdfFile::open(bytes).unwrap();
        let d = page_dict(&f, f.pages().unwrap()[1]).unwrap();
        assert_eq!(d.get(b"CropBox").and_then(Obj::numbers).unwrap(), [10.0, 20.0, 60.0, 90.0]);
        s.crop_pages(&[(1, None)]).unwrap();
        assert!(s.tail().is_none(), "uncropped is the file as it was");
    }

    #[test]
    fn recognised_text_is_one_step_per_run() {
        use crate::session::{OcrLayer, OcrWord};
        let path = temp("ocr.pdf", &pages_pdf(3));
        let mut s = Session::open(&path).unwrap();
        let layer = |t: &str| OcrLayer {
            words: vec![OcrWord { quad: [[10.0, 50.0], [60.0, 50.0], [60.0, 40.0], [10.0, 40.0]], shown: t.into() }],
        };
        s.set_page_text(7, vec![(0, layer("ab "))]).unwrap();
        s.set_page_text(7, vec![(2, layer("cd "))]).unwrap();
        assert!(!s.take_layout_changed(), "pages keep their place");
        assert!(s.has_page_text(0) && !s.has_page_text(1) && s.has_page_text(2));
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(&s.tail().unwrap());
        let f = PdfFile::open(bytes).unwrap();
        let pages = f.pages().unwrap();
        let d = page_dict(&f, pages[2]).unwrap();
        let contents = d.get(b"Contents").and_then(Obj::as_array).unwrap().to_vec();
        let Obj::Stream(_, last) = f.resolve(contents.last().unwrap()).unwrap() else { panic!() };
        let last = String::from_utf8(last).unwrap();
        assert!(last.contains("3 Tr") && last.contains("[<0063> <0064> <0020>] TJ"), "{last}");
        let res = d.get(b"Resources").and_then(Obj::as_dict).unwrap();
        assert!(res.get(b"Font").and_then(Obj::as_dict).unwrap().get(b"EditsOcr").is_some());
        assert!(res.get(b"ProcSet").is_some(), "inherited resources are kept");
        // One undo takes both pages' text away.
        s.undo().unwrap();
        assert!(!s.has_page_text(0) && !s.has_page_text(2));
        assert!(s.tail().is_none());
        // A new run is a new step.
        s.redo().unwrap();
        s.set_page_text(8, vec![(1, layer("e "))]).unwrap();
        s.undo().unwrap();
        assert!(s.has_page_text(0) && !s.has_page_text(1));
    }

    #[test]
    fn pictures_become_a_new_pdf() {
        // A 2 x 1 pixel PNG (landscape) and a 1 x 2 one.
        let png = |w: u32, h: u32| {
            let mut out = Vec::new();
            {
                let mut e = png::Encoder::new(&mut out, w, h);
                e.set_color(png::ColorType::Grayscale);
                let mut wr = e.write_header().unwrap();
                wr.write_image_data(&vec![128u8; (w * h) as usize]).unwrap();
            }
            PdfImage::from_bytes(&out).unwrap()
        };
        let dir = std::env::temp_dir().join(format!("edits-pages-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("pictures.pdf");
        images_to_pdf(vec![png(2, 1), png(1, 2)], Some((595.0, 842.0)), &target).unwrap();
        let f = PdfFile::open(std::fs::read(&target).unwrap()).unwrap();
        let pages = f.pages().unwrap();
        assert_eq!(pages.len(), 2, "the empty base page is left out");
        let size = |i: usize| page_dict(&f, pages[i]).unwrap().get(b"MediaBox").and_then(Obj::numbers).unwrap();
        assert_eq!(size(0), [0.0, 0.0, 842.0, 595.0], "turned for a wide picture");
        assert_eq!(size(1), [0.0, 0.0, 595.0, 842.0]);
        assert!(!super::super::temp_path_for(&target).with_extension("edits-base").exists());
    }
}
