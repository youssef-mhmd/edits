//! Editing: annotations as data, the undo history, appearance streams, and
//! a small PDF reader and incremental-update writer. No PDFium here: PDFium
//! renders the base file with the update this crate builds on top.

pub mod ap;
pub mod model;
pub mod pdf;
pub mod session;

pub use model::{Annot, AnnotId, Kind, pdf_date_now};
pub use session::{EditError, Listed, Session, Status, temp_path_for};
