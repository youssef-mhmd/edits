//! The render thread, the render worker processes, and their public handle.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::docs::{Docs, OpenDoc, Stamp, load_library};
use crate::library::find_pdfium_dir;
use crate::queue::{Priority, Queue};
use crate::worker::{Worker, WorkerCommand};

/// Identifies an open document. Assigned when the open is submitted.
pub type DocId = u32;
/// Search matches on one page: its index and character ranges [start, end).
pub type PageHits = (u32, Vec<[u32; 2]>);

/// Identifies a queued job, for cancelling or re-prioritising it.
pub type Ticket = u64;

/// Default limit on a process's private memory before its documents are
/// reopened to release what PDFium holds (see `Docs::trim`).
pub const DEFAULT_MEMORY_LIMIT_MB: u64 = 128;

/// A page whose render (loading it included) takes longer than this is
/// heavy: once render workers are enabled, its renders go to them, in
/// parallel, and can be stopped.
pub const HEAVY_RENDER_US: u64 = 40_000;
/// A cancelled render that has run this long is stopped by ending its worker.
/// Shorter ones are left to finish: restarting a worker costs more.
const KILL_AFTER: Duration = Duration::from_millis(30);
/// A render in a worker that takes longer than this is stopped and the page
/// shown as an error, so a hostile page cannot keep a worker busy forever.
const RENDER_TIMEOUT: Duration = Duration::from_secs(20);
/// Workers with nothing to do for this long exit, giving their memory back.
const WORKER_IDLE_EXIT: Duration = Duration::from_secs(30);

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
    #[error("{0}")]
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
    /// Pages whose size could not be read; shown at a substitute size.
    pub broken_pages: Vec<u32>,
    /// Time spent on the render thread opening the file and reading page sizes.
    pub open_us: u64,
}

/// A region of a page rendered at a given size.
///
/// The page is rendered as if it were `page_w` × `page_h` device pixels, and
/// the rectangle `x, y, w, h` (device pixels, from the top-left) is returned.
/// A whole-page preview is `x = y = 0, w = page_w, h = page_h`.
///
/// `rotation` turns the page clockwise in quarter turns (view rotation, on
/// top of the page's own /Rotate). `page_w` and `page_h` are the size after
/// rotation, as it appears on screen.
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
    #[serde(default)]
    pub rotation: u8,
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
        if self.rotation > 3 {
            return Err(EngineError::InvalidSpec("rotation must be 0 to 3 quarter turns"));
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
    /// Time spent drawing, not counting loading (parsing) the page.
    pub render_us: u64,
}

type Reply<T> = Box<dyn FnOnce(Result<T, EngineError>) + Send>;

/// Runs on the render thread with the open document, or the error if there is none.
type Call = Box<dyn FnOnce(Result<&mut OpenDoc, EngineError>) + Send>;

enum Job {
    Open { doc: DocId, path: PathBuf, password: Option<String>, reply: Reply<DocInfo> },
    Close { doc: DocId },
    Render { spec: RenderSpec, submitted: Instant, reply: Reply<Tile> },
    Call { doc: DocId, call: Call },
}

impl Job {
    fn fail(self, err: EngineError) {
        match self {
            Job::Open { reply, .. } => reply(Err(err)),
            Job::Render { reply, .. } => reply(Err(err)),
            Job::Call { call, .. } => call(Err(err)),
            Job::Close { .. } => {}
        }
    }
}

/// Where an open document came from, so a worker can open its own copy.
#[derive(Clone)]
struct DocSource {
    path: PathBuf,
    password: Option<String>,
    stamp: Option<Stamp>,
    /// Changes each time the document is reopened after a save.
    generation: u64,
}

/// Render workers: how to start them and how many run.
#[derive(Default)]
struct Pool {
    cmd: Option<WorkerCommand>,
    pdfium_dir: Option<PathBuf>,
    max: usize,
    /// Driver threads, one per worker process.
    drivers: usize,
    /// Drivers waiting for a job.
    idle: usize,
    watchdog: bool,
}

/// A render taken by a worker driver.
struct Running {
    spec: RenderSpec,
    /// Set once the job is sent to a worker; ending it stops the render.
    child: Option<Arc<Mutex<Child>>>,
    started: Instant,
    /// Why the render was stopped, if it was.
    stopped: Option<EngineError>,
}

#[derive(Default)]
struct State {
    queue: Queue<Job>,
    next_ticket: Ticket,
    next_doc: DocId,
    stopping: bool,
    /// Pages that render slowly; they go to workers when there are any.
    heavy: HashSet<(DocId, u32)>,
    sources: HashMap<DocId, DocSource>,
    running: HashMap<Ticket, Running>,
    pool: Pool,
    last_generation: u64,
}

impl State {
    fn next_generation(&mut self) -> u64 {
        self.last_generation += 1;
        self.last_generation
    }
}

/// True if `job` should run in a worker rather than on the render thread.
fn offloadable(pool: &Pool, heavy: &HashSet<(DocId, u32)>, sources: &HashMap<DocId, DocSource>, job: &Job) -> bool {
    pool.cmd.is_some()
        && matches!(job, Job::Render { spec, .. }
            if heavy.contains(&(spec.doc, spec.page)) && sources.contains_key(&spec.doc))
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    memory_limit: std::sync::atomic::AtomicU64,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock leaves the queue consistent, so poisoning is ignored.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Starts worker drivers for heavy renders that are waiting, up to the limit.
fn ensure_drivers(shared: &Arc<Shared>, st: &mut State) {
    if st.pool.cmd.is_none() || st.heavy.is_empty() || st.stopping {
        return;
    }
    let State { queue, heavy, sources, pool, .. } = st;
    let waiting = queue.count_where(|j| offloadable(pool, heavy, sources, j));
    let mut free = pool.idle;
    while pool.drivers < pool.max && free < waiting {
        let s = shared.clone();
        if std::thread::Builder::new().name("edits-worker".into()).spawn(move || drive_worker(s)).is_err() {
            break;
        }
        pool.drivers += 1;
        free += 1;
    }
}

/// A heavy render taken off the queue by a driver.
struct Taken {
    ticket: Ticket,
    priority: Priority,
    spec: RenderSpec,
    submitted: Instant,
    reply: Reply<Tile>,
    source: Option<DocSource>,
    /// Documents the worker has open that have since been closed.
    stale: Vec<DocId>,
    cmd: Option<WorkerCommand>,
    pdfium_dir: Option<PathBuf>,
}

/// Waits for the most urgent heavy render. None when stopping or idle for long.
fn take_heavy(shared: &Shared, worker: Option<&Worker>) -> Option<Taken> {
    let mut st = shared.lock();
    st.pool.idle += 1;
    let mut timed_out = false;
    let found = loop {
        if st.stopping {
            break None;
        }
        let State { queue, heavy, sources, pool, .. } = &mut *st;
        if let Some(x) = queue.pop_where(|j| offloadable(pool, heavy, sources, j)) {
            break Some(x);
        }
        if timed_out {
            break None;
        }
        let (g, t) = shared.wake.wait_timeout(st, WORKER_IDLE_EXIT).unwrap_or_else(|e| e.into_inner());
        st = g;
        timed_out = t.timed_out();
    };
    st.pool.idle -= 1;
    match found {
        Some((ticket, priority, Job::Render { spec, submitted, reply })) => {
            let source = st.sources.get(&spec.doc).cloned();
            let stale = worker
                .map(|w| {
                    w.open
                        .iter()
                        .filter(|(d, g)| st.sources.get(d).map(|s| s.generation) != Some(**g))
                        .map(|(d, _)| *d)
                        .collect()
                })
                .unwrap_or_default();
            st.running.insert(ticket, Running { spec, child: None, started: Instant::now(), stopped: None });
            let (cmd, pdfium_dir) = (st.pool.cmd.clone(), st.pool.pdfium_dir.clone());
            Some(Taken { ticket, priority, spec, submitted, reply, source, stale, cmd, pdfium_dir })
        }
        Some((ticket, priority, job)) => {
            // Only renders are offloadable; never lose a job all the same.
            st.queue.push(ticket, priority, job);
            st.pool.drivers -= 1;
            None
        }
        None => {
            st.pool.drivers -= 1;
            None
        }
    }
}

/// Hands a render back to the render thread. With `doc`, that document is
/// no longer sent to workers; without, workers are switched off.
fn give_back(shared: &Shared, t: Taken, doc: Option<DocId>) {
    let mut st = shared.lock();
    st.running.remove(&t.ticket);
    match doc {
        Some(d) => {
            st.sources.remove(&d);
        }
        None => st.pool.cmd = None,
    }
    st.queue.push(t.ticket, t.priority, Job::Render { spec: t.spec, submitted: t.submitted, reply: t.reply });
    drop(st);
    shared.wake.notify_all();
}

/// One driver thread per worker process: takes heavy renders off the queue,
/// sends them to its worker and replies with the result.
fn drive_worker(shared: Arc<Shared>) {
    let mut worker: Option<Worker> = None;
    while let Some(t) = take_heavy(&shared, worker.as_ref()) {
        let queued_us = t.submitted.elapsed().as_micros() as u64;
        let (Some(cmd), Some(source)) = (t.cmd.clone(), t.source.clone()) else {
            let doc = t.spec.doc;
            give_back(&shared, t, Some(doc));
            continue;
        };
        if worker.is_none() {
            match Worker::spawn(&cmd, t.pdfium_dir.as_deref()) {
                Ok(w) => worker = Some(w),
                Err(e) => {
                    log::warn!("cannot start a render worker: {e}");
                    give_back(&shared, t, None);
                    shared.lock().pool.drivers -= 1;
                    return;
                }
            }
        }
        let w = worker.as_mut().expect("spawned above");
        let mut alive = t.stale.iter().all(|d| w.close(*d).is_ok());
        if alive && !w.open.contains_key(&t.spec.doc) {
            match w.open(t.spec.doc, source.generation, &source.path, source.password.as_deref(), source.stamp) {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    log::warn!("render worker cannot open {}: {e}", source.path.display());
                    let doc = t.spec.doc;
                    give_back(&shared, t, Some(doc));
                    continue;
                }
                Err(_) => alive = false,
            }
        }
        if !alive {
            // The worker died opening the file: let the render thread have it.
            worker = None;
            let doc = t.spec.doc;
            give_back(&shared, t, Some(doc));
            continue;
        }

        // Send the render, unless it was cancelled while the worker got ready.
        let Taken { ticket, spec, reply, .. } = t;
        {
            let mut st = shared.lock();
            let Some(r) = st.running.get_mut(&ticket) else { continue };
            if let Some(e) = r.stopped.take() {
                st.running.remove(&ticket);
                drop(st);
                reply(Err(e));
                continue;
            }
            r.child = Some(w.child.clone());
            r.started = Instant::now();
        }
        let result = w.render(spec);
        let stopped = shared.lock().running.remove(&ticket).and_then(|r| r.stopped);
        match result {
            Ok(Ok((w, h, rgba, render_us))) => reply(Ok(Tile { spec, w, h, rgba, queued_us, render_us })),
            Ok(Err(e)) => reply(Err(e)),
            Err(_) => {
                worker = None;
                reply(Err(stopped.unwrap_or_else(|| {
                    log::warn!("render worker stopped on page {} of document {}", spec.page, spec.doc);
                    EngineError::Render("the page could not be drawn".into())
                })));
            }
        }
    }
}

/// Stops worker renders that run too long.
fn watchdog(shared: Arc<Shared>) {
    loop {
        std::thread::sleep(Duration::from_millis(250));
        let mut st = shared.lock();
        if st.stopping {
            return;
        }
        for r in st.running.values_mut() {
            if let Some(c) = &r.child
                && r.stopped.is_none()
                && r.started.elapsed() > RENDER_TIMEOUT
            {
                r.stopped = Some(EngineError::Render("the page takes too long to draw".into()));
                crate::worker::kill(c);
            }
        }
    }
}

/// Marks a running worker render cancelled, ending its worker if it has run
/// long enough that finishing would cost more than starting a new one.
fn cancel_running(r: &mut Running) {
    if r.stopped.is_some() {
        return;
    }
    r.stopped = Some(EngineError::Cancelled);
    if let Some(c) = &r.child
        && r.started.elapsed() >= KILL_AFTER
    {
        crate::worker::kill(c);
    }
}

/// Handle to the render thread. Cheap to share behind an `Arc`.
pub struct Engine {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
    pdfium_dir: PathBuf,
}

/// A sensible number of render workers for this machine: one fewer than the
/// cores, at most three.
pub fn default_worker_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).saturating_sub(1).clamp(1, 3)
}

impl Engine {
    /// Starts the render thread, loading PDFium from [`find_pdfium_dir`].
    pub fn start() -> Result<Self, EngineError> {
        let dir = find_pdfium_dir().ok_or(EngineError::LibraryNotFound)?;
        Self::start_with_library_dir(&dir)
    }

    pub fn start_with_library_dir(dir: &Path) -> Result<Self, EngineError> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            memory_limit: (DEFAULT_MEMORY_LIMIT_MB * 1_000_000).into(),
        });
        let (ready_tx, ready_rx) = mpsc::channel();
        let lib_dir = dir.to_path_buf();
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("edits-render".into())
            .spawn(move || {
                let pdfium = match load_library(&lib_dir) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                RenderThread { docs: Docs::new(pdfium), shared: thread_shared }.run();
            })
            .map_err(|e| EngineError::LibraryLoad(e.to_string()))?;

        ready_rx.recv().map_err(|_| EngineError::Stopped)??;
        Ok(Self { shared, thread: Mutex::new(Some(thread)), pdfium_dir: dir.to_path_buf() })
    }

    /// Lets heavy pages render in up to `max` worker processes started with
    /// `cmd`. Workers start when a heavy page is first seen and exit when idle.
    /// If a worker cannot be started, everything stays on the render thread.
    pub fn enable_workers(&self, cmd: WorkerCommand, max: usize) {
        let mut st = self.shared.lock();
        st.pool.cmd = (max > 0).then_some(cmd);
        st.pool.max = max;
        st.pool.pdfium_dir = Some(self.pdfium_dir.clone());
        if !st.pool.watchdog && max > 0 {
            let s = self.shared.clone();
            st.pool.watchdog =
                std::thread::Builder::new().name("edits-watchdog".into()).spawn(move || watchdog(s)).is_ok();
        }
    }

    /// Number of worker driver threads running (each with at most one process).
    pub fn workers(&self) -> usize {
        self.shared.lock().pool.drivers
    }

    /// True if page `page` of `doc` has been found slow to render.
    pub fn is_heavy(&self, doc: DocId, page: u32) -> bool {
        self.shared.lock().heavy.contains(&(doc, page))
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
        let offload = offloadable(&st.pool, &st.heavy, &st.sources, &job);
        st.queue.push(ticket, priority, job);
        if offload {
            ensure_drivers(&self.shared, &mut st);
        }
        drop(st);
        self.shared.wake.notify_all();
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
        {
            let mut st = self.shared.lock();
            st.sources.remove(&doc);
            st.heavy.retain(|(d, _)| *d != doc);
        }
        self.cancel_where(|s| s.doc == doc);
        self.submit(CONTROL_PRIORITY, |_| Job::Close { doc });
    }

    /// Queues a render. `reply` runs on the render thread or a worker
    /// driver thread, or on the cancelling thread with [`EngineError::Cancelled`].
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

    /// Runs `f` on the render thread with document `doc` and waits for its result.
    fn call<R: Send + 'static>(
        &self,
        doc: DocId,
        priority: Priority,
        f: impl FnOnce(&mut OpenDoc) -> Result<R, EngineError> + Send + 'static,
    ) -> Result<R, EngineError> {
        let (tx, rx) = mpsc::channel();
        let call: Call = Box::new(move |od| {
            let _ = tx.send(od.and_then(f));
        });
        self.submit(priority, |_| Job::Call { doc, call });
        rx.recv().map_err(|_| EngineError::Stopped)?
    }

    /// Page fractions (top-left origin, /Rotate applied) to PDF user space.
    pub fn to_user_space(&self, doc: DocId, page: u32, points: Vec<[f32; 2]>) -> Result<Vec<[f32; 2]>, EngineError> {
        self.call(doc, CONTROL_PRIORITY - 1, move |od| crate::annots::to_user_space(od.page(page)?, &points))
    }

    /// PDF user space to page fractions.
    pub fn to_fractions(&self, doc: DocId, page: u32, points: Vec<[f32; 2]>) -> Result<Vec<[f32; 2]>, EngineError> {
        self.call(doc, CONTROL_PRIORITY - 1, move |od| crate::annots::to_fractions(od.page(page)?, &points))
    }

    /// Shows the document with `tail`, an incremental update built by
    /// `edits_edit`, on top of its file (None: the file as it is). While a
    /// document has an update, its pages render on the render thread only;
    /// workers read the file without it.
    pub fn set_overlay(&self, doc: DocId, tail: Option<std::sync::Arc<Vec<u8>>>) -> Result<(), EngineError> {
        self.reopen(doc, None, Some(tail))
    }

    /// Like [`Engine::set_overlay`], for an update that adds, removes,
    /// reorders or reshapes pages: the text index is dropped and the new
    /// page list is returned.
    pub fn set_overlay_pages(&self, doc: DocId, tail: Option<std::sync::Arc<Vec<u8>>>) -> Result<DocInfo, EngineError> {
        if tail.is_some() {
            self.shared.lock().sources.remove(&doc);
        }
        let start = Instant::now();
        self.call(doc, CONTROL_PRIORITY, move |od| {
            od.tail = tail;
            od.texts.clear();
            od.reload()?;
            let (page_count, pages, broken_pages) = page_sizes(od);
            Ok(DocInfo { doc, page_count, pages, broken_pages, open_us: start.elapsed().as_micros() as u64 })
        })
    }

    /// The document's pages as they are now.
    pub fn info(&self, doc: DocId) -> Result<DocInfo, EngineError> {
        self.call(doc, CONTROL_PRIORITY, move |od| {
            let (page_count, pages, broken_pages) = page_sizes(od);
            Ok(DocInfo { doc, page_count, pages, broken_pages, open_us: 0 })
        })
    }

    /// Opens the document's file again after it was replaced by a save, or
    /// with `path`, the file it was saved as. Any update shown is dropped.
    pub fn reload(&self, doc: DocId, path: Option<PathBuf>) -> Result<(), EngineError> {
        self.reopen(doc, path, Some(None))
    }

    fn reopen(
        &self,
        doc: DocId,
        path: Option<PathBuf>,
        tail: Option<Option<std::sync::Arc<Vec<u8>>>>,
    ) -> Result<(), EngineError> {
        let shared = self.shared.clone();
        if matches!(tail, Some(Some(_))) {
            // Off the workers before the change shows.
            self.shared.lock().sources.remove(&doc);
        }
        self.call(doc, CONTROL_PRIORITY, move |od| {
            if let Some(p) = path {
                od.path = p;
            }
            if let Some(t) = tail {
                od.tail = t;
            }
            od.reload()?;
            if od.tail.is_some() {
                return Ok(());
            }
            let mut st = shared.lock();
            let generation = st.next_generation();
            st.sources.insert(
                doc,
                DocSource { path: od.path.clone(), password: od.password.clone(), stamp: od.stamp, generation },
            );
            Ok(())
        })
    }

    /// Writes the whole document, as PDFium holds it (unsaved annotations
    /// included), to `path`. For files an incremental save cannot handle.
    pub fn save_full(&self, doc: DocId, path: PathBuf) -> Result<(), EngineError> {
        self.call(doc, CONTROL_PRIORITY - 1, move |od| {
            od.doc.save_to_file(&path).map_err(|e| EngineError::Render(crate::docs::describe(&e)))
        })
    }

    /// The document's bookmarks. Empty if it has none.
    pub fn outline(&self, doc: DocId) -> Result<Vec<crate::nav::OutlineItem>, EngineError> {
        self.call(doc, CONTROL_PRIORITY - 1, |od| Ok(crate::nav::outline(&od.doc)))
    }

    /// The links on page `page`, with internal targets resolved to pages.
    pub fn links(&self, doc: DocId, page: u32, priority: Priority) -> Result<Vec<crate::nav::Link>, EngineError> {
        self.call(doc, priority, move |od| {
            od.page(page)?;
            // Borrow the cached page and the document together.
            let p = &od.pages.last().expect("page() just cached it").1;
            Ok(crate::nav::links(&od.doc, p))
        })
    }

    /// Page text with character boxes, for selection and highlights.
    pub fn page_text(&self, doc: DocId, page: u32, priority: Priority) -> Result<crate::text::PageText, EngineError> {
        self.call(doc, priority, move |od| {
            let text = crate::text::page_text(od.page(page)?);
            if od.texts[page as usize].is_none() {
                od.texts[page as usize] = Some(text.text.as_str().into());
            }
            Ok(text)
        })
    }

    /// Reads the text of up to `count` pages not yet in the index, starting
    /// at `from`. Returns how many pages are still missing afterwards.
    pub fn index_text(&self, doc: DocId, from: u32, count: u32, priority: Priority) -> Result<u32, EngineError> {
        self.call(doc, priority, move |od| {
            let n = od.page_count;
            let mut done = 0;
            for i in (from..n).chain(0..from.min(n)) {
                if done >= count {
                    break;
                }
                if od.texts[i as usize].is_none() {
                    // A page that cannot be read indexes as empty rather than stopping the index.
                    let _ = od.chars(i).map_err(|_| od.texts[i as usize] = Some("".into()));
                    done += 1;
                }
            }
            Ok(od.texts.iter().filter(|t| t.is_none()).count() as u32)
        })
    }

    /// Searches `pages`, reading text the index does not have yet.
    /// Returns, for each page with matches, its index and match ranges.
    pub fn search_pages(
        &self,
        doc: DocId,
        pages: Vec<u32>,
        query: String,
        options: crate::text::SearchOptions,
        priority: Priority,
    ) -> Result<Vec<PageHits>, EngineError> {
        self.call(doc, priority, move |od| {
            let mut out = Vec::new();
            let n = od.page_count;
            for p in pages.into_iter().filter(|p| *p < n) {
                let text = od.chars(p).unwrap_or_else(|_| "".into());
                let found = crate::text::find(&text, &query, &options, 10_000);
                if !found.is_empty() {
                    out.push((p, found));
                }
            }
            Ok(out)
        })
    }

    /// Cancels a queued job, or a render running in a worker. Returns false
    /// if it already finished or runs on the render thread.
    pub fn cancel(&self, ticket: Ticket) -> bool {
        let mut st = self.shared.lock();
        let job = st.queue.remove(ticket);
        match job {
            Some(job) => {
                drop(st);
                job.fail(EngineError::Cancelled);
                true
            }
            None => st.running.get_mut(&ticket).map(cancel_running).is_some(),
        }
    }

    /// Cancels every queued render whose spec matches, and stops matching
    /// renders running in workers. Returns how many were cancelled.
    pub fn cancel_where(&self, mut pred: impl FnMut(&RenderSpec) -> bool) -> usize {
        let mut st = self.shared.lock();
        let drained = st.queue.drain_where(|job| matches!(job, Job::Render { spec, .. } if pred(spec)));
        let mut stopped = 0;
        for r in st.running.values_mut().filter(|r| r.stopped.is_none() && pred(&r.spec)) {
            cancel_running(r);
            stopped += 1;
        }
        drop(st);
        let n = drained.len() + stopped;
        for (_, job) in drained {
            job.fail(EngineError::Cancelled);
        }
        n
    }

    /// Changes the priority of a queued job. Returns false if it is not queued.
    pub fn set_priority(&self, ticket: Ticket, priority: Priority) -> bool {
        self.shared.lock().queue.set_priority(ticket, priority)
    }

    /// Sets the process memory above which open documents are reopened to
    /// release PDFium's caches. PDFium keeps every object it has read,
    /// including image data, until a document is closed.
    pub fn set_memory_limit_mb(&self, mb: u64) {
        self.shared.memory_limit.store(mb * 1_000_000, std::sync::atomic::Ordering::Relaxed);
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
            for r in st.running.values_mut() {
                r.stopped = Some(EngineError::Stopped);
                if let Some(c) = &r.child {
                    crate::worker::kill(c);
                }
            }
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

struct RenderThread {
    docs: Docs,
    shared: Arc<Shared>,
}

impl RenderThread {
    fn run(mut self) {
        loop {
            let job = {
                let mut st = self.shared.lock();
                loop {
                    if st.stopping {
                        return;
                    }
                    // Heavy renders are left for the workers, if there are any.
                    let State { queue, heavy, sources, pool, .. } = &mut *st;
                    if let Some((_, _, job)) = queue.pop_where(|j| !offloadable(pool, heavy, sources, j)) {
                        break job;
                    }
                    st = self.shared.wake.wait(st).unwrap_or_else(|e| e.into_inner());
                }
            };
            // A panic inside PDFium bindings must not take the thread down.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(job)));
            if outcome.is_err() {
                log::error!("render job panicked");
            }
            self.docs.trim(self.shared.memory_limit.load(std::sync::atomic::Ordering::Relaxed));
        }
    }

    fn handle(&mut self, job: Job) {
        match job {
            Job::Open { doc, path, password, reply } => reply(self.open(doc, &path, password.as_deref())),
            Job::Close { doc } => self.docs.close(doc),
            Job::Call { doc, call } => match self.docs.docs.get_mut(&doc) {
                Some(od) => call(Ok(od)),
                None => call(Err(EngineError::UnknownDoc(doc))),
            },
            Job::Render { spec, submitted, reply } => {
                let queued_us = submitted.elapsed().as_micros() as u64;
                let start = Instant::now();
                let result = self.docs.render(&spec);
                let total_us = start.elapsed().as_micros() as u64;
                if total_us > HEAVY_RENDER_US {
                    // Later renders of this page go to workers, including
                    // the ones already queued.
                    let mut st = self.shared.lock();
                    if st.heavy.insert((spec.doc, spec.page)) {
                        ensure_drivers(&self.shared, &mut st);
                        drop(st);
                        self.shared.wake.notify_all();
                    }
                }
                reply(result.map(|(w, h, rgba, load_us)| Tile {
                    spec,
                    w,
                    h,
                    rgba,
                    queued_us,
                    render_us: total_us.saturating_sub(load_us),
                }))
            }
        }
    }

    fn open(&mut self, id: DocId, path: &Path, password: Option<&str>) -> Result<DocInfo, EngineError> {
        let start = Instant::now();
        let od = self.docs.load(id, path, password)?;
        let (page_count, pages, broken_pages) = page_sizes(od);
        let source = DocSource {
            path: path.to_path_buf(),
            password: password.map(str::to_string),
            stamp: od.stamp,
            generation: 0,
        };
        self.shared.lock().sources.insert(id, source);
        Ok(DocInfo { doc: id, page_count, pages, broken_pages, open_us: start.elapsed().as_micros() as u64 })
    }
}

/// Page count and sizes. Read one page at a time: a single broken page must
/// not make the whole document unreadable. It gets the previous page's size
/// (or A4) and is reported, and rendering it later fails on its own.
fn page_sizes(od: &OpenDoc) -> (u32, Vec<PageSize>, Vec<u32>) {
    let page_count = od.page_count;
    let mut pages = Vec::with_capacity(page_count as usize);
    let mut broken_pages = Vec::new();
    for i in 0..page_count {
        let size = od
            .doc
            .pages()
            .page_size(i as pdfium_render::prelude::PdfPageIndex)
            .ok()
            .map(|r| PageSize { w: r.width().value, h: r.height().value })
            .filter(|s| s.w.is_finite() && s.h.is_finite() && s.w >= 1.0 && s.h >= 1.0);
        pages.push(size.unwrap_or_else(|| {
            broken_pages.push(i);
            pages.last().copied().unwrap_or(PageSize { w: 595.0, h: 842.0 })
        }));
    }
    (page_count, pages, broken_pages)
}
