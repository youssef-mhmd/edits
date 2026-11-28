//! Editing commands: annotations, undo and redo, save (PRD 8.4).
//!
//! Each open document has an editing session (`edits_edit::Session`): its
//! annotations, history, and the incremental update they make. After every
//! change the engine shows the file with that update on top, and the front
//! end is told which pages to draw again. Geometry crosses to the front end
//! in page fractions (top-left origin, /Rotate applied).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use edits_core::{DocId, Engine, EngineError};
use edits_edit::{Annot, AnnotId, Kind, Session, Status};
use serde::{Deserialize, Serialize};

/// A document's session, or why it cannot be edited.
type Slot = Result<Session, String>;

/// Editing sessions of every open document.
#[derive(Default)]
pub struct EditState(Mutex<HashMap<DocId, Slot>>, Mutex<HashMap<DocId, edits_edit::Recovery>>);

/// Folder for crash-recovery files (set at start-up).
static RECOVERY_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn set_recovery_dir(dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    let _ = RECOVERY_DIR.set(dir);
}

/// The recovery file for `path` (one per document, named by a hash of its path).
fn recovery_file(path: &std::path::Path) -> Option<PathBuf> {
    let key = path.display().to_string().to_lowercase();
    let h = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    RECOVERY_DIR.get().map(|d| d.join(format!("{h:016x}.json")))
}

/// Writes the session's unsaved work, or removes the file when there is none.
fn persist(s: &Session) {
    let Some(file) = recovery_file(s.path()) else { return };
    match s.recovery() {
        Some(r) => {
            let Ok(text) = serde_json::to_vec(&r) else { return };
            let tmp = file.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &file);
            }
        }
        None => {
            let _ = std::fs::remove_file(&file);
        }
    }
}

fn forget_recovery(path: &std::path::Path) {
    if let Some(f) = recovery_file(path) {
        let _ = std::fs::remove_file(f);
    }
}

impl EditState {
    /// Starts a session for a newly opened document. `pages` is the page
    /// count the renderer found; editing is refused if edits reads the file
    /// differently (damaged files), so changes never land on the wrong page.
    pub fn opened(&self, doc: DocId, path: PathBuf, pages: u32, password: &str) {
        let slot = match Session::open_with_password(&path, password) {
            Ok(s) if s.page_count().ok() == Some(pages) => Ok(s),
            Ok(_) => {
                Err("this file is damaged, so it cannot be changed. Save a copy with Save As, then edit the copy."
                    .into())
            }
            Err(edits_edit::pdf::PdfError::Encrypted) => {
                Err("this file's protection could not be opened for changes".into())
            }
            Err(e) => Err(format!("this file cannot be changed: {e}")),
        };
        // Unsaved work from a session that ended without saving.
        if let (Ok(s), Some(file)) = (&slot, recovery_file(&path)) {
            let r = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<edits_edit::Recovery>(&b).ok());
            match r {
                Some(r) if s.can_restore(&r) => {
                    self.1.lock().unwrap_or_else(|e| e.into_inner()).insert(doc, r);
                }
                _ => {
                    let _ = std::fs::remove_file(file);
                }
            }
        }
        self.lock().insert(doc, slot);
    }

    /// The document was closed: saved, or its changes given up.
    pub fn closed(&self, doc: DocId) {
        self.1.lock().unwrap_or_else(|e| e.into_inner()).remove(&doc);
        if let Some(Ok(s)) = self.lock().remove(&doc) {
            forget_recovery(s.path());
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<DocId, Slot>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The result of a change: the new undo state and the pages to draw again.
#[derive(Serialize)]
pub struct Changed {
    pub status: Status,
    pub pages: Vec<u32>,
    /// The annotation a new one got, or the one changed.
    pub id: Option<AnnotId>,
    /// The new page list, when pages were added, removed, moved, turned or
    /// cropped: the document is shown anew.
    pub info: Option<edits_core::DocInfo>,
}

/// An annotation for the front end, geometry in page fractions.
#[derive(Serialize)]
pub struct View {
    pub id: AnnotId,
    pub page: u32,
    pub kind: Kind,
    pub subtype: String,
    /// Left, top, right, bottom.
    pub rect: [f32; 4],
    pub line: Option<[f32; 4]>,
    pub color: [f32; 3],
    pub fill: Option<[f32; 3]>,
    pub opacity: f32,
    pub width: f32,
    pub arrow: bool,
    pub contents: String,
    pub author: String,
    pub modified: String,
    pub font_size: f32,
    pub label: String,
    /// A picture stamp.
    pub picture: bool,
}

/// A new annotation from the front end, geometry in page fractions.
#[derive(Deserialize)]
pub struct NewAnnot {
    pub page: u32,
    pub kind: Kind,
    /// Text markup: one rectangle (left, top, right, bottom) per line.
    #[serde(default)]
    pub rects: Vec<[f32; 4]>,
    /// Freehand strokes.
    #[serde(default)]
    pub ink: Vec<Vec<[f32; 2]>>,
    /// Lines: x1, y1, x2, y2.
    #[serde(default)]
    pub line: Option<[f32; 4]>,
    /// Rectangles and ellipses: left, top, right, bottom.
    #[serde(default)]
    pub rect: Option<[f32; 4]>,
    pub color: [f32; 3],
    #[serde(default)]
    pub fill: Option<[f32; 3]>,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default = "one")]
    pub width: f32,
    #[serde(default)]
    pub arrow: bool,
    #[serde(default)]
    pub author: String,
    /// Notes and text boxes.
    #[serde(default)]
    pub contents: String,
    #[serde(default = "twelve")]
    pub font_size: f32,
    /// A stamp's word.
    #[serde(default)]
    pub label: String,
    /// A picture file for a picture stamp.
    #[serde(default)]
    pub image_path: Option<String>,
    /// Or the picture itself, as a PNG or JPEG data URL (signatures).
    #[serde(default)]
    pub image_data: Option<String>,
}

fn twelve() -> f32 {
    12.0
}

fn one() -> f32 {
    1.0
}

/// Changes to an annotation's look; absent fields stay as they are.
#[derive(Deserialize)]
pub struct Style {
    pub color: Option<[f32; 3]>,
    /// `Some(None)` removes the fill.
    #[serde(default, with = "serde_with_none")]
    pub fill: Option<Option<[f32; 3]>>,
    pub width: Option<f32>,
    pub opacity: Option<f32>,
    pub contents: Option<String>,
    pub font_size: Option<f32>,
}

/// Deserialises a field that may be absent, null, or a value.
mod serde_with_none {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(d).map(Some)
    }
}

fn user(engine: &Engine, doc: DocId, page: u32, pts: Vec<[f32; 2]>) -> Result<Vec<[f32; 2]>, String> {
    engine.to_user_space(doc, page, pts).map_err(|e| e.to_string())
}

/// A rectangle in fractions (left, top, right, bottom) to user space (left, bottom, right, top).
fn user_rect(engine: &Engine, doc: DocId, page: u32, r: [f32; 4]) -> Result<[f32; 4], String> {
    let p = user(engine, doc, page, vec![[r[0], r[1]], [r[2], r[3]]])?;
    Ok([p[0][0].min(p[1][0]), p[0][1].min(p[1][1]), p[0][0].max(p[1][0]), p[0][1].max(p[1][1])])
}

fn view(engine: &Engine, doc: DocId, id: AnnotId, a: &Annot) -> Result<View, String> {
    let mut pts = vec![[a.rect[0], a.rect[3]], [a.rect[2], a.rect[1]], [a.rect[0], a.rect[1]], [a.rect[2], a.rect[3]]];
    if let Some(l) = a.line {
        pts.push([l[0], l[1]]);
        pts.push([l[2], l[3]]);
    }
    let f = engine.to_fractions(doc, a.page, pts).map_err(|e| e.to_string())?;
    // With /Rotate, any corner can end up top-left.
    let xs = f[..4].iter().map(|p| p[0]);
    let ys = f[..4].iter().map(|p| p[1]);
    let rect = [
        xs.clone().fold(f32::MAX, f32::min),
        ys.clone().fold(f32::MAX, f32::min),
        xs.fold(f32::MIN, f32::max),
        ys.fold(f32::MIN, f32::max),
    ];
    Ok(View {
        id,
        page: a.page,
        kind: a.kind,
        subtype: a.subtype.clone(),
        rect,
        line: a.line.map(|_| [f[4][0], f[4][1], f[5][0], f[5][1]]),
        color: a.color,
        fill: a.fill,
        opacity: a.opacity,
        width: a.width,
        arrow: a.arrow,
        contents: a.contents.clone(),
        author: a.author.clone(),
        modified: a.modified.clone(),
        font_size: a.font_size,
        label: a.label.clone(),
        picture: a.image.is_some(),
    })
}

/// Runs `f` on the document's session, off the interface thread, then shows
/// the result: the engine reopens the document with the new update.
pub(crate) async fn with_session<R: Send + 'static>(
    app: tauri::AppHandle,
    doc: DocId,
    f: impl FnOnce(&Engine, &mut Session) -> Result<R, String> + Send + 'static,
) -> Result<R, String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::AppState>();
        let edit = app.state::<EditState>();
        let mut all = edit.lock();
        let slot = all.get_mut(&doc).ok_or_else(|| EngineError::UnknownDoc(doc).to_string())?;
        let s = slot.as_mut().map_err(|e| e.clone())?;
        f(&state.engine, s)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn show(engine: &Engine, doc: DocId, s: &Session) -> Result<(), String> {
    engine.set_overlay(doc, s.tail()).map_err(|e| e.to_string())
}

pub(crate) fn changed(
    engine: &Engine,
    doc: DocId,
    s: &mut Session,
    pages: Vec<u32>,
    id: Option<AnnotId>,
) -> Result<Changed, String> {
    let info = if s.take_layout_changed() {
        Some(engine.set_overlay_pages(doc, s.tail()).map_err(|e| e.to_string())?)
    } else {
        show(engine, doc, s)?;
        None
    };
    persist(s);
    Ok(Changed { status: s.status(), pages, id, info })
}

/// The annotations on `page` (comments, not links or form fields).
#[tauri::command]
pub async fn annots_page(app: tauri::AppHandle, doc: DocId, page: u32) -> Result<Vec<View>, String> {
    with_session(app, doc, move |engine, s| {
        s.import(page).map_err(|e| e.to_string())?;
        s.list(page).iter().map(|l| view(engine, doc, l.id, &l.annot)).collect()
    })
    .await
}

/// The annotations of every page, for the Comments panel.
#[tauri::command]
pub async fn annots_all(app: tauri::AppHandle, doc: DocId) -> Result<Vec<View>, String> {
    with_session(app, doc, move |engine, s| {
        let pages = s.page_count().map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for page in 0..pages {
            // A page that cannot be read lists nothing rather than failing the rest.
            if s.import(page).is_err() {
                continue;
            }
            for l in s.list(page) {
                out.push(view(engine, doc, l.id, &l.annot)?);
            }
        }
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn annot_add(app: tauri::AppHandle, doc: DocId, annot: NewAnnot) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        let n = annot;
        let mut a = Annot::new(n.page, n.kind);
        a.color = n.color.map(|c| c.clamp(0.0, 1.0));
        a.fill = n.fill.map(|f| f.map(|c| c.clamp(0.0, 1.0)));
        a.opacity = n.opacity.clamp(0.05, 1.0);
        a.width = n.width.clamp(0.1, 50.0);
        a.arrow = n.arrow;
        a.author = n.author;
        a.modified = edits_edit::pdf_date_now();
        match n.kind {
            k if k.is_markup() => {
                if n.rects.is_empty() {
                    return Err("nothing selected".into());
                }
                let pts =
                    n.rects.iter().flat_map(|r| [[r[0], r[1]], [r[2], r[1]], [r[0], r[3]], [r[2], r[3]]]).collect();
                let u = user(engine, doc, n.page, pts)?;
                a.quads = u
                    .chunks(4)
                    .map(|c| [c[0][0], c[0][1], c[1][0], c[1][1], c[2][0], c[2][1], c[3][0], c[3][1]])
                    .collect();
            }
            Kind::Ink => {
                for stroke in n.ink.into_iter().filter(|s| !s.is_empty()) {
                    a.ink.push(user(engine, doc, n.page, stroke)?);
                }
                if a.ink.is_empty() {
                    return Err("nothing drawn".into());
                }
            }
            Kind::Line => {
                let l = n.line.ok_or("no line")?;
                let u = user(engine, doc, n.page, vec![[l[0], l[1]], [l[2], l[3]]])?;
                a.line = Some([u[0][0], u[0][1], u[1][0], u[1][1]]);
            }
            Kind::Square | Kind::Circle => {
                a.rect = user_rect(engine, doc, n.page, n.rect.ok_or("no rectangle")?)?;
            }
            Kind::Note => {
                // The icon's top-left corner at the point clicked.
                let r = n.rect.ok_or("no place")?;
                let p = user(engine, doc, n.page, vec![[r[0], r[1]]])?[0];
                let s = edits_edit::model::NOTE_SIZE;
                a.rect = [p[0], p[1] - s, p[0] + s, p[1]];
                a.contents = n.contents;
            }
            Kind::FreeText => {
                if n.contents.trim().is_empty() {
                    return Err("the text box is empty".into());
                }
                let mut r = user_rect(engine, doc, n.page, n.rect.ok_or("no place")?)?;
                if r[2] - r[0] < 20.0 {
                    // A click, not a drag: a box 200 points wide.
                    r[2] = r[0] + 200.0;
                }
                a.rect = r;
                a.contents = n.contents;
                a.font_size = n.font_size.clamp(4.0, 144.0);
                a.width = 0.0;
                a.fit_text();
            }
            Kind::Stamp => {
                let r = n.rect.ok_or("no place")?;
                let p = user(engine, doc, n.page, vec![[r[0], r[1]], [r[2], r[3]], [0.0, 0.0], [1.0, 1.0]])?;
                let picture = match (n.image_path, n.image_data) {
                    (Some(path), _) => Some(std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?),
                    (None, Some(url)) => {
                        let b64 = url.split_once(',').map_or(url.as_str(), |(_, d)| d);
                        Some(edits_edit::image::decode_base64(b64).ok_or("the picture could not be read")?)
                    }
                    _ => None,
                };
                if let Some(bytes) = picture {
                    let img = edits_edit::image::PdfImage::from_bytes(&bytes)?;
                    // A dragged box, or a third of the page wide around the click.
                    let (pw, ph) = ((p[3][0] - p[2][0]).abs(), (p[3][1] - p[2][1]).abs());
                    let dragged = (p[1][0] - p[0][0]).abs() > 10.0;
                    let w = if dragged { (p[1][0] - p[0][0]).abs() } else { (pw / 3.0).min(img.width as f32 * 0.75) };
                    let h = w * img.height as f32 / img.width as f32;
                    let (cx, cy) = if dragged {
                        ((p[0][0] + p[1][0]) / 2.0, (p[0][1] + p[1][1]) / 2.0)
                    } else {
                        (p[0][0], p[0][1])
                    };
                    let h = h.min(ph);
                    a.rect = [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0];
                    a.image = Some(std::sync::Arc::new(img));
                } else {
                    if n.label.trim().is_empty() {
                        return Err("the stamp has no word".into());
                    }
                    a.label = n.label;
                    a.font_size = n.font_size.clamp(4.0, 144.0);
                    a.fit_label(p[0][0], p[0][1]);
                }
            }
            _ => return Err("this kind of annotation cannot be added yet".into()),
        }
        a.fit_rect();
        let (id, page) = s.add(a).map_err(|e| e.to_string())?;
        changed(engine, doc, s, vec![page], Some(id))
    })
    .await
}

/// Moves an annotation by `dx`, `dy` in page fractions.
#[tauri::command]
pub async fn annot_move(app: tauri::AppHandle, doc: DocId, id: AnnotId, dx: f32, dy: f32) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        let mut a = s.get(id).ok_or("that annotation is gone")?;
        let u = user(engine, doc, a.page, vec![[0.0, 0.0], [dx, dy]])?;
        a.translate(u[1][0] - u[0][0], u[1][1] - u[0][1]);
        a.modified = edits_edit::pdf_date_now();
        let pages = s.change(id, a).map_err(|e| e.to_string())?;
        changed(engine, doc, s, pages, Some(id))
    })
    .await
}

/// Fits an annotation into `rect` (page fractions: left, top, right, bottom).
#[tauri::command]
pub async fn annot_resize(app: tauri::AppHandle, doc: DocId, id: AnnotId, rect: [f32; 4]) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        let mut a = s.get(id).ok_or("that annotation is gone")?;
        let to = user_rect(engine, doc, a.page, rect)?;
        if to[2] - to[0] < 1.0 || to[3] - to[1] < 1.0 {
            return Err("too small".into());
        }
        a.resize(to);
        if matches!(a.kind, Kind::Ink | Kind::Line) {
            // Lines and ink were scaled; their /Rect follows the strokes.
            a.fit_rect();
        }
        if a.kind == Kind::FreeText {
            // The text wraps to the new width; the height follows the text.
            a.own_ap = true;
            a.fit_text();
        }
        if a.kind == Kind::Stamp && a.drawable() {
            a.own_ap = true;
        }
        a.modified = edits_edit::pdf_date_now();
        let pages = s.change(id, a).map_err(|e| e.to_string())?;
        changed(engine, doc, s, pages, Some(id))
    })
    .await
}

/// Changes an annotation's look or note.
#[tauri::command]
pub async fn annot_style(app: tauri::AppHandle, doc: DocId, id: AnnotId, style: Style) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        let mut a = s.get(id).ok_or("that annotation is gone")?;
        let looks = style.color.is_some() || style.fill.is_some() || style.width.is_some() || style.opacity.is_some();
        if looks {
            if !a.kind.drawable() {
                return Err("edits cannot redraw this kind of annotation; it can be moved or deleted".into());
            }
            // From now on edits draws it.
            a.own_ap = true;
        }
        if let Some(c) = style.color {
            a.color = c.map(|v| v.clamp(0.0, 1.0));
        }
        if let Some(f) = style.fill {
            a.fill = f.map(|f| f.map(|v| v.clamp(0.0, 1.0)));
        }
        if let Some(w) = style.width {
            a.width = w.clamp(0.1, 50.0);
            a.fit_rect();
        }
        if let Some(o) = style.opacity {
            a.opacity = o.clamp(0.05, 1.0);
        }
        if let Some(c) = style.contents {
            if a.kind == Kind::FreeText && c != a.contents {
                a.own_ap = true;
            }
            a.contents = c;
        }
        if let Some(size) = style.font_size {
            a.font_size = size.clamp(4.0, 144.0);
            if a.kind == Kind::Stamp && !a.label.is_empty() {
                a.own_ap = true;
                a.fit_label(a.rect[0], a.rect[3]);
            }
            if a.kind == Kind::FreeText {
                a.own_ap = true;
            }
        }
        if a.kind == Kind::FreeText && a.own_ap {
            a.fit_text();
        }
        a.modified = edits_edit::pdf_date_now();
        let pages = s.change(id, a).map_err(|e| e.to_string())?;
        changed(engine, doc, s, pages, Some(id))
    })
    .await
}

#[tauri::command]
pub async fn annot_remove(app: tauri::AppHandle, doc: DocId, id: AnnotId) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        let page = s.remove(id).map_err(|e| e.to_string())?;
        changed(engine, doc, s, vec![page], None)
    })
    .await
}

#[tauri::command]
pub async fn edit_undo(app: tauri::AppHandle, doc: DocId) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        let pages = s.undo().map_err(|e| e.to_string())?;
        changed(engine, doc, s, pages, None)
    })
    .await
}

#[tauri::command]
pub async fn edit_redo(app: tauri::AppHandle, doc: DocId) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        let pages = s.redo().map_err(|e| e.to_string())?;
        changed(engine, doc, s, pages, None)
    })
    .await
}

#[derive(Serialize)]
pub struct Saved {
    pub status: Status,
    pub path: String,
    pub ms: f64,
}

/// Saves the document, to its own file or with `path` to a new one (Save As).
#[tauri::command]
pub async fn save_doc(app: tauri::AppHandle, doc: DocId, path: Option<String>) -> Result<Saved, String> {
    with_session(app, doc, move |engine, s| {
        let t = std::time::Instant::now();
        let target = path.map(PathBuf::from).unwrap_or_else(|| s.path().to_path_buf());
        let before = s.path().to_path_buf();
        s.save(&target).map_err(|e| format!("could not save {}: {e}", target.display()))?;
        // The saved file shows the pages as they were shown.
        s.take_layout_changed();
        forget_recovery(&before);
        persist(s);
        engine.reload(doc, Some(target.clone())).map_err(|e| e.to_string())?;
        Ok(Saved { status: s.status(), path: target.display().to_string(), ms: t.elapsed().as_secs_f64() * 1000.0 })
    })
    .await
}

#[tauri::command]
pub async fn edit_status(app: tauri::AppHandle, doc: DocId) -> Result<Status, String> {
    with_session(app, doc, move |_, s| Ok(s.status())).await
}

/// When unsaved work for `doc` was kept after edits last closed without
/// saving it (Unix ms), or null.
#[tauri::command]
pub fn recovery_offer(edits: tauri::State<'_, EditState>, doc: DocId) -> Option<u64> {
    edits.1.lock().unwrap_or_else(|e| e.into_inner()).get(&doc).map(|r| r.written as u64)
}

/// Brings back the unsaved work offered by `recovery_offer`.
#[tauri::command]
pub async fn recovery_restore(app: tauri::AppHandle, doc: DocId) -> Result<Changed, String> {
    use tauri::Manager;
    let r = app.state::<EditState>().1.lock().unwrap_or_else(|e| e.into_inner()).remove(&doc);
    let r = r.ok_or("nothing to restore")?;
    with_session(app, doc, move |engine, s| {
        let pages = s.restore(r).map_err(|e| e.to_string())?;
        changed(engine, doc, s, pages, None)
    })
    .await
}

/// Gives up kept or current unsaved work for `doc` (the reader chose not to keep it).
#[tauri::command]
pub fn recovery_discard(edits: tauri::State<'_, EditState>, doc: DocId) {
    edits.1.lock().unwrap_or_else(|e| e.into_inner()).remove(&doc);
    if let Some(Ok(s)) = edits.lock().get(&doc) {
        forget_recovery(s.path());
    }
}

/// Draws every annotation and form field into the pages (undoable until saved).
#[tauri::command]
pub async fn flatten_doc(app: tauri::AppHandle, doc: DocId) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        s.flatten().map_err(|e| e.to_string())?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

/// The Windows user name, as the default author of annotations.
#[tauri::command]
pub fn user_name() -> String {
    std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default()
}

/// Gives pages the text recognised on them (one undo step per OCR run) and
/// shows it, if the page list is still `keys`. Returns the new undo state.
pub(crate) fn commit_ocr(
    app: &tauri::AppHandle,
    doc: DocId,
    keys: &[u32],
    run: u64,
    layers: Vec<(u32, edits_edit::OcrLayer)>,
) -> Result<Status, String> {
    use tauri::Manager;
    let state = app.state::<crate::AppState>();
    let edit = app.state::<EditState>();
    let mut all = edit.lock();
    let s = all.get_mut(&doc).ok_or("the document was closed")?.as_mut().map_err(|e| e.clone())?;
    if s.page_keys() != keys {
        return Err("the pages changed while their text was being read".into());
    }
    let pages: Vec<u32> = layers.iter().map(|l| l.0).collect();
    s.set_page_text(run, layers).map_err(|e| e.to_string())?;
    state.engine.set_overlay_text(doc, s.tail(), pages).map_err(|e| e.to_string())?;
    persist(s);
    Ok(s.status())
}

/// The page keys of a document now, and whether it can be changed.
pub(crate) fn page_keys(app: &tauri::AppHandle, doc: DocId) -> Result<Vec<u32>, String> {
    use tauri::Manager;
    let edit = app.state::<EditState>();
    let all = edit.lock();
    let s = all.get(&doc).ok_or("the document was closed")?.as_ref().map_err(|e| e.clone())?;
    Ok(s.page_keys())
}
