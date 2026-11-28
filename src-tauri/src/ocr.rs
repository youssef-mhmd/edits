//! Recognising text on scanned pages (PRD 5.7), in the background.
//!
//! One OCR thread owns the models. A run reads a document's pages one at a
//! time: the page is rendered at 300 dpi by the engine at a low priority (so
//! tiles always come first), read by edits-ocr on this thread, and its words
//! are turned into PDF user space. Results go into the editing session in
//! batches (the visible pages first, then every few pages), all as one undo
//! step, and the engine shows the new text layer. The front end hears of
//! progress through `ocr-progress` events and can put pages first (the ones
//! being looked at) or stop the run.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use edits_core::{DocId, Engine, RenderSpec};
use edits_ocr::{Digits, Gray, Options};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

/// Page renders for OCR: above the text index, below everything the reader sees.
const RENDER_PRIORITY: u32 = 100;
/// Pages read at this resolution (arabic-ocr reads PDFs at 300 dpi).
const DPI: f32 = 300.0;
/// Results are handed to the session at least this often...
const COMMIT_EVERY: Duration = Duration::from_secs(4);
/// ...or once this many pages are waiting.
const COMMIT_PAGES: usize = 8;
/// The models (about 100 MB) are freed after this long without work.
const UNLOAD_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Deserialize)]
pub struct OcrOptions {
    #[serde(default = "yes")]
    pub diacritics: bool,
    #[serde(default)]
    pub digits: Digits,
}

fn yes() -> bool {
    true
}

impl OcrOptions {
    fn options(&self) -> Options {
        Options { diacritics: self.diacritics, digits: self.digits, ..Options::default() }
    }
}

struct Job {
    doc: DocId,
    run: u64,
    /// Pages still to read, in order.
    todo: VecDeque<u32>,
    total: u32,
    done: u32,
    /// Pages that already had text and were left alone.
    skipped: u32,
    options: OcrOptions,
    skip_text: bool,
    /// The page keys when the run began.
    keys: Vec<u32>,
}

#[derive(Default)]
struct Inner {
    jobs: Vec<Job>,
    next_run: u64,
    thread: bool,
}

/// The OCR thread's work and the models it shares with region reads.
#[derive(Default)]
pub struct OcrState {
    inner: Mutex<Inner>,
    wake: Condvar,
    models: Arc<Mutex<Option<edits_ocr::Engine>>>,
}

#[derive(Clone, Serialize)]
struct Progress {
    doc: DocId,
    run: u64,
    done: u32,
    skipped: u32,
    total: u32,
    /// Pages whose text was just added.
    pages: Vec<u32>,
    status: Option<edits_edit::Status>,
    finished: bool,
    error: Option<String>,
}

impl OcrState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stops the document's run, if it has one (it was closed, or its pages changed).
    pub fn cancel(&self, doc: DocId) {
        // A page being read when this happens is dropped: its run is gone.
        self.lock().jobs.retain(|j| j.doc != doc);
    }
}

fn paths() -> edits_ocr::Paths {
    edits_ocr::Paths::beside_exe()
}

/// The models, loaded on first use with half the processor's threads so
/// that the interface stays smooth while pages are read.
fn with_models<R>(models: &Mutex<Option<edits_ocr::Engine>>, f: impl FnOnce(&mut edits_ocr::Engine) -> R) -> R {
    let mut g = models.lock().unwrap_or_else(|e| e.into_inner());
    let e = g.get_or_insert_with(|| {
        let mut e = edits_ocr::Engine::new(paths());
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
        e.set_threads((n / 2).clamp(1, 4));
        e
    });
    f(e)
}

/// The render size for OCR of a page `w` x `h` points: 300 dpi, less where a
/// side would pass the engine's largest tile.
fn ocr_size(w: f32, h: f32) -> (u32, u32) {
    let scale = (DPI / 72.0).min(edits_core::MAX_TILE_SIDE as f32 / w.max(h).max(1.0));
    (((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
}

/// Reads one page; returns its text layer (None for a page left alone).
fn read_page(
    engine: &Engine,
    models: &Mutex<Option<edits_ocr::Engine>>,
    doc: DocId,
    page: u32,
    options: &Options,
    skip_text: bool,
) -> Result<Option<edits_edit::OcrLayer>, String> {
    let info = engine.info(doc).map_err(|e| e.to_string())?;
    let size = info.pages.get(page as usize).ok_or("no such page")?;
    if skip_text {
        let t = engine.page_text(doc, page, RENDER_PRIORITY).map_err(|e| e.to_string())?;
        if t.text.chars().any(|c| !c.is_whitespace()) {
            return Ok(None);
        }
    }
    let (w, h) = ocr_size(size.w, size.h);
    let spec = RenderSpec { doc, page, page_w: w, page_h: h, x: 0, y: 0, w, h, rotation: 0 };
    let tile = engine.render(spec, RENDER_PRIORITY).map_err(|e| e.to_string())?;
    let gray = Gray::from_rgba(tile.w as usize, tile.h as usize, &tile.rgba);
    drop(tile);
    let page_read = with_models(models, |m| m.read(&gray, options)).map_err(|e| e.to_string())?;
    let words = page_read.layer_words();
    if words.is_empty() {
        return Ok(Some(edits_edit::OcrLayer::default()));
    }
    let (tw, th) = (gray.width as f32, gray.height as f32);
    let points: Vec<[f32; 2]> = words.iter().flat_map(|(q, _)| q.iter().map(|c| [c[0] / tw, c[1] / th])).collect();
    let user = engine.to_user_space(doc, page, points).map_err(|e| e.to_string())?;
    Ok(Some(edits_edit::OcrLayer {
        words: words
            .into_iter()
            .zip(user.as_chunks::<4>().0)
            .map(|((_, shown), q)| edits_edit::OcrWord { quad: *q, shown })
            .collect(),
    }))
}

fn run_thread(app: tauri::AppHandle) {
    let ocr = app.state::<OcrState>();
    let engine = app.state::<crate::AppState>().engine.clone();
    let mut idle_since = Instant::now();
    // Per run: results waiting to go into the session, and the last commit.
    let mut ready: HashMap<u64, (Vec<(u32, edits_edit::OcrLayer)>, Instant)> = HashMap::new();
    loop {
        // The next page, or wait.
        let next = {
            let mut g = ocr.lock();
            loop {
                if let Some(j) = g.jobs.first_mut() {
                    let page = j.todo.pop_front();
                    break Some((j.doc, j.run, page, j.options, j.skip_text, j.keys.clone()));
                }
                let (ng, timeout) = ocr.wake.wait_timeout(g, Duration::from_secs(5)).unwrap_or_else(|e| e.into_inner());
                g = ng;
                if timeout.timed_out()
                    && g.jobs.is_empty()
                    && idle_since.elapsed() > UNLOAD_AFTER
                    && let Ok(mut m) = ocr.models.try_lock()
                {
                    *m = None;
                }
            }
        };
        let Some((doc, run, page, options, skip_text, keys)) = next else { continue };
        idle_since = Instant::now();
        let result = page.map(|p| read_page(&engine, &ocr.models, doc, p, &options.options(), skip_text));

        let mut g = ocr.lock();
        let Some(j) = g.jobs.iter_mut().find(|j| j.run == run) else {
            ready.remove(&run);
            continue;
        };
        let entry = ready.entry(run).or_insert_with(|| (Vec::new(), Instant::now()));
        let mut error = None;
        match (page, result) {
            (Some(p), Some(Ok(Some(layer)))) => {
                j.done += 1;
                entry.0.push((p, layer));
            }
            (Some(_), Some(Ok(None))) => {
                j.done += 1;
                j.skipped += 1;
            }
            (_, Some(Err(e))) => error = Some(e),
            _ => {}
        }
        let finished = j.todo.is_empty() || error.is_some();
        let (done, skipped, total) = (j.done, j.skipped, j.total);
        let due = finished || entry.0.len() >= COMMIT_PAGES || entry.1.elapsed() >= COMMIT_EVERY;
        if finished {
            g.jobs.retain(|j| j.run != run);
        }
        drop(g);

        let mut pages = Vec::new();
        let mut status = None;
        if due && let Some((layers, _)) = ready.remove(&run) {
            pages = layers.iter().map(|l| l.0).collect();
            if !layers.is_empty() {
                match crate::edit::commit_ocr(&app, doc, &keys, run, layers) {
                    Ok(s) => status = Some(s),
                    Err(e) => {
                        error.get_or_insert(e);
                        pages.clear();
                        ocr.cancel(doc);
                    }
                }
            }
            if !finished && error.is_none() {
                ready.insert(run, (Vec::new(), Instant::now()));
            }
        }
        let finished = finished || error.is_some();
        if finished {
            ready.remove(&run);
        }
        let _ = app.emit("ocr-progress", Progress { doc, run, done, skipped, total, pages, status, finished, error });
    }
}

/// True if the OCR runtime and models are installed.
#[tauri::command]
pub fn ocr_available() -> bool {
    paths().available()
}

/// Starts reading `pages` of `doc` (replacing a run it has), `first` first.
/// `skip_text`: leave pages that already have text alone. Returns the run.
#[tauri::command]
pub fn ocr_start(
    app: tauri::AppHandle,
    ocr: tauri::State<'_, OcrState>,
    doc: DocId,
    pages: Vec<u32>,
    first: Vec<u32>,
    options: OcrOptions,
    skip_text: bool,
) -> Result<u64, String> {
    if !paths().available() {
        return Err("text recognition is not installed".into());
    }
    let keys = crate::edit::page_keys(&app, doc)?;
    let n = keys.len() as u32;
    let mut todo: VecDeque<u32> = first.iter().copied().filter(|p| *p < n && pages.contains(p)).collect();
    for p in pages {
        if p < n && !todo.contains(&p) {
            todo.push_back(p);
        }
    }
    if todo.is_empty() {
        return Err("there are no pages to read".into());
    }
    ocr.cancel(doc);
    let mut g = ocr.lock();
    g.next_run += 1;
    let run = g.next_run;
    let total = todo.len() as u32;
    g.jobs.push(Job { doc, run, todo, total, done: 0, skipped: 0, options, skip_text, keys });
    if !g.thread {
        g.thread = true;
        let app = app.clone();
        std::thread::Builder::new()
            .name("edits-ocr".into())
            .spawn(move || run_thread(app))
            .map_err(|e| e.to_string())?;
    }
    drop(g);
    ocr.wake.notify_all();
    Ok(run)
}

/// Puts `pages` (the ones on screen) at the front of the document's run.
#[tauri::command]
pub fn ocr_first(ocr: tauri::State<'_, OcrState>, doc: DocId, pages: Vec<u32>) {
    let mut g = ocr.lock();
    if let Some(j) = g.jobs.iter_mut().find(|j| j.doc == doc) {
        for p in pages.iter().rev() {
            if let Some(i) = j.todo.iter().position(|t| t == p) {
                j.todo.remove(i);
                j.todo.push_front(*p);
            }
        }
    }
}

/// Stops the document's run. Pages already read keep their text.
#[tauri::command]
pub fn ocr_cancel(ocr: tauri::State<'_, OcrState>, doc: DocId) {
    ocr.cancel(doc);
}

/// Reads the text in part of a page: `rect` is left, top, right, bottom in
/// page fractions (as shown, /Rotate applied, no view rotation).
#[tauri::command]
pub async fn ocr_region(
    app: tauri::AppHandle,
    doc: DocId,
    page: u32,
    rect: [f32; 4],
    options: OcrOptions,
) -> Result<String, String> {
    if !paths().available() {
        return Err("text recognition is not installed".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let engine = app.state::<crate::AppState>().engine.clone();
        let models = app.state::<OcrState>().models.clone();
        let info = engine.info(doc).map_err(|e| e.to_string())?;
        let size = info.pages.get(page as usize).ok_or("no such page")?;
        let (pw, ph) = ocr_size(size.w, size.h);
        let clamp = |v: f32| v.clamp(0.0, 1.0);
        let (l, t) = (clamp(rect[0].min(rect[2])), clamp(rect[1].min(rect[3])));
        let (r, b) = (clamp(rect[0].max(rect[2])), clamp(rect[1].max(rect[3])));
        let x = ((l * pw as f32) as u32).min(pw - 1);
        let y = ((t * ph as f32) as u32).min(ph - 1);
        let w = (((r - l) * pw as f32).round() as u32).clamp(1, edits_core::MAX_TILE_SIDE);
        let h = (((b - t) * ph as f32).round() as u32).clamp(1, edits_core::MAX_TILE_SIDE);
        if w < 8 || h < 8 {
            return Err("the area is too small".into());
        }
        let spec = RenderSpec { doc, page, page_w: pw, page_h: ph, x, y, w, h, rotation: 0 };
        // The reader is waiting: as soon as the tiles on screen are done.
        let tile = engine.render(spec, 2000).map_err(|e| e.to_string())?;
        let gray = Gray::from_rgba(tile.w as usize, tile.h as usize, &tile.rgba);
        let read = with_models(&models, |m| m.read(&gray, &options.options())).map_err(|e| e.to_string())?;
        Ok(read.text())
    })
    .await
    .map_err(|e| e.to_string())?
}
