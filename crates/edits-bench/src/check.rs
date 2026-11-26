//! Corpus checks: every file opens and renders without a crash or hang, and
//! lopdf can load, decrypt, clean and re-save it.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use edits_core::{Engine, EngineError, RenderSpec};

use crate::Res;

const TIMEOUT: Duration = Duration::from_secs(120);
const THUMB_W: u32 = 200;
/// Pages rendered per file in the crash check; very long files are sampled.
const MAX_PAGES: u32 = 400;

/// Password convention for corpus files: `...-user-<password>.pdf`.
pub fn password_for(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    stem.rsplit_once("-user-").map(|(_, pw)| pw.to_string())
}

pub fn pdf_files(dir: &Path) -> Res<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))? {
            let p = entry?.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Child process: open the file, render a thumbnail of each page, print one line.
/// Expected failures (damaged file) are reported, not crashed on.
pub fn render_file(path: &Path) -> Res<bool> {
    let engine = Engine::start()?;
    let t = Instant::now();
    let info = match engine.open(path, password_for(path)) {
        Ok(i) => i,
        Err(e @ (EngineError::Open(_) | EngineError::PasswordRequired)) => {
            println!("REFUSED open: {}", e.to_string().replace(['\n', '\r'], " "));
            return Ok(true);
        }
        Err(e) => return Err(e.into()),
    };
    let step = info.page_count.div_ceil(MAX_PAGES).max(1);
    let mut errors = 0;
    let mut rendered = 0;
    for page in (0..info.page_count).step_by(step as usize) {
        let p = info.pages[page as usize];
        let h = ((THUMB_W as f32 * p.h / p.w).round() as u32).clamp(1, 4096);
        let spec =
            RenderSpec { doc: info.doc, page, page_w: THUMB_W, page_h: h, x: 0, y: 0, w: THUMB_W, h, rotation: 0 };
        match engine.render(spec, 0) {
            Ok(_) => rendered += 1,
            Err(_) => errors += 1,
        }
    }
    println!(
        "OK pages={} rendered={rendered} page_errors={errors} ms={:.0}",
        info.page_count,
        t.elapsed().as_secs_f64() * 1000.0
    );
    Ok(true)
}

fn run_child(args: &[&str]) -> (String, String) {
    let exe = std::env::current_exe().expect("current exe");
    let mut child = match Command::new(exe).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => return ("ERROR".into(), e.to_string()),
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut s) = child.stdout.take() {
                    let _ = s.read_to_string(&mut out);
                }
                let mut err = String::new();
                if let Some(mut s) = child.stderr.take() {
                    let _ = s.read_to_string(&mut err);
                }
                let line = out.lines().last().unwrap_or("").to_string();
                if status.success() {
                    let verdict = if line.starts_with("REFUSED") { "REFUSED" } else { "OK" };
                    return (verdict.into(), line);
                }
                return ("CRASH".into(), format!("{status} {}", err.trim()));
            }
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                return ("HANG".into(), format!("killed after {}s", TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return ("ERROR".into(), e.to_string()),
        }
    }
}

pub fn run_all(dir: &Path) -> Res<bool> {
    let files = pdf_files(dir)?;
    if files.is_empty() {
        return Err(format!("no PDF files under {}", dir.display()).into());
    }
    let mut bad = 0;
    for f in &files {
        let (verdict, detail) = run_child(&["render-file", f.to_str().unwrap_or_default()]);
        if verdict == "CRASH" || verdict == "HANG" || verdict == "ERROR" {
            bad += 1;
        }
        println!("{verdict:<8} {}  {detail}", f.display());
    }
    println!("\n{} files, {} crashed or hung", files.len(), bad);
    Ok(bad == 0)
}

/// lopdf evaluation for the PRD 8.1 decision: can it load, decrypt, prune,
/// compress and save each file, and does PDFium still open the result?
pub fn lopdf_all(dir: &Path) -> Res<bool> {
    let files = pdf_files(dir)?;
    let out_dir = std::env::temp_dir().join("edits-lopdf-check");
    std::fs::create_dir_all(&out_dir)?;
    let engine = Engine::start()?;
    let mut ok = 0;
    println!("{:<44} {:>8} {:>8} {:>8} {:>8}  result", "file", "load ms", "save ms", "MB in", "MB out");
    for f in &files {
        let name = f.file_name().unwrap_or_default().to_string_lossy().to_string();
        let size_in = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
        let t = Instant::now();
        let loaded = match password_for(f) {
            Some(pw) => lopdf::Document::load_with_password(f, &pw),
            None => lopdf::Document::load(f),
        };
        let load_ms = t.elapsed().as_secs_f64() * 1000.0;
        let mut doc = match loaded {
            Ok(d) => d,
            Err(e) => {
                println!("{name:<44} {load_ms:>8.0} {:>8} {size_in:>8.1} {:>8}  load failed: {e}", "-", "-");
                continue;
            }
        };
        let pages_lopdf = doc.get_pages().len() as u32;
        let was_encrypted = doc.is_encrypted();
        if was_encrypted && let Err(e) = doc.decrypt(&password_for(f).unwrap_or_default()) {
            println!("{name:<44} {load_ms:>8.0} {:>8} {size_in:>8.1} {:>8}  decrypt failed: {e}", "-", "-");
            continue;
        }
        let t = Instant::now();
        doc.prune_objects();
        doc.delete_zero_length_streams();
        doc.compress();
        let out = out_dir.join(&name);
        if let Err(e) = doc.save(&out) {
            println!("{name:<44} {load_ms:>8.0} {:>8} {size_in:>8.1} {:>8}  save failed: {e}", "-", "-");
            continue;
        }
        let save_ms = t.elapsed().as_secs_f64() * 1000.0;
        let size_out = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
        let verdict = match engine.open(&out, None) {
            Ok(info) if info.page_count == pages_lopdf => {
                ok += 1;
                engine.close(info.doc);
                format!("ok ({pages_lopdf} pages{})", if was_encrypted { ", decrypted" } else { "" })
            }
            Ok(info) => format!("page count changed: lopdf {pages_lopdf}, PDFium {}", info.page_count),
            Err(e) => format!("PDFium cannot open result: {e}"),
        };
        println!("{name:<44} {load_ms:>8.0} {save_ms:>8.0} {size_in:>8.1} {size_out:>8.1}  {verdict}");
        let _ = std::fs::remove_file(&out);
    }
    println!("\n{ok} of {} files round-tripped", files.len());
    Ok(true)
}
