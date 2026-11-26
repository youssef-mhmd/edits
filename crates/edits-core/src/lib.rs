//! Core of edits: PDFium loading, the render thread and its priority queue.
//!
//! PDFium allows one thread at a time, so every PDFium call happens on one
//! dedicated render thread owned by [`Engine`]. Callers submit jobs with a
//! priority and get results through callbacks; queued jobs can be cancelled or
//! re-prioritised at any time.

mod engine;
mod library;
mod nav;
mod queue;

pub use engine::{DocId, DocInfo, Engine, EngineError, PageSize, RenderSpec, Ticket, Tile};
pub use library::{PDFIUM_DIR_ENV, find_pdfium_dir};
pub use nav::{Link, OutlineItem, Target};
pub use queue::Priority;
