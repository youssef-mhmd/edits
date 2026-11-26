//! Engine-level benchmarks: the parts of section 7 that do not need the window.
//!
//! The viewport model is a 1280 x 800 window at 125% scaling (1600 x 1000
//! device pixels), with the page at fit-width (1400 device pixels wide).

use std::path::{Path, PathBuf};
use std::time::Instant;

use edits_core::{DocInfo, Engine, RenderSpec};
use serde::Serialize;

use crate::Res;
use crate::stats::{Summary, summarize};
use crate::targets::{self, Check};

pub const VIEW_W: u32 = 1600;
pub const VIEW_H: u32 = 1000;
pub const FIT_W: u32 = 1400;
pub const TILE: u32 = 512;
pub const PREVIEW_W: u32 = 300;
const SCROLL_PAGES: u32 = 40;

#[derive(Serialize)]
struct FileResult {
    file: PathBuf,
    size_mb: f64,
    pages: u32,
    open_ms: f64,
    first_preview_ms: f64,
    first_sharp_ms: f64,
    zoom_sharp_ms: Summary,
    page_preview_ms: Summary,
    page_sharp_ms: Summary,
    tile_render_ms: Summary,
    checks: Vec<Check>,
}

#[derive(Serialize)]
struct Report {
    engine_start_ms: f64,
    files: Vec<FileResult>,
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn page_px(info: &DocInfo, page: u32, width_px: u32) -> (u32, u32) {
    let p = info.pages[page as usize];
    let h = (width_px as f64 * p.h as f64 / p.w as f64).round().max(1.0) as u32;
    (width_px, h)
}

/// Renders the tiles of `page` that intersect a viewport-sized window at
/// (`vx`, `vy`) in page pixels. Returns per-tile render times.
fn render_view(engine: &Engine, info: &DocInfo, page: u32, page_w: u32, vx: u32, vy: u32) -> Res<Vec<f64>> {
    let (pw, ph) = page_px(info, page, page_w);
    let mut times = Vec::new();
    let mut ty = vy / TILE * TILE;
    while ty < (vy + VIEW_H).min(ph) {
        let mut tx = vx / TILE * TILE;
        while tx < (vx + VIEW_W).min(pw) {
            let spec =
                RenderSpec { doc: info.doc, page, page_w: pw, page_h: ph, x: tx, y: ty, w: TILE, h: TILE, rotation: 0 };
            let tile = engine.render(spec, 10)?;
            times.push(tile.render_us as f64 / 1000.0);
            tx += TILE;
        }
        ty += TILE;
    }
    Ok(times)
}

fn preview(engine: &Engine, info: &DocInfo, page: u32) -> Res<()> {
    let (pw, ph) = page_px(info, page, PREVIEW_W);
    engine.render(
        RenderSpec { doc: info.doc, page, page_w: pw, page_h: ph, x: 0, y: 0, w: pw, h: ph.min(4096), rotation: 0 },
        20,
    )?;
    Ok(())
}

fn bench_file(engine: &Engine, file: &Path) -> Res<FileResult> {
    let size_mb = std::fs::metadata(file)?.len() as f64 / 1e6;

    let t = Instant::now();
    let info = engine.open(file, None)?;
    let open_ms = ms(t);

    // First page: preview, then the sharp viewport.
    preview(engine, &info, 0)?;
    let first_preview_ms = ms(t);
    let mut tile_times = render_view(engine, &info, 0, FIT_W, 0, 0)?;
    let first_sharp_ms = ms(t);

    // Zoom steps on page 0: 125%, 150%, 175%, 200% of fit width, centred.
    let mut zoom = Vec::new();
    for step in [1.25, 1.5, 1.75, 2.0] {
        let w = (FIT_W as f64 * step) as u32;
        let (pw, ph) = page_px(&info, 0, w);
        let vx = pw.saturating_sub(VIEW_W) / 2;
        let vy = ph.saturating_sub(VIEW_H) / 2;
        let t = Instant::now();
        tile_times.extend(render_view(engine, &info, 0, w, vx, vy)?);
        zoom.push(ms(t));
    }

    // Scroll: each page arrives as a preview, then its visible part goes sharp.
    let mut previews = Vec::new();
    let mut sharps = Vec::new();
    for page in 1..info.page_count.min(SCROLL_PAGES + 1) {
        let t = Instant::now();
        preview(engine, &info, page)?;
        previews.push(ms(t));
        let t = Instant::now();
        tile_times.extend(render_view(engine, &info, page, FIT_W, 0, 0)?);
        sharps.push(ms(t));
    }
    engine.close(info.doc);

    let large = info.page_count >= 1000 || size_mb >= 100.0;
    let open_target = if large { targets::OPEN_LARGE_FIRST_PAGE_MS } else { targets::OPEN_10MB_FIRST_PAGE_MS };
    let page_sharp_ms = summarize(&sharps);
    let zoom_sharp_ms = summarize(&zoom);
    let mut checks = vec![
        // The app adds process start and window time on top; this is the engine's share.
        Check::new("engine: open to first page sharp", first_sharp_ms, open_target, "ms"),
        Check::new("engine: zoom step to sharp (p95)", zoom_sharp_ms.p95, targets::ZOOM_SHARP_MS, "ms"),
    ];
    if !sharps.is_empty() {
        checks.push(Check::new(
            "engine: scrolled page to sharp (p95)",
            page_sharp_ms.p95,
            targets::SHARP_AFTER_SCROLL_MS,
            "ms",
        ));
    }

    Ok(FileResult {
        file: file.to_path_buf(),
        size_mb,
        pages: info.page_count,
        open_ms,
        first_preview_ms,
        first_sharp_ms,
        zoom_sharp_ms,
        page_preview_ms: summarize(&previews),
        page_sharp_ms,
        tile_render_ms: summarize(&tile_times),
        checks,
    })
}

pub fn run(files: &[PathBuf], json: Option<&str>) -> Res<bool> {
    if files.is_empty() {
        return Err("core needs at least one FILE".into());
    }
    let t = Instant::now();
    let engine = Engine::start()?;
    let engine_start_ms = ms(t);
    println!("engine start (load PDFium, start thread): {engine_start_ms:.1} ms");

    let mut results = Vec::new();
    let mut all_pass = true;
    for f in files {
        match bench_file(&engine, f) {
            Ok(r) => {
                println!(
                    "\n{} ({:.1} MB, {} pages)\n  open {:.1} ms | first preview {:.1} ms | first sharp {:.1} ms\n  \
                     zoom p50 {:.1} / p95 {:.1} ms | page preview p50 {:.1} / p95 {:.1} ms | page sharp p50 {:.1} / p95 {:.1} ms | tile p50 {:.1} / p95 {:.1} ms",
                    f.display(),
                    r.size_mb,
                    r.pages,
                    r.open_ms,
                    r.first_preview_ms,
                    r.first_sharp_ms,
                    r.zoom_sharp_ms.p50,
                    r.zoom_sharp_ms.p95,
                    r.page_preview_ms.p50,
                    r.page_preview_ms.p95,
                    r.page_sharp_ms.p50,
                    r.page_sharp_ms.p95,
                    r.tile_render_ms.p50,
                    r.tile_render_ms.p95,
                );
                targets::print(&r.checks);
                all_pass &= r.checks.iter().all(|c| c.pass);
                results.push(r);
            }
            Err(e) => {
                println!("\n{}: ERROR {e}", f.display());
                all_pass = false;
            }
        }
    }

    if let Some(out) = json {
        let report = Report { engine_start_ms, files: results };
        std::fs::write(out, serde_json::to_string_pretty(&report)?)?;
    }
    Ok(all_pass)
}

/// Private memory of this process in MB.
fn private_mb() -> f64 {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let Ok(mut sys) = System::new() else { return 0.0 };
    let pid = Pid::from_u32(std::process::id());
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing().with_memory());
    sys.process(pid).map(|p| p.virtual_memory() as f64 / 1e6).unwrap_or(0.0)
}

/// Engine memory while rendering a preview of every page, as the viewer's
/// background fill does. `edits-bench mem-previews FILE`
pub fn mem_previews(file: &Path) -> Res<bool> {
    let engine = Engine::start()?;
    let info = engine.open(file, None)?;
    println!("after open: {:.0} MB", private_mb());
    for page in 0..info.page_count {
        let (pw, ph) = page_px(&info, page, PREVIEW_W);
        let spec =
            RenderSpec { doc: info.doc, page, page_w: pw, page_h: ph, x: 0, y: 0, w: pw, h: ph.min(4096), rotation: 0 };
        drop(engine.render(spec, 0)?);
        if (page + 1) % 250 == 0 {
            println!("after {} previews: {:.0} MB", page + 1, private_mb());
        }
    }
    engine.close(info.doc);
    std::thread::sleep(std::time::Duration::from_millis(200));
    println!("after close: {:.0} MB", private_mb());
    Ok(true)
}
