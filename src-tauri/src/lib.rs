//! The edits desktop app: Tauri commands and the tile protocol.
//!
//! Page tiles reach the WebView through the `tile` URL scheme
//! (`http://tile.localhost/...` on Windows). Each request is one render job
//! on the engine's queue; the response body is raw RGBA.

mod bench;
mod nav;
#[cfg(windows)]
mod shm;
mod state;
mod tiles;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use edits_core::{DocId, DocInfo, Engine, EngineError};
use serde::Serialize;
use tauri::{Emitter, Manager};

/// Command-line options. `edits [--bench-out FILE] [PDF]`
#[derive(Debug, Default, Clone)]
pub struct Args {
    pub file: Option<PathBuf>,
    pub bench_out: Option<PathBuf>,
}

impl Args {
    fn parse() -> Self {
        let mut args = Args::default();
        let mut it = std::env::args_os().skip(1);
        while let Some(a) = it.next() {
            if a == "--bench-out" {
                args.bench_out = it.next().map(PathBuf::from);
            } else if args.file.is_none() {
                // Absolute, so the same file opened twice is recognised as one tab.
                let p = PathBuf::from(a);
                args.file = Some(std::path::absolute(&p).unwrap_or(p));
            }
        }
        args
    }
}

/// The open started at launch, so it overlaps WebView start-up.
type InitialOpen = Arc<Mutex<Option<std::sync::mpsc::Receiver<Result<DocInfo, EngineError>>>>>;

pub struct AppState {
    pub engine: Arc<Engine>,
    pub args: Args,
    initial: InitialOpen,
    /// Unix ms when `run` started and when Tauri finished setup (bench timing).
    main_unix_ms: f64,
    setup_unix_ms: Arc<Mutex<f64>>,
}

pub fn unix_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

#[derive(Serialize)]
struct Startup {
    file: Option<String>,
    bench: bool,
    /// Tile transport: "shm" (Windows default), "protocol" or "ipc";
    /// `EDITS_TRANSPORT` overrides it for measurements.
    transport: &'static str,
    main_unix_ms: f64,
    setup_unix_ms: f64,
}

#[tauri::command]
fn startup(state: tauri::State<'_, AppState>) -> Startup {
    Startup {
        file: state.args.file.as_ref().map(|p| p.display().to_string()),
        bench: state.args.bench_out.is_some(),
        transport: match std::env::var("EDITS_TRANSPORT").as_deref() {
            Ok("ipc") => "ipc",
            Ok("protocol") => "protocol",
            _ if cfg!(windows) => "shm",
            _ => "protocol",
        },
        main_unix_ms: state.main_unix_ms,
        setup_unix_ms: *state.setup_unix_ms.lock().unwrap_or_else(|e| e.into_inner()),
    }
}

/// Opens `path`, or with no path returns the file given on the command line.
#[tauri::command]
async fn open_doc(
    state: tauri::State<'_, AppState>,
    path: Option<String>,
    password: Option<String>,
) -> Result<DocInfo, EngineError> {
    let engine = state.engine.clone();
    let initial = state.initial.clone();
    let initial_path = state.args.file.clone();
    tauri::async_runtime::spawn_blocking(move || match path {
        Some(p) => engine.open(p, password),
        // A password for the launch file: open it again with the password.
        None if password.is_some() => {
            engine.open(initial_path.ok_or(EngineError::Open("no file given".into()))?, password)
        }
        None => {
            let rx = initial.lock().unwrap_or_else(|e| e.into_inner()).take();
            rx.ok_or(EngineError::Open("no file given".into()))?.recv().map_err(|_| EngineError::Stopped)?
        }
    })
    .await
    .map_err(|_| EngineError::Stopped)?
}

#[tauri::command]
fn close_doc(state: tauri::State<'_, AppState>, doc: DocId) {
    state.engine.close(doc);
}

/// Cancels queued renders for `doc` except those for the listed
/// `[page, page_w]` pairs (the pages in view at their current sizes) and
/// any render whose width is in `keep_widths` (background previews).
#[tauri::command]
fn set_viewport(state: tauri::State<'_, AppState>, doc: DocId, keep: Vec<[u32; 2]>, keep_widths: Vec<u32>) -> usize {
    state
        .engine
        .cancel_where(|s| s.doc == doc && !keep.contains(&[s.page, s.page_w]) && !keep_widths.contains(&s.page_w))
}

/// PDF paths from another launch's command line, made absolute.
fn files_from_argv(argv: &[String], cwd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for a in argv.iter().skip(1) {
        if skip_next {
            skip_next = false;
        } else if a == "--bench-out" {
            skip_next = true;
        } else if !a.starts_with("--") {
            let p = PathBuf::from(a);
            let p = if p.is_absolute() { p } else { PathBuf::from(cwd).join(p) };
            out.push(p.display().to_string());
        }
    }
    out
}

pub fn run() {
    let main_unix_ms = unix_ms();
    let args = Args::parse();
    let engine = Arc::new(Engine::start().unwrap_or_else(|e| {
        // Without PDFium nothing works; say why instead of opening a blank window.
        eprintln!("edits: {e}");
        std::process::exit(1);
    }));

    let initial: InitialOpen = Arc::new(Mutex::new(None));
    if let Some(file) = &args.file {
        let (tx, rx) = std::sync::mpsc::channel();
        engine.open_with(file.clone(), None, move |r| {
            let _ = tx.send(r);
        });
        *initial.lock().unwrap_or_else(|e| e.into_inner()) = Some(rx);
    }

    let tile_engine = engine.clone();
    let setup_ms = Arc::new(Mutex::new(0.0));
    let setup_ms_hook = setup_ms.clone();
    let mut builder = tauri::Builder::default();
    // One window: a second launch hands its files to the running app as
    // tabs. Bench runs and UI tests (EDITS_NO_SINGLE_INSTANCE) stay separate.
    if args.bench_out.is_none() && std::env::var_os("EDITS_NO_SINGLE_INSTANCE").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let files = files_from_argv(&argv, &cwd);
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            if !files.is_empty() {
                let _ = app.emit("open-files", files);
            }
        }));
    }
    builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("tile", move |_ctx, request, responder| {
            tiles::handle(&tile_engine, request, responder);
        })
        .manage(AppState { engine, args, initial, main_unix_ms, setup_unix_ms: setup_ms.clone() })
        .invoke_handler(tauri::generate_handler![
            startup,
            open_doc,
            close_doc,
            set_viewport,
            tiles::render_tile_ipc,
            #[cfg(windows)]
            shm::render_tile_shm,
            bench::bench_report,
            nav::outline,
            nav::page_links,
            state::load_state,
            state::save_state,
        ])
        .setup(move |_app| {
            *setup_ms_hook.lock().unwrap_or_else(|e| e.into_inner()) = unix_ms();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running edits");
}
