//! Page text, the background text index, and streamed search.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use edits_core::{DocId, Engine, EngineError, PageText, SearchOptions};
use serde::Serialize;
use tauri::ipc::Channel;

/// Text for selection: above sharp tiles, the user is waiting on it.
const PAGE_TEXT_PRIORITY: u32 = 2800;
/// Search chunks: between previews and tiles.
const SEARCH_PRIORITY: u32 = 1500;
/// The background index: below everything else.
const INDEX_PRIORITY: u32 = 0;
/// Pages per search job: small enough that rendering is never held up long.
const SEARCH_CHUNK: usize = 8;
/// How long after opening the index starts.
const INDEX_DELAY: std::time::Duration = std::time::Duration::from_millis(1500);
/// Pages per index job.
const INDEX_CHUNK: u32 = 2;

#[tauri::command]
pub async fn page_text(
    state: tauri::State<'_, crate::AppState>,
    doc: DocId,
    page: u32,
) -> Result<PageText, EngineError> {
    let engine = state.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.page_text(doc, page, PAGE_TEXT_PRIORITY))
        .await
        .map_err(|_| EngineError::Stopped)?
}

/// Reads every page's text in the background so search can start at once.
/// Stops when done or when the document is closed.
pub fn start_index(engine: Arc<Engine>, doc: DocId) {
    std::thread::Builder::new()
        .name("edits-index".into())
        .spawn(move || {
            // Let the first pages render before the index competes for the render thread.
            std::thread::sleep(INDEX_DELAY);
            let mut from = 0;
            loop {
                match engine.index_text(doc, from, INDEX_CHUNK, INDEX_PRIORITY) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => from += INDEX_CHUNK,
                }
            }
        })
        .ok();
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SearchEvent {
    /// Matches on some pages: `[page, [[start, end], ...]]`, and how many pages are searched so far.
    Batch {
        results: Vec<(u32, Vec<[u32; 2]>)>,
        searched: u32,
    },
    Done {
        searched: u32,
    },
    Error {
        message: String,
    },
}

/// Running searches, so a new one (or the user) can stop an old one.
#[derive(Default)]
pub struct Searches {
    next: AtomicU64,
    running: Mutex<HashMap<u64, (DocId, Arc<AtomicBool>)>>,
}

impl Searches {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<u64, (DocId, Arc<AtomicBool>)>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Searches the whole document starting at `start_page` and wrapping round,
/// sending results as they are found. Returns the search's id. Starting a
/// search stops any earlier one on the same document.
#[derive(serde::Deserialize)]
pub struct SearchRequest {
    doc: DocId,
    query: String,
    options: SearchOptions,
    start_page: u32,
    page_count: u32,
}

#[tauri::command]
pub fn search(
    state: tauri::State<'_, crate::AppState>,
    searches: tauri::State<'_, Arc<Searches>>,
    request: SearchRequest,
    on_event: Channel<SearchEvent>,
) -> u64 {
    let SearchRequest { doc, query, options, start_page, page_count } = request;
    let id = searches.next.fetch_add(1, Ordering::Relaxed) + 1;
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut running = searches.lock();
        for (d, flag) in running.values() {
            if *d == doc {
                flag.store(true, Ordering::Relaxed);
            }
        }
        running.insert(id, (doc, stop.clone()));
    }
    let engine = state.engine.clone();
    let searches = searches.inner().clone();
    std::thread::Builder::new()
        .name("edits-search".into())
        .spawn(move || {
            let n = page_count;
            let start = start_page.min(n.saturating_sub(1));
            let order: Vec<u32> = (start..n).chain(0..start).collect();
            let mut searched = 0;
            for chunk in order.chunks(SEARCH_CHUNK) {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                match engine.search_pages(doc, chunk.to_vec(), query.clone(), options, SEARCH_PRIORITY) {
                    Ok(results) => {
                        searched += chunk.len() as u32;
                        if on_event.send(SearchEvent::Batch { results, searched }).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = on_event.send(SearchEvent::Error { message: e.to_string() });
                        break;
                    }
                }
            }
            if !stop.load(Ordering::Relaxed) {
                let _ = on_event.send(SearchEvent::Done { searched });
            }
            searches.lock().remove(&id);
        })
        .ok();
    id
}

#[tauri::command]
pub fn cancel_search(searches: tauri::State<'_, Arc<Searches>>, id: u64) {
    if let Some((_, flag)) = searches.lock().remove(&id) {
        flag.store(true, Ordering::Relaxed);
    }
}
