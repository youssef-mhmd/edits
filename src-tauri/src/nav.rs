//! Bookmarks and links for the front end.

use edits_core::{DocId, EngineError, Link, OutlineItem};

/// Priority for link lookups: after visible previews, before sharp tiles.
const LINKS_PRIORITY: u32 = 2500;

#[tauri::command]
pub async fn outline(state: tauri::State<'_, crate::AppState>, doc: DocId) -> Result<Vec<OutlineItem>, EngineError> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.outline(doc)).await.map_err(|_| EngineError::Stopped)?
}

#[tauri::command]
pub async fn page_links(
    state: tauri::State<'_, crate::AppState>,
    doc: DocId,
    page: u32,
) -> Result<Vec<Link>, EngineError> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.links(doc, page, LINKS_PRIORITY))
        .await
        .map_err(|_| EngineError::Stopped)?
}
