//! Render worker processes.
//!
//! PDFium runs one thread per process, so pages that take long to draw (many
//! thousands of vector paths) are rendered in separate processes, in
//! parallel. A worker is the app's own executable started with
//! [`WORKER_ARG`]; it opens its own copy of each document and renders the
//! tiles it is sent. The render thread keeps everything else.
//!
//! A worker that crashes or hangs on a damaged page takes only that page's
//! tile with it. A render that is no longer wanted is stopped by ending the
//! worker; a new one starts on the next job.
//!
//! Protocol, over the worker's stdin and stdout: frames of a little-endian
//! u32 length and a body. Requests are JSON [`Request`]s. Each request gets
//! one JSON [`Reply`]; a [`Reply::Tile`] is followed by one more frame with
//! the RGBA bytes.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::docs::{Docs, Stamp, file_stamp, load_library};
use crate::engine::{DEFAULT_MEMORY_LIMIT_MB, DocId, EngineError, RenderSpec};
use crate::library::find_pdfium_dir;

/// First argument that turns the executable into a render worker.
pub const WORKER_ARG: &str = "--edits-render-worker";

/// Largest frame accepted (a 4096 x 4096 RGBA tile plus room).
const MAX_FRAME: usize = 4096 * 4096 * 4 + 1024;

#[derive(Serialize, Deserialize, Debug)]
pub(crate) enum Request {
    /// `stamp` is the file's size and modification time when the render
    /// thread opened it; a worker refuses a file that has changed since.
    Open {
        doc: DocId,
        path: PathBuf,
        password: Option<String>,
        stamp: Option<(u64, u64, u32)>,
    },
    Close {
        doc: DocId,
    },
    Render {
        spec: RenderSpec,
    },
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) enum Reply {
    Ok,
    Err(String),
    Tile { w: u32, h: u32, render_us: u64 },
}

pub(crate) fn stamp_wire(s: Option<Stamp>) -> Option<(u64, u64, u32)> {
    let (len, t) = s?;
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((len, d.as_secs(), d.subsec_nanos()))
}

fn write_frame(w: &mut impl Write, body: &[u8]) -> std::io::Result<()> {
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(body)
}

fn read_frame(r: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(body)
}

fn send_json(w: &mut impl Write, v: &impl Serialize) -> std::io::Result<()> {
    let body = serde_json::to_vec(v).map_err(std::io::Error::other)?;
    write_frame(w, &body)
}

fn read_json<T: for<'de> Deserialize<'de>>(r: &mut impl Read) -> std::io::Result<T> {
    let body = read_frame(r)?;
    serde_json::from_slice(&body).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// If this process was started as a render worker, serves requests until
/// its input closes and returns true; the caller should then exit. Call it
/// first thing in `main`.
pub fn serve_if_requested() -> bool {
    if std::env::args_os().nth(1).is_none_or(|a| a != WORKER_ARG) {
        return false;
    }
    serve();
    true
}

fn serve() {
    let Some(pdfium) = find_pdfium_dir().and_then(|d| load_library(&d).ok()) else {
        return;
    };
    let mut docs = Docs::new(pdfium);
    // Requests are read on their own thread so that the worker ends as soon
    // as the app goes away, even in the middle of a long render.
    let (tx, rx) = std::sync::mpsc::sync_channel::<Request>(16);
    std::thread::spawn(move || {
        let mut input = BufReader::new(std::io::stdin().lock());
        loop {
            match read_json::<Request>(&mut input) {
                Ok(req) => {
                    if tx.send(req).is_err() {
                        break;
                    }
                }
                Err(_) => std::process::exit(0),
            }
        }
    });
    let mut out = BufWriter::new(std::io::stdout().lock());
    let limit = DEFAULT_MEMORY_LIMIT_MB * 1_000_000;
    for req in rx {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&mut docs, req)));
        let ok = match result {
            Ok((reply, pixels)) => send_json(&mut out, &reply)
                .and_then(|()| pixels.map_or(Ok(()), |p| write_frame(&mut out, &p)))
                .and_then(|()| out.flush()),
            Err(_) => send_json(&mut out, &Reply::Err("the page could not be drawn".into())).and_then(|()| out.flush()),
        };
        if ok.is_err() {
            break;
        }
        docs.trim(limit);
    }
}

fn handle(docs: &mut Docs, req: Request) -> (Reply, Option<Vec<u8>>) {
    match req {
        Request::Open { doc, path, password, stamp } => {
            if stamp_wire(file_stamp(&path)) != stamp {
                return (Reply::Err("the file changed on disk".into()), None);
            }
            match docs.load(doc, &path, password.as_deref()) {
                Ok(_) => (Reply::Ok, None),
                Err(e) => (Reply::Err(e.to_string()), None),
            }
        }
        Request::Close { doc } => {
            docs.close(doc);
            (Reply::Ok, None)
        }
        Request::Render { spec } => {
            let start = std::time::Instant::now();
            match docs.render(&spec) {
                Ok((w, h, rgba, load_us)) => {
                    let render_us = (start.elapsed().as_micros() as u64).saturating_sub(load_us);
                    (Reply::Tile { w, h, render_us }, Some(rgba))
                }
                Err(e) => (Reply::Err(e.to_string()), None),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// App side

/// How to start a worker.
#[derive(Debug, Clone)]
pub struct WorkerCommand {
    pub exe: PathBuf,
    pub args: Vec<String>,
}

impl WorkerCommand {
    /// The running executable, which must call [`serve_if_requested`] first thing in `main`.
    pub fn current_exe() -> Option<Self> {
        Some(Self { exe: std::env::current_exe().ok()?, args: vec![WORKER_ARG.into()] })
    }
}

/// A rendered tile from a worker: width, height, RGBA bytes and render time in µs.
pub(crate) type Rendered = (u32, u32, Vec<u8>, u64);

/// A running worker process, used by one driver thread at a time.
pub(crate) struct Worker {
    /// Shared so that a cancel or the watchdog can end it from another thread.
    pub child: Arc<Mutex<Child>>,
    input: BufWriter<ChildStdin>,
    output: BufReader<ChildStdout>,
    /// Documents this worker has open.
    pub open: std::collections::HashSet<DocId>,
}

impl Worker {
    pub fn spawn(cmd: &WorkerCommand, pdfium_dir: Option<&Path>) -> std::io::Result<Self> {
        let mut c = Command::new(&cmd.exe);
        c.args(&cmd.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        if let Some(d) = pdfium_dir {
            c.env(crate::library::PDFIUM_DIR_ENV, d);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            c.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = c.spawn()?;
        let input = BufWriter::new(child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?);
        let output = BufReader::new(child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?);
        Ok(Self { child: Arc::new(Mutex::new(child)), input, output, open: Default::default() })
    }

    fn request(&mut self, req: &Request) -> std::io::Result<Reply> {
        send_json(&mut self.input, req)?;
        self.input.flush()?;
        read_json(&mut self.output)
    }

    /// Opens or closes documents. An error means the worker is gone.
    pub fn open(
        &mut self,
        doc: DocId,
        path: &Path,
        password: Option<&str>,
        stamp: Option<Stamp>,
    ) -> std::io::Result<Result<(), String>> {
        let req =
            Request::Open { doc, path: path.into(), password: password.map(Into::into), stamp: stamp_wire(stamp) };
        Ok(match self.request(&req)? {
            Reply::Ok => {
                self.open.insert(doc);
                Ok(())
            }
            Reply::Err(e) => Err(e),
            Reply::Tile { .. } => Err("unexpected reply".into()),
        })
    }

    pub fn close(&mut self, doc: DocId) -> std::io::Result<()> {
        self.open.remove(&doc);
        self.request(&Request::Close { doc }).map(drop)
    }

    /// Renders a tile. The outer error means the worker is gone.
    pub fn render(&mut self, spec: RenderSpec) -> std::io::Result<Result<Rendered, EngineError>> {
        Ok(match self.request(&Request::Render { spec })? {
            Reply::Tile { w, h, render_us } => {
                let rgba = read_frame(&mut self.output)?;
                if rgba.len() != (w * h * 4) as usize {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "tile size mismatch"));
                }
                Ok((w, h, rgba, render_us))
            }
            Reply::Err(e) => Err(EngineError::Render(e)),
            Reply::Ok => Err(EngineError::Render("unexpected reply".into())),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let mut c = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Ends a worker from any thread.
pub(crate) fn kill(child: &Mutex<Child>) {
    let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
}
