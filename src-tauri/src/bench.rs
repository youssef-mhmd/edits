//! Bench mode (`--bench-out FILE`): the front end runs a fixed script and
//! sends its measurements here; we add memory use, write the file and exit.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// Memory of this process and all its descendants (the WebView2 browser,
/// renderer and GPU processes), in MB: (working set, private commit).
/// Working set double-counts pages shared between processes; private
/// commit does not, and is closer to what Task Manager shows.
fn memory_mb() -> (f64, f64, Vec<(String, f64)>) {
    let Ok(mut sys) = System::new() else { return (0.0, 0.0, Vec::new()) };
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_memory().with_cmd(UpdateKind::Always),
    );
    let root = Pid::from_u32(std::process::id());
    let mut family = vec![root];
    let mut i = 0;
    while i < family.len() {
        let parent = family[i];
        family.extend(sys.processes().iter().filter(|(_, p)| p.parent() == Some(parent)).map(|(pid, _)| *pid));
        i += 1;
    }
    let procs: Vec<_> = family.iter().filter_map(|pid| sys.process(*pid)).collect();
    let ws = procs.iter().map(|p| p.memory() as f64).sum::<f64>() / 1e6;
    let private = procs.iter().map(|p| p.virtual_memory() as f64).sum::<f64>() / 1e6;
    let each = procs
        .iter()
        .map(|p| {
            let kind = p
                .cmd()
                .iter()
                .find_map(|a| a.to_str().and_then(|a| a.strip_prefix("--type=")).map(str::to_string))
                .unwrap_or_else(|| p.name().to_string_lossy().into_owned());
            (kind, p.virtual_memory() as f64 / 1e6)
        })
        .collect();
    (ws, private, each)
}

#[tauri::command]
pub fn bench_report(app: tauri::AppHandle, state: tauri::State<'_, crate::AppState>, mut report: serde_json::Value) {
    if let Some(obj) = report.as_object_mut() {
        let (ws, private, each) = memory_mb();
        obj.insert("memory_by_process".into(), serde_json::json!(each));
        obj.insert("memory_ws_mb".into(), ws.into());
        obj.insert("memory_mb".into(), private.into());
    }
    if let Some(out) = &state.args.bench_out {
        let text = serde_json::to_string_pretty(&report).unwrap_or_default();
        if let Err(e) = std::fs::write(out, text) {
            eprintln!("edits: cannot write bench report: {e}");
        }
    }
    app.exit(0);
}
