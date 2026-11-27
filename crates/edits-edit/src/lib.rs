//! Editing: annotations as data, the undo history, appearance streams and
//! incremental save. No PDFium here; `edits-core` keeps PDFium's copy of the
//! document in step (see [`edits::LayerOp`]).

pub mod ap;
pub mod edits;
pub mod model;
pub mod save;

pub use edits::{Edits, LayerOp, Listed, Status};
pub use model::{Annot, AnnotId, Kind, ObjRef, pdf_date_now};
pub use save::{PageChange, SaveError, SaveOutcome, save_incremental};
