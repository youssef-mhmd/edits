//! Locating the bundled PDFium library.

use std::path::{Path, PathBuf};

/// Environment variable that overrides where `pdfium.dll` is loaded from.
pub const PDFIUM_DIR_ENV: &str = "EDITS_PDFIUM_DIR";

fn lib_name() -> &'static str {
    if cfg!(windows) {
        "pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    }
}

/// Finds the directory that holds the PDFium library.
///
/// Search order: `EDITS_PDFIUM_DIR`, the executable's directory (installed
/// app), then `vendor/pdfium/bin` in the source tree (development builds).
pub fn find_pdfium_dir() -> Option<PathBuf> {
    let has_lib = |dir: &Path| dir.join(lib_name()).is_file();

    if let Some(dir) = std::env::var_os(PDFIUM_DIR_ENV).map(PathBuf::from)
        && has_lib(&dir)
    {
        return Some(dir);
    }

    if let Some(exe_dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        if has_lib(&exe_dir) {
            return Some(exe_dir);
        }
        // Tauri bundles resources in a `resources` folder next to the exe on some targets.
        let res = exe_dir.join("resources");
        if has_lib(&res) {
            return Some(res);
        }
    }

    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/pdfium/bin");
    if has_lib(&dev) {
        return Some(dev);
    }

    None
}
