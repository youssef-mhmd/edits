//! Small persistent state (recent files, last positions), stored as one JSON
//! file in the app's data folder. The front end owns its shape.

use std::path::PathBuf;

use tauri::Manager;

/// Larger than any sane state; protects against a runaway writer.
const MAX_STATE_BYTES: usize = 1_000_000;

fn state_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    // Tests point this elsewhere so they never touch the user's recent files.
    let dir = match std::env::var_os("EDITS_STATE_DIR") {
        Some(d) => PathBuf::from(d),
        None => app.path().app_data_dir().map_err(|e| e.to_string())?,
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("state.json"))
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
