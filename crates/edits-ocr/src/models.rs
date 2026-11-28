//! The two ONNX models, run with ONNX Runtime on the CPU (arabic-ocr
//! detect/detector.py and recognize/recognizer.py).
//!
//! onnxruntime.dll is loaded at run time, on the first OCR, so starting edits
//! never pays for it.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ort::ep;
use ort::session::Session;
use ort::value::Tensor;

use crate::charset;
use crate::ctc::{self, Decoded};
use crate::detect::{self, Detection, PreparedPage};
use crate::image::{self, Gray, py_round};
use crate::{Error, Result};

static RUNTIME: OnceLock<std::result::Result<(), String>> = OnceLock::new();

/// Load onnxruntime.dll once per process.
pub fn init_runtime(dll: &Path) -> Result<()> {
    RUNTIME
        .get_or_init(|| {
            let builder = ort::init_from(dll).map_err(|e| format!("{}: {e}", dll.display()))?;
            builder.with_name("edits-ocr").commit();
            Ok(())
        })
        .clone()
        .map_err(Error::Runtime)
}

fn session(path: &Path, threads: usize) -> Result<Session> {
    let err = |e: &dyn std::fmt::Display| Error::Model(format!("{}: {e}", path.display()));
    // No memory arena: pages and lines all differ in size, so an arena keeps
    // growing (arabic-ocr measured 600+ MB after 20 pages); about 2% slower.
    let cpu = ep::CPU::default().with_arena_allocator(false).build();
    Session::builder()
        .map_err(|e| err(&e))?
        .with_execution_providers([cpu])
        .map_err(|e| err(&e))?
        .with_memory_pattern(false)
        .map_err(|e| err(&e))?
        .with_intra_threads(threads)
        .map_err(|e| err(&e))?
        .commit_from_file(path)
        .map_err(|e| err(&e))
}

fn meta(s: &Session, key: &str) -> Option<String> {
    s.metadata().ok().and_then(|m| m.custom(key))
}

pub struct Detector {
    session: Session,
    pub max_side: usize,
}

impl Detector {
    pub fn load(path: &Path, threads: usize) -> Result<Detector> {
        let session = session(path, threads)?;
        let max_side = meta(&session, "max_side").and_then(|v| v.parse().ok()).unwrap_or(detect::MAX_SIDE);
        Ok(Detector { session, max_side })
    }

    /// Raw core map of a prepared page (its full padded size, row-major).
    pub fn probability_map(&mut self, page: &PreparedPage) -> Result<Vec<f32>> {
        let (w, h) = (page.image.width, page.image.height);
        let input = Tensor::from_array(([1usize, 1, h, w], page.image.data.clone()))
            .map_err(|e| Error::Model(e.to_string()))?;
        let out = self.session.run(ort::inputs![input]).map_err(|e| Error::Model(e.to_string()))?;
        let (_, data) = out[0].try_extract_tensor::<f32>().map_err(|e| Error::Model(e.to_string()))?;
        Ok(data.to_vec())
    }

    /// Line boxes on a page image.
    pub fn detect(&mut self, gray: &Gray) -> Result<Vec<Detection>> {
        let page = detect::prepare_page(gray, self.max_side);
        let prob = self.probability_map(&page)?;
        Ok(detect::boxes_from_prob(&prob, page.image.width, &page))
    }
}

/// Narrower lines are padded up to this width (as in training).
pub const MIN_WIDTH: usize = 32;

pub struct Recognizer {
    /// One session per worker thread (Session::run needs exclusive access);
    /// each uses one thread, since a line is too small a job to split.
    sessions: Vec<Mutex<Session>>,
    path: PathBuf,
    pub height: usize,
    pub stride: usize,
}

impl Recognizer {
    pub fn load(path: &Path) -> Result<Recognizer> {
        let first = session(path, 1)?;
        if let Some(fp) = meta(&first, "charset_fingerprint")
            && fp != charset::FINGERPRINT
        {
            return Err(Error::Model(format!(
                "{} was trained with a different charset ({fp}, this build has {})",
                path.display(),
                charset::FINGERPRINT
            )));
        }
        let height = meta(&first, "input_height").and_then(|v| v.parse().ok()).unwrap_or(64);
        let stride = meta(&first, "width_stride").and_then(|v| v.parse().ok()).unwrap_or(4);
        Ok(Recognizer { sessions: vec![Mutex::new(first)], path: path.to_owned(), height, stride })
    }

    /// Add sessions up to `workers` (loaded when first needed).
    fn ensure_workers(&mut self, workers: usize) -> Result<()> {
        while self.sessions.len() < workers.max(1) {
            self.sessions.push(Mutex::new(session(&self.path, 1)?));
        }
        Ok(())
    }

    /// Grayscale line -> model height, keeping proportions (area when
    /// shrinking, cubic when growing), then padded on the LEFT (the end of a
    /// right-to-left line) by repeating the edge, to a multiple of the stride.
    pub fn prepare_line(&self, line: &Gray) -> Gray {
        let h = self.height;
        let g = if line.height != h {
            let nw = py_round(line.width as f64 * h as f64 / line.height as f64).max(1) as usize;
            if line.height > h { image::resize_area(line, nw, h) } else { image::resize_cubic(line, nw, h) }
        } else {
            line.clone()
        };
        let width = g.width.max(MIN_WIDTH).div_ceil(self.stride) * self.stride;
        let pad = width - g.width;
        let mut out = Gray::new(width, h, 0);
        for y in 0..h {
            let src = g.row(y);
            let dst = &mut out.data[y * width..(y + 1) * width];
            dst[..pad].fill(src[0]);
            dst[pad..].copy_from_slice(src);
        }
        out
    }

    /// Read line images; results come back in the same order. Lines are only
    /// batched with lines of exactly the same padded width, so a line's result
    /// never depends on the others.
    pub fn recognize_batch(&mut self, lines: &[Gray], workers: usize, max_batch: usize) -> Result<Vec<Decoded>> {
        let prepared: Vec<Gray> = lines.iter().map(|l| self.prepare_line(l)).collect();
        let mut by_width: Vec<(usize, Vec<usize>)> = Vec::new();
        for (i, l) in prepared.iter().enumerate() {
            match by_width.iter_mut().find(|(w, _)| *w == l.width) {
                Some((_, v)) => v.push(i),
                None => by_width.push((l.width, vec![i])),
            }
        }
        let chunks: Vec<(usize, Vec<usize>)> =
            by_width.iter().flat_map(|(w, idx)| idx.chunks(max_batch).map(move |c| (*w, c.to_vec()))).collect();
        let workers = workers.clamp(1, chunks.len().max(1));
        self.ensure_workers(workers)?;
        let (height, stride) = (self.height, self.stride);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let results: Mutex<Vec<Option<Decoded>>> = Mutex::new(vec![None; lines.len()]);
        let failure: Mutex<Option<Error>> = Mutex::new(None);
        let run_chunk = |session: &Mutex<Session>, (width, idx): &(usize, Vec<usize>)| -> Result<()> {
            let mut batch = Vec::with_capacity(idx.len() * width * height);
            for &i in idx {
                batch.extend_from_slice(&prepared[i].data);
            }
            let input =
                Tensor::from_array(([idx.len(), 1, height, *width], batch)).map_err(|e| Error::Model(e.to_string()))?;
            let mut s = session.lock().map_err(|_| Error::Model("recognizer poisoned".into()))?;
            let out = s.run(ort::inputs![input]).map_err(|e| Error::Model(e.to_string()))?;
            let (shape, data) = out[0].try_extract_tensor::<f32>().map_err(|e| Error::Model(e.to_string()))?;
            // (T, B, classes) log-probabilities.
            let (t, b, c) = (shape[0] as usize, shape[1] as usize, shape[2] as usize);
            let steps = (width / stride).min(t);
            let mut res = results.lock().map_err(|_| Error::Model("results poisoned".into()))?;
            for (k, &i) in idx.iter().enumerate().take(b) {
                res[i] = Some(ctc::greedy_decode(&data[k * c..], steps, b * c, c));
            }
            Ok(())
        };
        std::thread::scope(|scope| {
            for session in self.sessions.iter().take(workers) {
                let (chunks, next, failure, run_chunk) = (&chunks, &next, &failure, &run_chunk);
                scope.spawn(move || {
                    loop {
                        let j = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(chunk) = chunks.get(j) else { break };
                        if let Err(e) = run_chunk(session, chunk)
                            && let Ok(mut f) = failure.lock()
                        {
                            f.get_or_insert(e);
                        }
                    }
                });
            }
        });
        if let Some(e) = failure.into_inner().ok().flatten() {
            return Err(e);
        }
        Ok(results.into_inner().unwrap_or_default().into_iter().map(Option::unwrap_or_default).collect())
    }
}
