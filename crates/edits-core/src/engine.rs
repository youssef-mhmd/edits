//! The render thread and its public handle.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Instant;

use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};

use crate::library::find_pdfium_dir;
use crate::queue::{Priority, Queue};

/// Identifies an open document. Assigned when the open is submitted.
pub type DocId = u32;
/// Identifies a queued job, for cancelling or re-prioritising it.
pub type Ticket = u64;

/// Pages kept loaded per document. Loading a page parses its content, so the
/// pages being scrolled through stay loaded while their tiles render.
const PAGE_CACHE: usize = 12;

/// Priority used for opening and closing documents: before any rendering.
const CONTROL_PRIORITY: Priority = Priority::MAX;

#[derive(Debug, thiserror::Error, Clone)]
pub enum EngineError {
    #[error("PDFium library not found; set EDITS_PDFIUM_DIR")]
    LibraryNotFound,
    #[error("failed to load PDFium: {0}")]
    LibraryLoad(String),
    #[error("could not open document: {0}")]
    Open(String),
    #[error("the document needs a password")]
    PasswordRequired,
    #[error("no open document with id {0}")]
    UnknownDoc(DocId),
    #[error("page {0} is out of range")]
    PageOutOfRange(u32),
    #[error("render failed: {0}")]
    Render(String),
    #[error("invalid render request: {0}")]
    InvalidSpec(&'static str),
    #[error("cancelled")]
    Cancelled,
    #[error("engine stopped")]
    Stopped,
}

/// Serialised as its message, for display in the interface.
impl Serialize for EngineError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// Page size in PDF points (1/72 inch), with page rotation applied.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct PageSize {
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocInfo {
    pub doc: DocId,
    pub page_count: u32,
    pub pages: Vec<PageSize>,
    /// Time spent on the render thread opening the file and reading page sizes.
    pub open_us: u64,
}

/// A region of a page rendered at a given size.
///
/// The page is rendered as if it were `page_w` × `page_h` device pixels, and
/// the rectangle `x, y, w, h` (device pixels, from the top-left) is returned.
/// A whole-page preview is `x = y = 0, w = page_w, h = page_h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderSpec {
    pub doc: DocId,
    pub page: u32,
    pub page_w: u32,
    pub page_h: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Largest tile side accepted, to bound memory per job.
pub const MAX_TILE_SIDE: u32 = 4096;
/// Largest virtual page side accepted (PDFium's own limit is about 2^15).
pub const MAX_PAGE_SIDE: u32 = 30_000;

impl RenderSpec {
    fn validate(&self) -> Result<(), EngineError> {
        if self.w == 0 || self.h == 0 || self.page_w == 0 || self.page_h == 0 {
            return Err(EngineError::InvalidSpec("empty size"));
        }
        if self.w > MAX_TILE_SIDE || self.h > MAX_TILE_SIDE {
            return Err(EngineError::InvalidSpec("tile too large"));
        }
        if self.page_w > MAX_PAGE_SIDE || self.page_h > MAX_PAGE_SIDE {
            return Err(EngineError::InvalidSpec("page too large"));
        }
        if self.x >= self.page_w || self.y >= self.page_h {
            return Err(EngineError::InvalidSpec("tile outside page"));
        }
        Ok(())
    }
}

/// A rendered region: tightly packed RGBA, `w * h * 4` bytes, opaque.
#[derive(Debug)]
pub struct Tile {
    pub spec: RenderSpec,
    /// Actual size; smaller than requested where the tile overhangs the page edge.
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    /// Time from submit until the render thread picked the job up.
    pub queued_us: u64,
    /// Time spent rendering.
    pub render_us: u64,
}

type Reply<T> = Box<dyn FnOnce(Result<T, EngineError>) + Send>;

enum Job {
    Open { doc: DocId, path: PathBuf, password: Option<String>, reply: Reply<DocInfo> },
    Close { doc: DocId },
    Render { spec: RenderSpec, submitted: Instant, reply: Reply<Tile> },
}

impl Job {
    fn fail(self, err: EngineError) {
        match self {
            Job::Open { reply, .. } => reply(Err(err)),
            Job::Render { reply, .. } => reply(Err(err)),
            Job::Close { .. } => {}
        }
    }
}

#[derive(Default)]
struct State {
    queue: Queue<Job>,
    next_ticket: Ticket,
    next_doc: DocId,
    stopping: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves the queue consistent, so poisoning is ignored.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Handle to the render thread. Cheap to share behind an `Arc`.
pub struct Engine {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    /// Starts the render thread, loading PDFium from [`find_pdfium_dir`].
    pub fn start() -> Result<Self, EngineError> {
        let dir = find_pdfium_dir().ok_or(EngineError::LibraryNotFound)?;
        Self::start_with_library_dir(&dir)
    }

    pub fn start_with_library_dir(dir: &Path) -> Result<Self, EngineError> {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), wake: Condvar::new() });
        let (ready_tx, ready_rx) = mpsc::channel();
        let lib_path = Pdfium::pdfium_platform_library_name_at_path(dir);
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("edits-render".into())
            .spawn(move || {
                let pdfium = match Pdfium::bind_to_library(&lib_path) {
                    Ok(bindings) => Pdfium::new(bindings),
                    // Another engine in this process loaded it already; share it.
                    Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Pdfium::default(),
                    Err(e) => {
                        let _ = ready_tx.send(Err(EngineError::LibraryLoad(e.to_string())));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                // The Pdfium handle must outlive every document opened from it;
                // it lives as long as this thread, which is the program.
                let pdfium: &'static Pdfium = Box::leak(Box::new(pdfium));
                RenderThread { pdfium, docs: HashMap::new() }.run(&thread_shared);
            })
            .map_err(|e| EngineError::LibraryLoad(e.to_string()))?;

        ready_rx.recv().map_err(|_| EngineError::Stopped)??;
        Ok(Self { shared, thread: Mutex::new(Some(thread)) })
    }

    fn submit(&self, priority: Priority, make: impl FnOnce(&mut State) -> Job) -> Ticket {
        let mut st = self.shared.lock();
        let job = make(&mut st);
        if st.stopping {
            drop(st);
            job.fail(EngineError::Stopped);
            return 0;
        }
        st.next_ticket += 1;
        let ticket = st.next_ticket;
        st.queue.push(ticket, priority, job);
        drop(st);
        self.shared.wake.notify_one();
        ticket
    }

    /// Queues opening a document. Returns its id at once; `reply` runs on the
    /// render thread when the open finishes.
    pub fn open_with(
        &self,
        path: impl Into<PathBuf>,
        password: Option<String>,
        reply: impl FnOnce(Result<DocInfo, EngineError>) + Send + 'static,
    ) -> DocId {
        let path = path.into();
        let mut doc_id = 0;
        self.submit(CONTROL_PRIORITY, |st| {
            st.next_doc += 1;
            doc_id = st.next_doc;
            Job::Open { doc: doc_id, path, password, reply: Box::new(reply) }
        });
        doc_id
    }

    /// Opens a document and waits for the result.
    pub fn open(&self, path: impl Into<PathBuf>, password: Option<String>) -> Result<DocInfo, EngineError> {
        let (tx, rx) = mpsc::channel();
        self.open_with(path, password, move |r| {
            let _ = tx.send(r);
        });
        rx.recv().map_err(|_| EngineError::Stopped)?
    }

    /// Closes a document. Its queued renders are cancelled.
    pub fn close(&self, doc: DocId) {
        self.cancel_where(|s| s.doc == doc);
        self.submit(CONTROL_PRIORITY, |_| Job::Close { doc });
    }

    /// Queues a render. `reply` runs on the render thread, or on the
    /// cancelling thread with [`EngineError::Cancelled`].
    pub fn render_with(
        &self,
        spec: RenderSpec,
        priority: Priority,
        reply: impl FnOnce(Result<Tile, EngineError>) + Send + 'static,
    ) -> Ticket {
        if let Err(e) = spec.validate() {
            reply(Err(e));
            return 0;
        }
        let submitted = Instant::now();
        self.submit(priority, |_| Job::Render { spec, submitted, reply: Box::new(reply) })
    }

    /// Renders and waits for the result.
    pub fn render(&self, spec: RenderSpec, priority: Priority) -> Result<Tile, EngineError> {
        let (tx, rx) = mpsc::channel();
        self.render_with(spec, priority, move |r| {
            let _ = tx.send(r);
        });
        rx.recv().map_err(|_| EngineError::Stopped)?
    }

    /// Cancels a queued job. Returns false if it already started or finished.
    pub fn cancel(&self, ticket: Ticket) -> bool {
        let job = self.shared.lock().queue.remove(ticket);
        match job {
            Some(job) => {
                job.fail(EngineError::Cancelled);
                true
            }
            None => false,
        }
    }

    /// Cancels every queued render whose spec matches. Returns how many.
    pub fn cancel_where(&self, mut pred: impl FnMut(&RenderSpec) -> bool) -> usize {
        let drained = self.shared.lock().queue.drain_where(|job| matches!(job, Job::Render { spec, .. } if pred(spec)));
        let n = drained.len();
        for (_, job) in drained {
            job.fail(EngineError::Cancelled);
        }
        n
    }

    /// Changes the priority of a queued job. Returns false if it is not queued.
    pub fn set_priority(&self, ticket: Ticket, priority: Priority) -> bool {
        self.shared.lock().queue.set_priority(ticket, priority)
    }

    /// Number of jobs waiting.
    pub fn queued(&self) -> usize {
        self.shared.lock().queue.len()
    }

    /// Stops the render thread after the current job. Queued jobs fail with
    /// [`EngineError::Stopped`].
    pub fn shutdown(&self) {
        let drained = {
            let mut st = self.shared.lock();
            st.stopping = true;
            st.queue.drain_where(|_| true)
        };
        self.shared.wake.notify_all();
        for (_, job) in drained {
            job.fail(EngineError::Stopped);
        }
        if let Some(t) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = t.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------------
// Render thread

struct OpenDoc {
    // Declared before `doc` so pages drop first.
    pages: Vec<(u32, PdfPage<'static>)>,
    doc: PdfDocument<'static>,
    page_count: u32,
}

impl OpenDoc {
    fn page(&mut self, index: u32) -> Result<&PdfPage<'static>, EngineError> {
        if index >= self.page_count {
            return Err(EngineError::PageOutOfRange(index));
        }
        if let Some(pos) = self.pages.iter().position(|(i, _)| *i == index) {
            // Move to the back: most recently used.
            let entry = self.pages.remove(pos);
            self.pages.push(entry);
        } else {
            let page = self.doc.pages().get(index as PdfPageIndex).map_err(|e| EngineError::Render(e.to_string()))?;
            if self.pages.len() >= PAGE_CACHE {
                self.pages.remove(0);
            }
            self.pages.push((index, page));
        }
        Ok(&self.pages.last().expect("just pushed").1)
    }
}

struct RenderThread {
    pdfium: &'static Pdfium,
    docs: HashMap<DocId, OpenDoc>,
}

impl RenderThread {
    fn run(mut self, shared: &Shared) {
        loop {
            let job = {
                let mut st = shared.lock();
                loop {
                    if st.stopping {
                        return;
                    }
                    if let Some((_, job)) = st.queue.pop() {
                        break job;
                    }
                    st = shared.wake.wait(st).unwrap_or_else(|e| e.into_inner());
                }
            };
            // A panic inside PDFium bindings must not take the thread down.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(job)));
            if outcome.is_err() {
                log::error!("render job panicked");
            }
        }
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::Open { doc, path, password, reply } => reply(self.open(doc, &path, password.as_deref())),
            Job::Close { doc } => {
                self.docs.remove(&doc);
            }
            Job::Render { spec, submitted, reply } => {
                let queued_us = submitted.elapsed().as_micros() as u64;
                let start = Instant::now();
                let result = self.render(&spec).map(|(w, h, rgba)| Tile {
                    spec,
                    w,
                    h,
                    rgba,
                    queued_us,
                    render_us: start.elapsed().as_micros() as u64,
                });
                reply(result)
            }
        }
    }

    fn open(&mut self, id: DocId, path: &Path, password: Option<&str>) -> Result<DocInfo, EngineError> {
        let start = Instant::now();
        let doc = self.pdfium.load_pdf_from_file(path, password).map_err(|e| match e {
            PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
                EngineError::PasswordRequired
            }
            PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FormatError) => {
                EngineError::Open("the file is damaged or is not a PDF".into())
            }
            PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FileError) => {
                EngineError::Open("the file could not be read".into())
            }
            PdfiumError::IoError(e) => EngineError::Open(e.to_string()),
            other => EngineError::Open(other.to_string()),
        })?;
        let sizes = doc.pages().page_sizes().map_err(|e| EngineError::Open(e.to_string()))?;
        let pages: Vec<PageSize> = sizes.iter().map(|r| PageSize { w: r.width().value, h: r.height().value }).collect();
        let page_count = pages.len() as u32;
        self.docs.insert(id, OpenDoc { pages: Vec::new(), doc, page_count });
        Ok(DocInfo { doc: id, page_count, pages, open_us: start.elapsed().as_micros() as u64 })
    }

    fn render(&mut self, spec: &RenderSpec) -> Result<(u32, u32, Vec<u8>), EngineError> {
        let doc = self.docs.get_mut(&spec.doc).ok_or(EngineError::UnknownDoc(spec.doc))?;
        let page = doc.page(spec.page)?;
        let w = spec.w.min(spec.page_w - spec.x);
        let h = spec.h.min(spec.page_h - spec.y);
        let render_err = |e: PdfiumError| EngineError::Render(e.to_string());

        let mut bitmap = PdfBitmap::empty(w as Pixels, h as Pixels, PdfBitmapFormat::BGRA).map_err(render_err)?;
        let config = PdfRenderConfig::new()
            .set_fixed_size(spec.page_w as Pixels, spec.page_h as Pixels)
            .set_origin(-(spec.x as Pixels), -(spec.y as Pixels))
            .set_format(PdfBitmapFormat::BGRA)
            .set_reverse_byte_order(true)
            .render_annotations(true)
            .render_form_data(true);
        // The tile is clipped to the page above, so the white clear that
        // pdfium-render applies to the page rectangle covers the whole bitmap.
        page.render_into_bitmap_with_config(&mut bitmap, &config).map_err(render_err)?;
        Ok((w, h, bitmap.as_raw_bytes()))
    }
}
