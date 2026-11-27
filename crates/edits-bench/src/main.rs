//! edits-bench: benchmarks (PRD section 7), corpus generation and corpus checks.
//!
//! ```text
//! edits-bench gen-corpus [--large] [DIR]          write synthetic corpus files
//! edits-bench make-scan IN.pdf OUT.pdf          image-only noisy "scan" of IN
//! edits-bench fuzz-corpus SRC OUT [--per-file N] damaged copies of real files, for `check`
//! edits-bench core [--workers N] [--json OUT] FILE...  engine benchmarks: open, first page, zoom, scroll
//! edits-bench app --exe EXE [--json OUT] FILE      app benchmarks: cold start, open, scroll, zoom
//! edits-bench startup --exe EXE [--runs N]       cold start only
//! edits-bench text FILE [PAGE]                   print a page's text as PDFium gives it (logical order check)
//! edits-bench search FILE QUERY                 search timing, cold and indexed (PRD 7)
//! edits-bench annot-demo SRC OUT              a copy of SRC with every kind of annotation edits makes
//! edits-bench save FILE                        incremental save timing, on a copy (PRD 7)
//! edits-bench text-check [DIR]                  page text against tests/text (reading order regressions)
//! edits-bench check [DIR]                          open and render every corpus file, one process each
//! edits-bench pdf-check [DIR]                    edits' own PDF reader and update writer on every file
//! edits-bench lopdf-check [DIR]                    load, decrypt, clean and re-save every file with lopdf
//! ```

mod app;
mod check;
mod core_bench;
mod corpus;
mod stats;
mod targets;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const DEFAULT_CORPUS: &str = "corpus/files";

fn main() -> ExitCode {
    // `core` starts this executable again as a render worker.
    if edits_core::worker::serve_if_requested() {
        return ExitCode::SUCCESS;
    }
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{}", USAGE);
        return ExitCode::from(2);
    }
    let cmd = args.remove(0);
    let result = match cmd.as_str() {
        "gen-corpus" => {
            let large = take_flag(&mut args, "--large");
            corpus::generate(&dir_arg(&args), large).map(|()| true)
        }
        "fuzz-corpus" => {
            let per = take_opt(&mut args, "--per-file").and_then(|n| n.parse().ok()).unwrap_or(4);
            match (args.first(), args.get(1)) {
                (Some(i), Some(o)) => corpus::fuzz(Path::new(i), Path::new(o), per).map(|()| true),
                _ => Err("fuzz-corpus needs SRC_DIR OUT_DIR".into()),
            }
        }
        "make-scan" => match (args.first(), args.get(1)) {
            (Some(i), Some(o)) => corpus::make_scan(&PathBuf::from(i), &PathBuf::from(o)).map(|()| true),
            _ => Err("make-scan needs INPUT.pdf OUTPUT.pdf".into()),
        },
        "core" => {
            let json = take_opt(&mut args, "--json");
            let workers = take_opt(&mut args, "--workers").and_then(|w| w.parse().ok());
            core_bench::run(&args.iter().map(PathBuf::from).collect::<Vec<_>>(), json.as_deref(), workers)
        }
        "app" => {
            let exe = take_opt(&mut args, "--exe");
            let json = take_opt(&mut args, "--json");
            let runs = take_opt(&mut args, "--runs").and_then(|r| r.parse().ok()).unwrap_or(5);
            match (exe, args.first()) {
                (Some(exe), Some(file)) => app::run(&PathBuf::from(exe), &PathBuf::from(file), runs, json.as_deref()),
                _ => Err("app needs --exe EXE and a FILE".into()),
            }
        }
        "startup" => {
            let exe = take_opt(&mut args, "--exe").ok_or("startup needs --exe EXE");
            let runs = take_opt(&mut args, "--runs").and_then(|r| r.parse().ok()).unwrap_or(10);
            exe.map_err(Into::into).and_then(|exe| app::startup(&PathBuf::from(exe), runs))
        }
        "mem-previews" => match args.first() {
            Some(f) => core_bench::mem_previews(&PathBuf::from(f)),
            None => Err("mem-previews needs a FILE".into()),
        },
        "text" => match args.first() {
            Some(f) => {
                let page = args.get(1).and_then(|p| p.parse().ok()).unwrap_or(1u32);
                core_bench::dump_text(&PathBuf::from(f), page.saturating_sub(1))
            }
            None => Err("text needs FILE [PAGE]".into()),
        },
        "search" => match (args.first(), args.get(1)) {
            (Some(f), Some(q)) => core_bench::search(&PathBuf::from(f), q),
            _ => Err("search needs FILE QUERY".into()),
        },
        "annot-demo" => match (args.first(), args.get(1)) {
            (Some(a), Some(b)) => core_bench::annot_demo(Path::new(a), Path::new(b)),
            _ => Err("annot-demo needs SRC OUT".into()),
        },
        "object" => match (args.first(), args.get(1).and_then(|n| n.parse().ok())) {
            (Some(f), Some(n)) => core_bench::object(Path::new(f), n),
            _ => Err("object needs FILE NUMBER".into()),
        },
        "fields" => match args.first() {
            Some(f) => core_bench::fields(Path::new(f)),
            None => Err("fields needs a FILE".into()),
        },
        "pages-demo" => match (args.first(), args.get(1)) {
            (Some(a), Some(b)) => core_bench::pages_demo(Path::new(a), Path::new(b)),
            _ => Err("pages-demo needs SRC OUT".into()),
        },
        "save" => match args.first() {
            Some(f) => core_bench::save(&PathBuf::from(f)),
            None => Err("save needs a FILE".into()),
        },
        "text-check" => core_bench::text_check(Path::new("tests/text"), &dir_arg(&args)),
        "check" => check::run_all(&dir_arg(&args)),
        "pdf-check" => check::pdf_all(&dir_arg(&args)),
        "pages-check" => check::pages_all(&dir_arg(&args)),
        "forms-check" => check::forms_all(&dir_arg(&args), args.get(1).map(Path::new)),
        "lopdf-check" => check::lopdf_all(&dir_arg(&args)),
        // Internal: run by `check` in a child process so a crash is contained.
        "render-file" => match args.first() {
            Some(f) => check::render_file(&PathBuf::from(f)),
            None => Err("render-file needs a FILE".into()),
        },
        _ => {
            eprintln!("{}", USAGE);
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "usage: edits-bench <gen-corpus|core|app|check|lopdf-check> [options]";

fn dir_arg(args: &[String]) -> PathBuf {
    PathBuf::from(args.first().map(String::as_str).unwrap_or(DEFAULT_CORPUS))
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != name);
    args.len() != before
}

fn take_opt(args: &mut Vec<String>, name: &str) -> Option<String> {
    let pos = args.iter().position(|a| a == name)?;
    args.remove(pos);
    (pos < args.len()).then(|| args.remove(pos))
}

pub type Res<T> = Result<T, Box<dyn std::error::Error>>;
