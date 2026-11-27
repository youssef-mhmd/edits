//! Engine-level benchmarks: the parts of section 7 that do not need the window.
//!
//! The viewport model is a 1280 x 800 window at 125% scaling (1600 x 1000
//! device pixels), with the page at fit-width (1400 device pixels wide).
//! Tiles of a view are requested all at once, as the viewer does, and a page
//! whose 512 px tiles are slow gets 1024 px tiles (viewer.ts `tileSide`).

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

/// A 512 px tile slower than this means the page gets 1024 px tiles.
const SLOW_TILE_MS: f64 = 40.0;

/// Pages found slow to tile, as the viewer remembers them.
type SlowPages = std::collections::HashSet<u32>;

/// Renders the tiles of `page` that intersect a viewport-sized window at
/// (`vx`, `vy`) in page pixels, all requested at once. Returns per-tile
/// render times.
fn render_view(
    engine: &Engine,
    info: &DocInfo,
    slow: &mut SlowPages,
    page: u32,
    page_w: u32,
    vx: u32,
    vy: u32,
) -> Res<Vec<f64>> {
    let (pw, ph) = page_px(info, page, page_w);
    let side = if slow.contains(&page) { 2 * TILE } else { TILE };
    let (tx_, rx) = std::sync::mpsc::channel();
    let mut n = 0;
    let mut ty = vy / side * side;
    while ty < (vy + VIEW_H).min(ph) {
        let mut tx = vx / side * side;
        while tx < (vx + VIEW_W).min(pw) {
            let spec =
                RenderSpec { doc: info.doc, page, page_w: pw, page_h: ph, x: tx, y: ty, w: side, h: side, rotation: 0 };
            let t = tx_.clone();
            engine.render_with(spec, 10, move |r| {
                let _ = t.send(r);
            });
            n += 1;
            tx += side;
        }
        ty += side;
    }
    let mut times = Vec::new();
    for _ in 0..n {
        let tile = rx.recv()??;
        let ms = tile.render_us as f64 / 1000.0;
        if side == TILE && ms > SLOW_TILE_MS {
            slow.insert(page);
        }
        times.push(ms);
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
    let mut slow = SlowPages::new();
    preview(engine, &info, 0)?;
    let first_preview_ms = ms(t);
    let mut tile_times = render_view(engine, &info, &mut slow, 0, FIT_W, 0, 0)?;
    let first_sharp_ms = ms(t);

    // Zoom steps on page 0: 125%, 150%, 175%, 200% of fit width, centred.
    let mut zoom = Vec::new();
    for step in [1.25, 1.5, 1.75, 2.0] {
        let w = (FIT_W as f64 * step) as u32;
        let (pw, ph) = page_px(&info, 0, w);
        let vx = pw.saturating_sub(VIEW_W) / 2;
        let vy = ph.saturating_sub(VIEW_H) / 2;
        let t = Instant::now();
        tile_times.extend(render_view(engine, &info, &mut slow, 0, w, vx, vy)?);
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
        tile_times.extend(render_view(engine, &info, &mut slow, page, FIT_W, 0, 0)?);
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

pub fn run(files: &[PathBuf], json: Option<&str>, workers: Option<usize>) -> Res<bool> {
    if files.is_empty() {
        return Err("core needs at least one FILE".into());
    }
    let t = Instant::now();
    let engine = Engine::start()?;
    let engine_start_ms = ms(t);
    let workers = workers.unwrap_or_else(edits_core::default_worker_count);
    if let Some(cmd) = edits_core::worker::WorkerCommand::current_exe() {
        engine.enable_workers(cmd, workers);
    }
    println!("engine start (load PDFium, start thread): {engine_start_ms:.1} ms; up to {workers} render workers");

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

/// Prints one page's text, to check reading order and character mapping.
pub fn dump_text(file: &Path, page: u32) -> Res<bool> {
    let engine = Engine::start()?;
    let info = engine.open(file, None)?;
    let t = engine.page_text(info.doc, page, 0)?;
    println!("{}", t.text);
    Ok(true)
}

/// Search timing as the app does it (8-page chunks from page 1), without and
/// with the text index built. `edits-bench search FILE QUERY`
pub fn search(file: &Path, query: &str) -> Res<bool> {
    use edits_core::SearchOptions;
    let engine = Engine::start()?;
    let mut all_pass = true;
    for indexed in [false, true] {
        let info = engine.open(file, None)?;
        let pages: Vec<u32> = (0..info.page_count).collect();
        if indexed {
            let t = Instant::now();
            while engine.index_text(info.doc, 0, 64, 0)? > 0 {}
            println!("index {} pages: {:.0} ms", info.page_count, ms(t));
        }
        let t = Instant::now();
        let mut first = None;
        let mut matches = 0;
        for chunk in pages.chunks(8) {
            let r = engine.search_pages(info.doc, chunk.to_vec(), query.to_string(), SearchOptions::default(), 0)?;
            if first.is_none() && !r.is_empty() {
                first = Some(ms(t));
            }
            matches += r.iter().map(|(_, m)| m.len()).sum::<usize>();
        }
        let total = ms(t);
        let label = if indexed { "indexed" } else { "cold" };
        println!("{label}: {matches} matches, first {:.0} ms, all {total:.0} ms", first.unwrap_or(f64::NAN));
        let mut checks = vec![Check::new(format!("search {label}: all results"), total, targets::SEARCH_ALL_MS, "ms")];
        if let Some(f) = first {
            checks.insert(0, Check::new(format!("search {label}: first result"), f, targets::SEARCH_FIRST_MS, "ms"));
        }
        targets::print(&checks);
        all_pass &= checks.iter().all(|c| c.pass);
        engine.close(info.doc);
    }
    Ok(all_pass)
}

/// Compares page text with the expected files in `tests/text`
/// (`<pdf stem>.p<page>.txt`), finding each PDF under the corpus folder.
/// Reading order is fragile; this catches regressions. `edits-bench text-check`
pub fn text_check(expected_dir: &Path, corpus: &Path) -> Res<bool> {
    let engine = Engine::start()?;
    let pdfs = crate::check::pdf_files(corpus)?;
    let mut ok = true;
    let mut entries: Vec<_> = std::fs::read_dir(expected_dir)?.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let Some((stem, page)) = name.strip_suffix(".txt").and_then(|n| n.rsplit_once(".p")) else { continue };
        let Ok(page) = page.parse::<u32>() else { continue };
        let Some(pdf) = pdfs.iter().find(|p| p.file_stem().is_some_and(|s| s == stem)) else {
            println!("SKIP     {name}: {stem}.pdf not in {}", corpus.display());
            continue;
        };
        let info = engine.open(pdf, None)?;
        let got = engine.page_text(info.doc, page.saturating_sub(1), 0)?.text;
        engine.close(info.doc);
        let want = std::fs::read_to_string(&path)?.replace("\r\n", "\n");
        let want = want.trim_end_matches('\n');
        if got == want {
            println!("OK       {name}");
        } else {
            ok = false;
            println!("CHANGED  {name}");
            for (k, (g, w)) in got.lines().zip(want.lines()).enumerate() {
                if g != w {
                    println!("  line {}:\n    expected: {w}\n    got:      {g}", k + 1);
                }
            }
            if got.lines().count() != want.lines().count() {
                println!("  lines: expected {}, got {}", want.lines().count(), got.lines().count());
            }
        }
    }
    Ok(ok)
}

/// Save timing (PRD 7: ≤ 200 ms after adding annotations). Copies FILE to a
/// temporary folder, adds a highlight on the first page, and times the
/// save (copy beside, append the update, rename) and the reopen.
/// `edits-bench save FILE`
pub fn save(file: &Path) -> Res<bool> {
    use edits_edit::{Annot, Kind, Session};
    let dir = std::env::temp_dir().join(format!("edits-bench-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let copy = dir.join(file.file_name().ok_or("no file name")?);
    std::fs::copy(file, &copy)?;
    let engine = Engine::start()?;
    let info = engine.open(&copy, None)?;
    let mut session = Session::open(&copy)?;
    let mut times = Vec::new();
    let mut edit_ms = Vec::new();
    for round in 0..5 {
        let y = 0.1 + round as f32 * 0.05;
        let c = engine.to_user_space(info.doc, 0, vec![[0.1, y], [0.6, y], [0.1, y + 0.03], [0.6, y + 0.03]])?;
        let mut a = Annot::new(0, Kind::Highlight);
        a.quads = vec![[c[0][0], c[0][1], c[1][0], c[1][1], c[2][0], c[2][1], c[3][0], c[3][1]]];
        a.fit_rect();
        a.color = [1.0, 0.9, 0.2];
        let t = Instant::now();
        session.add(a)?;
        engine.set_overlay(info.doc, session.tail())?;
        edit_ms.push(ms(t));
        let t = Instant::now();
        session.save(&copy)?;
        engine.reload(info.doc, None)?;
        times.push(ms(t));
    }
    engine.close(info.doc);
    let size_mb = std::fs::metadata(&copy)?.len() as f64 / 1e6;
    let s = summarize(&times);
    println!(
        "{} ({size_mb:.1} MB): save p50 {:.0} / max {:.0} ms ({:?}); edit shown p50 {:.0} ms",
        file.display(),
        s.p50,
        s.max,
        times.iter().map(|t| t.round()).collect::<Vec<_>>(),
        summarize(&edit_ms).p50
    );
    let check = Check::new("save after adding a highlight (p50)", s.p50, targets::SAVE_MS, "ms");
    targets::print(std::slice::from_ref(&check));
    let _ = std::fs::remove_dir_all(&dir);
    Ok(check.pass)
}

/// Writes a copy of `src` with one of every annotation edits makes, for
/// checking in other readers (Acrobat, Chrome, Edge).
/// `edits-bench annot-demo SRC OUT`
pub fn annot_demo(src: &Path, out: &Path) -> Res<bool> {
    use edits_edit::{Annot, Kind, Session};
    std::fs::copy(src, out)?;
    let mut s = Session::open(out)?;
    let base = |kind: Kind, color: [f32; 3]| {
        let mut a = Annot::new(0, kind);
        a.color = color;
        a.author = "edits".into();
        a.modified = edits_edit::pdf_date_now();
        a
    };
    let mut hl = base(Kind::Highlight, [1.0, 0.92, 0.23]);
    hl.quads = vec![[60.0, 760.0, 300.0, 760.0, 60.0, 740.0, 300.0, 740.0]];
    hl.fit_rect();
    s.add(hl)?;
    for (k, y, c) in [
        (Kind::Underline, 715.0, [0.0, 0.4, 1.0]),
        (Kind::StrikeOut, 690.0, [0.9, 0.1, 0.1]),
        (Kind::Squiggly, 665.0, [0.1, 0.6, 0.2]),
    ] {
        let mut a = base(k, c);
        a.quads = vec![[60.0, y + 14.0, 300.0, y + 14.0, 60.0, y, 300.0, y]];
        a.fit_rect();
        s.add(a)?;
    }
    let mut ink = base(Kind::Ink, [0.9, 0.1, 0.1]);
    ink.width = 2.0;
    ink.ink = vec![(0..40).map(|i| [330.0 + i as f32 * 5.0, 700.0 + (i as f32 * 0.5).sin() * 20.0]).collect()];
    ink.fit_rect();
    s.add(ink)?;
    let mut line = base(Kind::Line, [0.0, 0.0, 0.0]);
    line.line = Some([330.0, 640.0, 520.0, 600.0]);
    line.arrow = true;
    line.width = 2.0;
    line.fit_rect();
    s.add(line)?;
    let mut sq = base(Kind::Square, [0.0, 0.4, 1.0]);
    sq.rect = [60.0, 520.0, 200.0, 600.0];
    sq.width = 2.0;
    s.add(sq)?;
    let mut ci = base(Kind::Circle, [0.9, 0.1, 0.1]);
    ci.rect = [220.0, 520.0, 320.0, 600.0];
    ci.fill = Some([1.0, 0.85, 0.85]);
    ci.width = 1.0;
    s.add(ci)?;
    let mut tb = base(Kind::FreeText, [0.0, 0.0, 0.0]);
    tb.rect = [340.0, 0.0, 540.0, 590.0];
    tb.font_size = 12.0;
    tb.width = 1.0;
    tb.fill = Some([1.0, 1.0, 0.85]);
    tb.contents = "مربع نص بالعربية مع كلمة English في المنتصف، والأرقام 123.\nA second paragraph in English.".into();
    tb.fit_text();
    s.add(tb)?;
    let mut note = base(Kind::Note, [1.0, 0.85, 0.2]);
    note.rect = [60.0, 470.0, 80.0, 490.0];
    note.contents = "ملاحظة لاصقة: sticky note text".into();
    s.add(note)?;
    for (i, label) in ["معتمد", "APPROVED", "مسودة DRAFT"].into_iter().enumerate() {
        let mut st = base(Kind::Stamp, [0.8, 0.05, 0.05]);
        st.label = label.into();
        st.font_size = 18.0;
        st.fit_label(120.0 + i as f32 * 140.0, 490.0);
        s.add(st)?;
    }
    s.save(out)?;
    println!("wrote {}", out.display());
    Ok(true)
}

/// Writes a sample of page changes to `out` for checking in other readers:
/// page 2 turned, page 1 copied and cropped, a blank page, page numbers and
/// headers in Arabic and English, and a watermark.
pub fn pages_demo(src: &Path, out: &Path) -> Res<bool> {
    use edits_edit::{Decor, Session};
    std::fs::copy(src, out)?;
    let mut s = Session::open(out)?;
    let n = s.page_count()?;
    let all = |s: &Session| (0..s.page_count().unwrap_or(0)).collect::<Vec<u32>>();
    if n > 1 {
        s.rotate_pages(&[1], 1)?;
    }
    s.duplicate_pages(&[0])?;
    s.crop_pages(&[(1, None)])?;
    s.insert_blank(s.page_count()?, 595.0, 842.0, 1)?;
    let base: Decor = serde_json::from_str("{}")?;
    s.decorate_pages(
        &all(&s),
        Decor {
            header: ["edits".into(), String::new(), "تقرير شهري".into()],
            footer: [String::new(), "صفحة {n} من {total}".into(), "Page {n} of {total}".into()],
            arabic_digits: false,
            color: [0.1, 0.2, 0.5],
            ..base.clone()
        },
    )?;
    s.decorate_pages(&all(&s), Decor { watermark: "مسودة DRAFT".into(), ..base })?;
    let len = s.save(out)?;
    println!("{}: {} pages, {len} bytes", out.display(), s.page_count()?);
    Ok(true)
}

/// Lists a document's form fields as edits reads them.
pub fn fields(path: &Path) -> Res<bool> {
    use edits_edit::pdf::{FileSource, PdfFile};
    let f = PdfFile::open(FileSource::open(path)?)?;
    let fields = edits_edit::forms::read_fields(&f)?;
    let mut kinds = std::collections::BTreeMap::new();
    for fl in &fields {
        *kinds.entry(format!("{:?}", fl.kind)).or_insert(0) += 1;
    }
    println!("{}: {} fields {kinds:?}", path.display(), fields.len());
    for fl in fields.iter().take(6) {
        println!(
            "  {:?} {} = {:?} ({} widgets, page {})",
            fl.kind,
            fl.name,
            fl.value,
            fl.widgets.len(),
            fl.widgets[0].page
        );
    }
    Ok(true)
}

/// Prints object `num` of a file as edits' reader sees it (decrypted).
pub fn object(path: &Path, num: u32) -> Res<bool> {
    use edits_edit::pdf::{FileSource, Obj, PdfFile};
    let pw = crate::check::password_for(path).unwrap_or_default();
    let f = PdfFile::open_with_password(FileSource::open(path)?, pw.as_bytes())?;
    match f.get((num, 0))? {
        Obj::Stream(d, data) => {
            let decoded = edits_edit::pdf::file::decode(&d, &data).map(|v| v.len());
            println!("stream {d:?}: {} bytes stored, decoded {decoded:?}", data.len());
        }
        o => println!("{o:?}"),
    }
    Ok(true)
}
