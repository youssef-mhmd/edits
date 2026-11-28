//! edits-ocr: a Rust port of the arabic-ocr pipeline (PRD 8.5).
//!
//! The models (detector and line recognizer, ONNX) are arabic-ocr's own,
//! unchanged; everything around them (preprocessing, line boxes, cropping,
//! CTC decoding, reading order, bidi, word correction) is ported from its
//! Python source so that both give the same text and boxes. The parity check
//! is `edits-bench ocr-parity`, against output stored by scripts/ocr-golden.py.
//!
//! ```text
//! let mut ocr = edits_ocr::Engine::new(edits_ocr::Paths::beside_exe());
//! let page = ocr.read(&gray, &edits_ocr::Options::default())?;
//! println!("{}", page.text());
//! ```
//!
//! Models and onnxruntime.dll are loaded on the first `read`.

pub mod bidi;
pub mod charset;
pub mod correct;
pub mod ctc;
pub mod detect;
pub mod image;
pub mod json;
pub mod layout;
pub mod models;
pub mod pipeline;
pub mod preprocess;

use std::path::{Path, PathBuf};

pub use charset::Digits;
pub use image::Gray;
pub use pipeline::{Block, Line, Options, Page, Word};

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("ONNX Runtime could not be loaded: {0}")]
    Runtime(String),
    #[error("OCR model: {0}")]
    Model(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub const DETECTOR: &str = "detector-v2.onnx";
pub const RECOGNIZER: &str = "recognizer-v6b-int8.onnx";
pub const LEXICON: &str = "lexicon.tsv.gz";
pub const RUNTIME_DLL: &str = "onnxruntime.dll";

/// Where the runtime and the models are.
#[derive(Clone, Debug)]
pub struct Paths {
    pub runtime: PathBuf,
    pub models: PathBuf,
}

impl Paths {
    /// A folder holding onnxruntime.dll and the models.
    pub fn in_dir(dir: &Path) -> Paths {
        Paths { runtime: dir.join(RUNTIME_DLL), models: dir.to_owned() }
    }

    /// $EDITS_OCR_DIR, else `ocr` beside the executable, else vendor/ocr in
    /// the source tree (development builds).
    pub fn beside_exe() -> Paths {
        if let Some(d) = std::env::var_os("EDITS_OCR_DIR") {
            return Paths::in_dir(Path::new(&d));
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
        {
            let d = dir.join("ocr");
            if d.join(DETECTOR).is_file() {
                return Paths::in_dir(&d);
            }
        }
        Paths::in_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/ocr"))
    }

    pub fn available(&self) -> bool {
        self.runtime.is_file() && self.models.join(DETECTOR).is_file() && self.models.join(RECOGNIZER).is_file()
    }
}

/// Loads everything on first use, then reads pages.
pub struct Engine {
    paths: Paths,
    threads: usize,
    ocr: Option<pipeline::Ocr>,
    lexicon_tried: bool,
}

impl Engine {
    pub fn new(paths: Paths) -> Engine {
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).min(8);
        Engine { paths, threads, ocr: None, lexicon_tried: false }
    }

    /// Threads for the detector and lines read at the same time.
    pub fn set_threads(&mut self, n: usize) {
        self.threads = n.max(1);
        if let Some(o) = &mut self.ocr {
            o.workers = self.threads;
        }
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn loaded(&self) -> bool {
        self.ocr.is_some()
    }

    /// Load the runtime and models now (otherwise done by the first read).
    pub fn load(&mut self) -> Result<&mut pipeline::Ocr> {
        if self.ocr.is_none() {
            models::init_runtime(&self.paths.runtime)?;
            let detector = models::Detector::load(&self.paths.models.join(DETECTOR), self.threads)?;
            let recognizer = models::Recognizer::load(&self.paths.models.join(RECOGNIZER))?;
            self.ocr = Some(pipeline::Ocr { detector, recognizer, lexicon: None, workers: self.threads });
        }
        Ok(self.ocr.as_mut().expect("loaded above"))
    }

    pub fn read(&mut self, gray: &Gray, options: &Options) -> Result<Page> {
        let want_lexicon = options.correct_words && !self.lexicon_tried;
        let lexicon_path = self.paths.models.join(LEXICON);
        let ocr = self.load()?;
        if want_lexicon {
            ocr.lexicon = correct::Lexicon::load(&lexicon_path).ok();
        }
        if options.correct_words {
            self.lexicon_tried = true;
        }
        self.ocr.as_mut().expect("loaded above").read(gray, options)
    }

    /// Free the models (they hold about 100 MB); the next read loads them again.
    pub fn unload(&mut self) {
        self.ocr = None;
        self.lexicon_tried = false;
    }
}
