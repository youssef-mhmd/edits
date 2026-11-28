//! Page items (PRD 5.6): text boxes, pictures, shapes, lines and tables
//! placed on pages as page content. The front end edits a page's whole list
//! and sends it back; geometry crosses as page fractions (left, top, right,
//! bottom, /Rotate applied), as for annotations.

use edits_core::{DocId, Engine};
use edits_edit::{Item, ItemKind};
use serde::{Deserialize, Serialize};

use crate::edit::{Changed, changed, with_session};

/// An item as the front end sees it: its box in page fractions.
#[derive(Serialize, Deserialize, Clone)]
pub struct ItemView {
    /// 0 for a new item.
    #[serde(default)]
    pub id: u32,
    pub rect: [f32; 4],
    #[serde(default)]
    pub group: Option<u32>,
    pub kind: ItemKind,
    /// For pictures: the picture's size in pixels.
    #[serde(default)]
    pub pixels: Option<[u32; 2]>,
}

/// Quarter turns clockwise of the page as shown, from where its shown x axis
/// points in user space.
fn turn_of(engine: &Engine, doc: DocId, page: u32) -> Result<u8, String> {
    let p = engine.to_user_space(doc, page, vec![[0.0, 0.0], [1.0, 0.0]]).map_err(|e| e.to_string())?;
    let (dx, dy) = (p[1][0] - p[0][0], p[1][1] - p[0][1]);
    Ok(if dx.abs() >= dy.abs() {
        if dx >= 0.0 { 0 } else { 2 }
    } else if dy >= 0.0 {
        1
    } else {
        3
    })
}

fn to_view(engine: &Engine, doc: DocId, page: u32, it: &Item, pixels: Option<[u32; 2]>) -> Result<ItemView, String> {
    let r = it.rect;
    let f = engine.to_fractions(doc, page, vec![[r[0], r[1]], [r[2], r[3]]]).map_err(|e| e.to_string())?;
    let rect = [f[0][0].min(f[1][0]), f[0][1].min(f[1][1]), f[0][0].max(f[1][0]), f[0][1].max(f[1][1])];
    Ok(ItemView { id: it.id, rect, group: it.group, kind: it.kind.clone(), pixels })
}

fn views(
    engine: &Engine,
    doc: DocId,
    page: u32,
    s: &edits_edit::Session,
    items: &[Item],
) -> Result<Vec<ItemView>, String> {
    items
        .iter()
        .map(|it| {
            let px = match it.kind {
                ItemKind::Image { image } => s.picture_size(image).map(|(w, h)| [w, h]),
                _ => None,
            };
            to_view(engine, doc, page, it, px)
        })
        .collect()
}

/// The items on `page`.
#[tauri::command]
pub async fn items_page(app: tauri::AppHandle, doc: DocId, page: u32) -> Result<Vec<ItemView>, String> {
    with_session(app, doc, move |engine, s| {
        let items = s.items(page);
        views(engine, doc, page, s, &items)
    })
    .await
}

#[derive(Serialize)]
pub struct ItemsChanged {
    #[serde(flatten)]
    pub changed: Changed,
    /// The page's items as they are now (new ones with their ids, boxes fitted).
    pub items: Vec<ItemView>,
}

/// Replaces the items on `page` with `items` (one undo step).
#[tauri::command]
pub async fn items_set(
    app: tauri::AppHandle,
    doc: DocId,
    page: u32,
    items: Vec<ItemView>,
) -> Result<ItemsChanged, String> {
    with_session(app, doc, move |engine, s| {
        let old = s.items(page);
        let turn = turn_of(engine, doc, page)?;
        let mut list = Vec::with_capacity(items.len());
        for v in items {
            let p = engine
                .to_user_space(doc, page, vec![[v.rect[0], v.rect[1]], [v.rect[2], v.rect[3]]])
                .map_err(|e| e.to_string())?;
            // An item keeps the turn it was placed with.
            let kept = old.iter().find(|o| o.id == v.id && v.id != 0).map(|o| o.turn);
            list.push(Item {
                id: v.id,
                rect: [p[0][0].min(p[1][0]), p[0][1].min(p[1][1]), p[0][0].max(p[1][0]), p[0][1].max(p[1][1])],
                turn: kept.unwrap_or(turn),
                group: v.group,
                kind: v.kind,
            });
        }
        let set = s.set_items(page, list).map_err(|e| e.to_string())?;
        let items = views(engine, doc, page, s, &set)?;
        let changed = changed(engine, doc, s, vec![page], None)?;
        Ok(ItemsChanged { changed, items })
    })
    .await
}

#[derive(Serialize)]
pub struct Picture {
    pub image: u32,
    pub pixels: [u32; 2],
}

/// Reads a JPEG or PNG file for a picture item; place it with `items_set`.
#[tauri::command]
pub async fn item_picture(app: tauri::AppHandle, doc: DocId, path: String) -> Result<Picture, String> {
    with_session(app, doc, move |_, s| {
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let img = edits_edit::image::PdfImage::from_bytes(&bytes).map_err(|e| format!("{path}: {e}"))?;
        let pixels = [img.width, img.height];
        Ok(Picture { image: s.add_picture(img), pixels })
    })
    .await
}

/// The installed fonts text can be set in: (family, has Arabic).
#[tauri::command]
pub async fn font_families() -> Vec<(String, bool)> {
    tauri::async_runtime::spawn_blocking(edits_edit::font_families).await.unwrap_or_default()
}

/// Writes a new document of `pages` empty pages, `size` points (width,
/// height), with `margins` (left, top, right, bottom, points) for guides.
#[tauri::command]
pub async fn new_document(target: String, size: [f32; 2], pages: u32, margins: [f32; 4]) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (w, h) = (size[0].clamp(72.0, 14_400.0), size[1].clamp(72.0, 14_400.0));
        let m = margins.map(|v| v.clamp(0.0, w.min(h) / 3.0));
        let bytes = edits_edit::blank_pdf(w, h, pages, Some(m));
        let tmp = edits_edit::temp_path_for(std::path::Path::new(&target));
        std::fs::write(&tmp, bytes).map_err(|e| format!("could not write {target}: {e}"))?;
        std::fs::rename(&tmp, &target).map_err(|e| format!("could not write {target}: {e}"))?;
        Ok(target)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The margins a document was made with (left, top, right, bottom, points), or null.
#[tauri::command]
pub async fn doc_margins(app: tauri::AppHandle, doc: DocId) -> Result<Option<[f32; 4]>, String> {
    with_session(app, doc, |_, s| Ok(s.margins())).await
}
