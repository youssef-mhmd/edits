//! Editing commands: annotations, undo and redo, save (PRD 8.4).
//!
//! Each open document has its edits and history here. A change goes to the
//! history first, then to PDFium's copy of the document so pages render
//! with it, and the front end is told which pages to draw again.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use edits_core::{DocId, Engine, EngineError};
use edits_edit::{Annot, AnnotId, Edits, Kind, LayerOp, Listed, SaveError, Status};
use serde::Serialize;

struct DocEdits {
    edits: Edits,
    path: PathBuf,
}

/// Edits of every open document.
#[derive(Default)]
pub struct EditState(Mutex<HashMap<DocId, DocEdits>>);

impl EditState {
    pub fn opened(&self, doc: DocId, path: PathBuf) {
        self.lock().insert(doc, DocEdits { edits: Edits::new(), path });
    }

    pub fn closed(&self, doc: DocId) {
        self.lock().remove(&doc);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<DocId, DocEdits>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The result of a change: the new undo state and the pages to draw again.
#[derive(Serialize)]
pub struct Changed {
    pub status: Status,
    pub pages: Vec<u32>,
    /// The annotation a new one got, if one was added.
    pub id: Option<AnnotId>,
}

fn unknown(doc: DocId) -> String {
    EngineError::UnknownDoc(doc).to_string()
}

/// Sends layer operations to the engine and records where created
/// annotations went. Returns the pages they touch.
fn apply(engine: &Engine, doc: DocId, edits: &mut Edits, ops: Vec<LayerOp>) -> Result<Vec<u32>, String> {
    let mut pages: Vec<u32> = ops
        .iter()
        .map(|op| match op {
            LayerOp::Create { annot, .. } => annot.page,
            LayerOp::SetHidden { page, .. } => *page,
        })
        .collect();
    pages.sort_unstable();
    pages.dedup();
    if ops.is_empty() {
        return Ok(pages);
    }
    for (id, r) in engine.edit_layer(doc, ops).map_err(|e| e.to_string())? {
        match r {
            Ok(index) => edits.set_layer(id, index),
            Err(e) => {
                log::warn!("annotation {id} could not be shown: {e}");
                edits.clear_layer(id);
            }
        }
    }
    Ok(pages)
}

/// Runs `f` on the document's edits, off the interface thread.
async fn with_edits<R: Send + 'static>(
    app: tauri::AppHandle,
    doc: DocId,
    f: impl FnOnce(&Engine, &mut DocEdits) -> Result<R, String> + Send + 'static,
) -> Result<R, String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<crate::AppState>();
        let edit = app.state::<EditState>();
        let mut all = edit.lock();
        let d = all.get_mut(&doc).ok_or_else(|| unknown(doc))?;
        f(&state.engine, d)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Adds text markup over `rects` (page fractions: left, top, right, bottom,
/// one per line of the selection).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn annot_markup(
    app: tauri::AppHandle,
    doc: DocId,
    page: u32,
    kind: Kind,
    rects: Vec<[f32; 4]>,
    color: [f32; 3],
    opacity: f32,
    author: String,
) -> Result<Changed, String> {
    with_edits(app, doc, move |engine, d| {
        if rects.is_empty() {
            return Err("nothing selected".into());
        }
        let points: Vec<[f32; 2]> =
            rects.iter().flat_map(|r| [[r[0], r[1]], [r[2], r[1]], [r[0], r[3]], [r[2], r[3]]]).collect();
        let user = engine.to_user_space(doc, page, points).map_err(|e| e.to_string())?;
        let quads: Vec<[f32; 8]> =
            user.chunks(4).map(|c| [c[0][0], c[0][1], c[1][0], c[1][1], c[2][0], c[2][1], c[3][0], c[3][1]]).collect();
        let annot = Annot {
            page,
            kind,
            rect: Annot::quads_bounds(&quads, 1.0),
            quads,
            color: color.map(|c| c.clamp(0.0, 1.0)),
            opacity: opacity.clamp(0.05, 1.0),
            contents: String::new(),
            author,
            modified: edits_edit::pdf_date_now(),
        };
        let (id, ops) = d.edits.add(annot);
        let pages = apply(engine, doc, &mut d.edits, ops)?;
        Ok(Changed { status: d.edits.status(), pages, id: Some(id) })
    })
    .await
}

#[tauri::command]
pub async fn annot_remove(app: tauri::AppHandle, doc: DocId, id: AnnotId) -> Result<Changed, String> {
    with_edits(app, doc, move |engine, d| {
        let ops = d.edits.remove(id);
        let pages = apply(engine, doc, &mut d.edits, ops)?;
        Ok(Changed { status: d.edits.status(), pages, id: None })
    })
    .await
}

#[tauri::command]
pub async fn edit_undo(app: tauri::AppHandle, doc: DocId) -> Result<Changed, String> {
    with_edits(app, doc, move |engine, d| {
        let ops = d.edits.undo();
        let pages = apply(engine, doc, &mut d.edits, ops)?;
        Ok(Changed { status: d.edits.status(), pages, id: None })
    })
    .await
}

#[tauri::command]
pub async fn edit_redo(app: tauri::AppHandle, doc: DocId) -> Result<Changed, String> {
    with_edits(app, doc, move |engine, d| {
        let ops = d.edits.redo();
        let pages = apply(engine, doc, &mut d.edits, ops)?;
        Ok(Changed { status: d.edits.status(), pages, id: None })
    })
    .await
}

/// Annotations edits added on `page`, for selecting them.
#[tauri::command]
pub async fn annots_on_page(app: tauri::AppHandle, doc: DocId, page: u32) -> Result<Vec<Listed>, String> {
    with_edits(app, doc, move |_, d| Ok(d.edits.on_page(page))).await
}

#[derive(Serialize)]
pub struct Saved {
    pub status: Status,
    pub path: String,
    pub ms: f64,
    /// True when the file had to be rewritten whole (encrypted files).
    pub full: bool,
}

/// Saves the document, to its own file or with `path` to a new one (Save As).
/// The result is written beside the target first and then replaces it.
#[tauri::command]
pub async fn save_doc(app: tauri::AppHandle, doc: DocId, path: Option<String>) -> Result<Saved, String> {
    with_edits(app, doc, move |engine, d| {
        let t = std::time::Instant::now();
        let target = path.map(PathBuf::from).unwrap_or_else(|| d.path.clone());
        let tmp = edits_edit::save::temp_path_for(&target);
        let mut full = false;
        match edits_edit::save_incremental(&d.path, &tmp, &d.edits.changes()) {
            Ok(outcome) => {
                edits_edit::save::replace(&tmp, &target).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    format!("could not replace {}: {e}", target.display())
                })?;
                engine.reload(doc, Some(target.clone())).map_err(|e| e.to_string())?;
                d.edits.saved(&outcome);
            }
            Err(SaveError::Encrypted) => {
                // PDFium rewrites the whole file, keeping its encryption.
                engine.save_full(doc, tmp.clone()).map_err(|e| e.to_string())?;
                edits_edit::save::replace(&tmp, &target).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    format!("could not replace {}: {e}", target.display())
                })?;
                engine.reload(doc, Some(target.clone())).map_err(|e| e.to_string())?;
                // Annotations are now plain file content; history starts again.
                d.edits = Edits::new();
                full = true;
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e.to_string());
            }
        }
        d.path = target.clone();
        Ok(Saved {
            status: d.edits.status(),
            path: target.display().to_string(),
            ms: t.elapsed().as_secs_f64() * 1000.0,
            full,
        })
    })
    .await
}

#[tauri::command]
pub async fn edit_status(app: tauri::AppHandle, doc: DocId) -> Result<Status, String> {
    with_edits(app, doc, move |_, d| Ok(d.edits.status())).await
}

/// The Windows user name, as the default author of annotations.
#[tauri::command]
pub fn user_name() -> String {
    std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default()
}
