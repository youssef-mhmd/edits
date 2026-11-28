//! Core of edits: PDFium loading, the render thread and its priority queue.
//!
//! PDFium allows one thread at a time, so every PDFium call in the app's
//! process happens on one dedicated render thread owned by [`Engine`].
//! Callers submit jobs with a priority and get results through callbacks;
//! queued jobs can be cancelled or re-prioritised at any time. Pages that
//! are slow to draw are rendered in parallel by worker processes
//! ([`worker`]).

mod annots;
mod docs;
mod engine;
mod library;
mod nav;
mod queue;
mod reading;
mod text;
pub mod worker;

pub use engine::{
    DEFAULT_MEMORY_LIMIT_MB, DocId, DocInfo, Engine, EngineError, HEAVY_RENDER_US, MAX_TILE_SIDE, PageHits, PageSize,
    RenderSpec, Ticket, Tile, default_worker_count,
};
pub use library::{PDFIUM_DIR_ENV, find_pdfium_dir};
pub use nav::{Link, OutlineItem, Target};
pub use queue::Priority;
pub use text::{PageText, SearchOptions, find, normalize};
