//! Small persistent state (recent files, last positions), stored as one JSON
//! file in the app's data folder. The front end owns its shape.

use std::path::PathBuf;

use tauri::Manager;

/// Larger than any sane state; protects against a runaway writer.
const MAX_STATE_BYTES: usize = 1_000_000;

/// The app's data folder. Tests point this elsewhere (EDITS_STATE_DIR) so
/// they never touch the user's recent files or recovery files.
pub fn data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    match std::env::var_os("EDITS_STATE_DIR") {
        Some(d) => Ok(PathBuf::from(d)),
        None => app.path().app_data_dir().map_err(|e| e.to_string()),
    }
}

fn state_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = data_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("state.json"))
}

/// The theme saved in the settings ("system", "light" or "dark"), read at
/// start-up so the window opens in the right colour before the front end runs.
pub fn saved_theme(app: &tauri::AppHandle) -> Option<String> {
    let text = std::fs::read(state_path(app).ok()?).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&text).ok()?;
    v.get("settings")?.get("theme")?.as_str().map(str::to_string)
}

/// The list of third-party software bundled with the app (PRD 10.1).
#[tauri::command]
pub fn third_party_licenses(app: tauri::AppHandle) -> String {
    let bundled = app.path().resource_dir().ok().map(|d| d.join("licenses").join("third-party.txt"));
    // Development builds read it from the source tree.
    let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../licenses/third-party.txt");
    bundled
        .into_iter()
        .chain([dev])
        .find_map(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| "The licence list is missing from this installation.".into())
}

/// The saved state, or `null` if there is none or it cannot be read.
#[tauri::command]
pub fn load_state(app: tauri::AppHandle) -> serde_json::Value {
    state_path(&app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Saves the state through a temporary file, so a crash cannot leave it half written.
#[tauri::command]
pub fn save_state(app: tauri::AppHandle, state: serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
    if text.len() > MAX_STATE_BYTES {
        return Err("state too large".into());
    }
    let path = state_path(&app)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}
