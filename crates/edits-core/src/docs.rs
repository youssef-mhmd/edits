//! Open documents and rendering, shared by the render thread and the render
//! worker processes. Everything here must run on the one thread that owns
//! the PDFium handle.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use pdfium_render::prelude::*;

use crate::engine::{DocId, EngineError, RenderSpec};

/// Pages kept loaded per document. Loading a page parses its content, so the
/// pages being scrolled through stay loaded while their tiles render.
const PAGE_CACHE: usize = 12;
/// How many newly loaded pages between memory checks.
const PAGES_PER_MEMORY_CHECK: u32 = 16;
/// Reopen only if memory grew at least this much since the last (re)open, so
/// a file that is simply large once open is not reopened over and over.
const MIN_GROWTH_TO_REOPEN: u64 = 32_000_000;

/// File size and modification time, to notice a file that changed on disk.
pub(crate) type Stamp = (u64, SystemTime);

pub(crate) fn file_stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()?))
}

/// Loads PDFium from `dir`. The handle lives as long as the program: every
/// document opened from it must not outlive it.
pub(crate) fn load_library(dir: &Path) -> Result<&'static Pdfium, EngineError> {
    let lib_path = Pdfium::pdfium_platform_library_name_at_path(dir);
    let pdfium = match Pdfium::bind_to_library(&lib_path) {
        Ok(bindings) => Pdfium::new(bindings),
        // Another engine in this process loaded it already; share it.
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Pdfium::default(),
        Err(e) => return Err(EngineError::LibraryLoad(e.to_string())),
    };
    Ok(Box::leak(Box::new(pdfium)))
}

/// A short, readable reason for a PDFium failure, for display on the page.
pub(crate) fn describe(e: &PdfiumError) -> String {
    match e {
        PdfiumError::PdfiumLibraryInternalError(i) => match i {
            PdfiumInternalError::FileError => "the file could not be read",
            PdfiumInternalError::PasswordError => "a password is needed",
            PdfiumInternalError::SecurityError => "the file uses an unsupported security handler",
            _ => "the page is damaged",
        }
        .to_string(),
        PdfiumError::IoError(e) => e.to_string(),
        other => format!("{other:?}").split_whitespace().collect::<Vec<_>>().join(" "),
    }
}

fn open_error(e: PdfiumError) -> EngineError {
    match e {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => EngineError::PasswordRequired,
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FormatError) => {
            EngineError::Open("the file is damaged or is not a PDF".into())
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FileError) => {
            EngineError::Open("the file could not be read".into())
        }
        PdfiumError::IoError(e) => EngineError::Open(e.to_string()),
        other => EngineError::Open(describe(&other)),
    }
}

pub(crate) struct OpenDoc {
    // Declared before `doc` so pages drop first.
    pub pages: Vec<(u32, PdfPage<'static>)>,
    pub doc: PdfDocument<'static>,
    pub page_count: u32,
    pub path: PathBuf,
    pub password: Option<String>,
    /// File stamp at open; a reopen is skipped if the file has changed since.
    pub stamp: Option<Stamp>,
    /// Pages loaded since the document was (re)opened.
    pub loads: u32,
    /// Text of each page once read (the text index). Kept across reopens.
    /// Only the render thread fills it.
    pub texts: Vec<Option<std::sync::Arc<str>>>,
    /// Holds edits not in the file yet: never reopen it to free memory.
    pub dirty: bool,
    pdfium: &'static Pdfium,
}

impl OpenDoc {
    pub fn page(&mut self, index: u32) -> Result<&PdfPage<'static>, EngineError> {
        if index >= self.page_count {
            return Err(EngineError::PageOutOfRange(index));
        }
        if let Some(pos) = self.pages.iter().position(|(i, _)| *i == index) {
            // Move to the back: most recently used.
            let entry = self.pages.remove(pos);
            self.pages.push(entry);
        } else {
            let page = self.doc.pages().get(index as PdfPageIndex).map_err(|e| EngineError::Render(describe(&e)))?;
            if self.pages.len() >= PAGE_CACHE {
                self.pages.remove(0);
            }
            self.pages.push((index, page));
            self.loads += 1;
        }
        Ok(&self.pages.last().expect("just pushed").1)
    }

    /// The page, loaded if needed, for changing it.
    pub fn page_mut(&mut self, index: u32) -> Result<&mut PdfPage<'static>, EngineError> {
        self.page(index)?;
        Ok(&mut self.pages.last_mut().expect("page() just cached it").1)
    }

    /// Opens the file again (after it was saved), dropping PDFium's copy
    /// and any edits not saved. The text index is kept.
    pub fn reload(&mut self) -> Result<(), EngineError> {
        let doc = self.pdfium.load_pdf_from_file(&self.path, self.password.as_deref()).map_err(open_error)?;
        self.pages.clear();
        self.doc = doc;
        self.page_count = self.doc.pages().len() as u32;
        self.texts.resize(self.page_count as usize, None);
        self.stamp = file_stamp(&self.path);
        self.loads = 0;
        self.dirty = false;
        Ok(())
    }

    /// The page's characters, from the index or read now and stored.
    pub fn chars(&mut self, index: u32) -> Result<std::sync::Arc<str>, EngineError> {
        if let Some(Some(t)) = self.texts.get(index as usize) {
            return Ok(t.clone());
        }
        let text: std::sync::Arc<str> = crate::text::page_chars(self.page(index)?).into();
        self.texts[index as usize] = Some(text.clone());
        Ok(text)
    }
}

/// Private memory of this process, in bytes.
pub(crate) fn process_memory() -> u64 {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let Ok(mut sys) = System::new() else { return 0 };
    let pid = Pid::from_u32(std::process::id());
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing().with_memory());
    // Windows: private bytes. Elsewhere sysinfo's virtual size is not useful; use resident.
    sys.process(pid).map(|p| if cfg!(windows) { p.virtual_memory() } else { p.memory() }).unwrap_or(0)
}

/// The documents open in this process.
pub(crate) struct Docs {
    pdfium: &'static Pdfium,
    pub docs: HashMap<DocId, OpenDoc>,
    loads_since_check: u32,
    /// Process memory right after the last open or reopen.
    baseline: u64,
}

impl Docs {
    pub fn new(pdfium: &'static Pdfium) -> Self {
        Self { pdfium, docs: HashMap::new(), loads_since_check: 0, baseline: 0 }
    }

    /// Opens `path` as document `id`.
    pub fn load(&mut self, id: DocId, path: &Path, password: Option<&str>) -> Result<&mut OpenDoc, EngineError> {
        let doc = self.pdfium.load_pdf_from_file(path, password).map_err(open_error)?;
        let page_count = doc.pages().len() as u32;
        self.baseline = process_memory();
        let od = OpenDoc {
            pages: Vec::new(),
            doc,
            page_count,
            path: path.to_path_buf(),
            password: password.map(str::to_string),
            stamp: file_stamp(path),
            loads: 0,
            texts: vec![None; page_count as usize],
            dirty: false,
            pdfium: self.pdfium,
        };
        self.docs.insert(id, od);
        Ok(self.docs.get_mut(&id).expect("just inserted"))
    }

    pub fn close(&mut self, id: DocId) {
        self.docs.remove(&id);
    }

    /// PDFium keeps every object it has parsed, image data included, for the
    /// life of a document, so reading through a large scanned or image-heavy
    /// file would hold most of the file in memory. Every few page loads,
    /// check memory; over the limit, reopen the documents that have loaded
    /// pages, which frees all of it. Reopening reads only the file's
    /// cross-reference data (milliseconds), and pages reload on demand.
    pub fn trim(&mut self, limit: u64) {
        let loads: u32 = self.docs.values().map(|d| d.loads).sum();
        if loads < self.loads_since_check + PAGES_PER_MEMORY_CHECK {
            return;
        }
        self.loads_since_check = loads;
        let now = process_memory();
        if now <= limit || now < self.baseline + MIN_GROWTH_TO_REOPEN {
            return;
        }
        for od in self.docs.values_mut().filter(|d| d.loads > 0 && !d.dirty) {
            if file_stamp(&od.path) != od.stamp {
                continue; // changed on disk: keep what we have rather than show a different file
            }
            od.pages.clear();
            match self.pdfium.load_pdf_from_file(&od.path, od.password.as_deref()) {
                Ok(doc) => {
                    od.doc = doc;
                    od.loads = 0;
                }
                Err(e) => log::warn!("reopen of {} failed: {}", od.path.display(), describe(&e)),
            }
        }
        self.loads_since_check = self.docs.values().map(|d| d.loads).sum();
        self.baseline = process_memory();
    }

    /// Renders `spec`: returns the actual width, height, RGBA bytes, and the
    /// microseconds spent loading the page (0 if it was loaded already).
    pub fn render(&mut self, spec: &RenderSpec) -> Result<(u32, u32, Vec<u8>, u64), EngineError> {
        let doc = self.docs.get_mut(&spec.doc).ok_or(EngineError::UnknownDoc(spec.doc))?;
        let loading = std::time::Instant::now();
        let page = doc.page(spec.page)?;
        let load_us = loading.elapsed().as_micros() as u64;
        let w = spec.w.min(spec.page_w - spec.x);
        let h = spec.h.min(spec.page_h - spec.y);
        let render_err = |e: PdfiumError| EngineError::Render(describe(&e));

        let mut bitmap = PdfBitmap::empty(w as Pixels, h as Pixels, PdfBitmapFormat::BGRA).map_err(render_err)?;
        let config = PdfRenderConfig::new()
            .set_fixed_size(spec.page_w as Pixels, spec.page_h as Pixels)
            .set_origin(-(spec.x as Pixels), -(spec.y as Pixels))
            .set_format(PdfBitmapFormat::BGRA)
            .set_reverse_byte_order(true)
            .render_annotations(true)
            .render_form_data(true)
            // Keeps PDFium from holding every decoded image it has drawn;
            // without it, background previews of image-heavy files grow the
            // process by about 100 MB per 1,000 pages.
            .limit_render_image_cache_size(true)
            .rotate(
                match spec.rotation {
                    1 => PdfPageRenderRotation::Degrees90,
                    2 => PdfPageRenderRotation::Degrees180,
                    3 => PdfPageRenderRotation::Degrees270,
                    _ => PdfPageRenderRotation::None,
                },
                false,
            );
        // The tile is clipped to the page above, so the white clear that
        // pdfium-render applies to the page rectangle covers the whole bitmap.
        page.render_into_bitmap_with_config(&mut bitmap, &config).map_err(render_err)?;
        Ok((w, h, bitmap.as_raw_bytes(), load_us))
    }
}
