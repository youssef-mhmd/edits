//! App-level benchmarks: launches the real app in bench mode.
//!
//! `edits.exe --bench-out OUT.json [FILE]` runs a fixed script in the window
//! (open, scroll, zoom), writes its measurements to OUT.json and exits. Times
//! in the report are Unix milliseconds, so this process can measure from the
//! moment it spawned the app.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::Res;
use crate::stats::{median, summarize};
use crate::targets::{self, Check};

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct AppReport {
    window_ready_unix_ms: f64,
    first_page_unix_ms: Option<f64>,
    /// Intervals between animation frames during the scripted scroll.
    frame_ms: Vec<f64>,
    /// Frames in which a page in view showed neither preview nor tiles.
    blank_frames: u32,
    /// For each page that came into view: time until it was fully sharp.
    sharp_after_scroll_ms: Vec<f64>,
    zoom_sharp_ms: Vec<f64>,
    /// Working set of the app process and its WebView children, end of run.
    memory_mb: Option<f64>,
    error: Option<String>,
    main_unix_ms: f64,
    setup_unix_ms: f64,
    navigation_start_unix_ms: f64,
    dom_content_loaded_unix_ms: f64,
    tiles: TileTimes,
    memory_ws_mb: Option<f64>,
    transfer: Option<Transfer>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct TimedSeq {
    total: Vec<f64>,
    render: Vec<f64>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct Transfer {
    small: TimedSeq,
    big: TimedSeq,
    parallel12_ms: f64,
}

#[derive(Deserialize, Debug, Default)]
#[serde(default)]
struct TileTimes {
    render: Vec<f64>,
    queued: Vec<f64>,
    total: Vec<f64>,
}

impl AppReport {
    /// Start-up phases in ms: spawn->main, main->setup, setup->navigation, navigation->DOM ready, DOM->first frame.
    fn phases(&self, spawned: f64) -> [f64; 5] {
        [
            self.main_unix_ms - spawned,
            self.setup_unix_ms - self.main_unix_ms,
            self.navigation_start_unix_ms - self.setup_unix_ms,
            self.dom_content_loaded_unix_ms - self.navigation_start_unix_ms,
            self.window_ready_unix_ms - self.dom_content_loaded_unix_ms,
        ]
    }
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

fn launch(exe: &Path, file: Option<&Path>) -> Res<(f64, AppReport)> {
    let out = std::env::temp_dir().join(format!("edits-bench-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let mut cmd = Command::new(exe);
    cmd.arg("--bench-out").arg(&out);
    if let Some(f) = file {
        cmd.arg(f);
    }
    let spawned = now_ms();
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(90) {
            let _ = child.kill();
            return Err("app did not finish the bench script in 90 s".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let text = std::fs::read_to_string(&out).map_err(|e| format!("no bench report from app: {e}"))?;
    let report: AppReport = serde_json::from_str(&text)?;
    if let Some(e) = &report.error {
        return Err(format!("app reported: {e}").into());
    }
    Ok((spawned, report))
}

/// Cold start only, no file: `edits-bench startup --exe EXE [--runs N]`.
pub fn startup(exe: &Path, runs: usize) -> Res<bool> {
    let mut cold = Vec::new();
    let mut phases = Vec::new();
    let mut mem = Vec::new();
    for _ in 0..runs {
        let (spawned, r) = launch(exe, None)?;
        cold.push(r.window_ready_unix_ms - spawned);
        phases.push(r.phases(spawned));
        mem.extend(r.memory_mb);
    }
    let ph: Vec<f64> = (0..5).map(|k| median(&phases.iter().map(|p| p[k]).collect::<Vec<_>>())).collect();
    println!(
        "cold start median {:.0} ms (runs: {:?}) | spawn->main {:.0} | main->navigation {:.0} | navigation->DOM {:.0} | DOM->frame {:.0} | private memory {:.0} MB",
        median(&cold),
        cold.iter().map(|c| c.round()).collect::<Vec<_>>(),
        ph[0],
        ph[1] + ph[2],
        ph[3],
        ph[4],
        median(&mem)
    );
    let check = Check::new("cold start to usable window (median)", median(&cold), targets::COLD_START_MS, "ms");
    targets::print(std::slice::from_ref(&check));
    Ok(check.pass)
}

pub fn run(exe: &Path, file: &Path, runs: usize, json: Option<&str>) -> Res<bool> {
    let mut cold = Vec::new();
    let mut first_page = Vec::new();
    let mut frames = Vec::new();
    let mut blank = 0;
    let mut sharp = Vec::new();
    let mut zoom = Vec::new();
    let mut memory = Vec::new();
    let mut idle_memory = Vec::new();
    let mut phases: Vec<[f64; 5]> = Vec::new();
    let mut tiles = TileTimes::default();
    let mut ws = Vec::new();
    let mut xfer: Vec<Transfer> = Vec::new();

    for i in 0..runs {
        // Start without a file: cold start to usable window.
        let (spawned, r) = launch(exe, None)?;
        cold.push(r.window_ready_unix_ms - spawned);
        phases.push(r.phases(spawned));
        idle_memory.extend(r.memory_mb);

        // Start with the file: double-click to first page, then scroll and zoom.
        let (spawned, r) = launch(exe, Some(file))?;
        let fp = r.first_page_unix_ms.ok_or("app did not report a first page")? - spawned;
        first_page.push(fp);
        frames.extend(r.frame_ms);
        blank += r.blank_frames;
        sharp.extend(r.sharp_after_scroll_ms);
        zoom.extend(r.zoom_sharp_ms);
        memory.extend(r.memory_mb);
        tiles.render.extend(r.tiles.render);
        tiles.queued.extend(r.tiles.queued);
        tiles.total.extend(r.tiles.total);
        ws.extend(r.memory_ws_mb);
        xfer.extend(r.transfer);
        println!("run {}: cold start {:.0} ms, first page {:.0} ms", i + 1, cold[i], fp);
    }

    let size_mb = std::fs::metadata(file)?.len() as f64 / 1e6;
    let open_target =
        if size_mb >= 100.0 { targets::OPEN_LARGE_FIRST_PAGE_MS } else { targets::OPEN_10MB_FIRST_PAGE_MS };
    let f = summarize(&frames);
    let s = summarize(&sharp);
    let z = summarize(&zoom);
    let mut checks = vec![
        Check::new("cold start to usable window (median)", median(&cold), targets::COLD_START_MS, "ms"),
        Check::new(
            format!("start with {size_mb:.0} MB file to first page (median)"),
            median(&first_page),
            open_target,
            "ms",
        ),
        Check::new("scroll frame interval (p95)", f.p95, targets::FRAME_P95_MS, "ms"),
        Check::new("frames with a blank page in view", blank as f64, 0.0, ""),
        Check::new("page sharp after scrolling into view (p95)", s.p95, targets::SHARP_AFTER_SCROLL_MS, "ms"),
        Check::new("zoom step to sharp (p95)", z.p95, targets::ZOOM_SHARP_MS, "ms"),
    ];
    if !memory.is_empty() && !idle_memory.is_empty() {
        checks.push(Check::new(
            "memory at end of run above no-file start (median)",
            median(&memory) - median(&idle_memory),
            targets::MEMORY_ABOVE_IDLE_MB,
            "MB",
        ));
    }
    println!("\n{} ({size_mb:.1} MB), {runs} runs, {} frames", file.display(), f.n);
    let ph: Vec<f64> = (0..5).map(|k| median(&phases.iter().map(|p| p[k]).collect::<Vec<_>>())).collect();
    println!(
        "  start-up phases (median ms): spawn->main {:.0} | main->setup {:.0} | setup->navigation {:.0} | \
         navigation->DOM ready {:.0} | DOM->first frame {:.0}",
        ph[0], ph[1], ph[2], ph[3], ph[4]
    );
    let (tr, tq, tt) = (summarize(&tiles.render), summarize(&tiles.queued), summarize(&tiles.total));
    println!(
        "  tiles: {} | render p50 {:.1} / p95 {:.1} ms | queued p50 {:.1} / p95 {:.1} ms | request total p50 {:.1} / p95 {:.1} ms",
        tr.n, tr.p50, tr.p95, tq.p50, tq.p95, tt.p50, tt.p95
    );
    if !idle_memory.is_empty() {
        println!("  memory with no file (median): {:.0} MB", median(&idle_memory));
    }
    if !xfer.is_empty() {
        let cat = |f: &dyn Fn(&Transfer) -> &Vec<f64>| xfer.iter().flat_map(|t| f(t).clone()).collect::<Vec<_>>();
        let small = median(&cat(&|t| &t.small.total));
        let big = median(&cat(&|t| &t.big.total));
        let big_render = median(&cat(&|t| &t.big.render));
        let par = median(&xfer.iter().map(|t| t.parallel12_ms).collect::<Vec<_>>());
        println!(
            "  transfer (median): 16x16 tile {small:.1} ms | 512x512 (1 MB) tile {big:.1} ms of which render {big_render:.1} ms | 12 x 1 MB in parallel {par:.0} ms"
        );
    }
    if !ws.is_empty() {
        println!("  memory at end, working set incl. shared pages (median): {:.0} MB", median(&ws));
    }
    println!("  zoom steps (ms): {:?}", zoom.iter().map(|z| z.round()).collect::<Vec<_>>());
    targets::print(&checks);

    if let Some(out) = json {
        let v = serde_json::json!({
            "file": file, "runs": runs, "cold_start_ms": cold, "first_page_ms": first_page,
            "frame_ms": f, "blank_frames": blank, "sharp_after_scroll_ms": s, "zoom_sharp_ms": z,
            "memory_mb": memory, "idle_memory_mb": idle_memory, "startup_phases_ms": ph,
            "tile_render_ms": tr, "tile_queued_ms": tq, "tile_total_ms": tt, "checks": checks,
        });
        std::fs::write(out, serde_json::to_string_pretty(&v)?)?;
    }
    Ok(checks.iter().all(|c| c.pass))
}
