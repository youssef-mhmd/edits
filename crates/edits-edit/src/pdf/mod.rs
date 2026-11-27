//! A small, lazy PDF reader and an incremental-update writer, in safe Rust.
//! Enough to read pages and annotations from files of any size and to
//! append changes to them; rendering stays with PDFium.

pub mod copy;
pub mod crypt;
pub mod file;
pub mod object;
pub mod write;

pub use file::{FileSource, Layered, PdfError, PdfFile, Source};
pub use object::{Dict, Obj, Ref};
