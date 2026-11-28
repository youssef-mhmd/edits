//! Organising pages (PRD 5.3): moving, turning, deleting, copying,
//! inserting and cropping pages of an open document, and writing pages
//! out as new files (extract, split, combine).
//!
//! Page changes go through the document's editing session like any other
//! change (undo, recovery, save). The front end gets the new page list back
//! (`Changed::info`) and shows the document anew.

use std::path::{Path, PathBuf};

use edits_core::DocId;
use edits_edit::Session;
use serde::Deserialize;

use crate::edit::{Changed, changed, with_session};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn pages_move(app: tauri::AppHandle, doc: DocId, pages: Vec<u32>, before: u32) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        s.move_pages(&pages, before).map_err(err)?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

#[tauri::command]
pub async fn pages_rotate(
    app: tauri::AppHandle,
    doc: DocId,
    pages: Vec<u32>,
    quarters: i32,
) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        s.rotate_pages(&pages, quarters).map_err(err)?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

#[tauri::command]
pub async fn pages_delete(app: tauri::AppHandle, doc: DocId, pages: Vec<u32>) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        s.delete_pages(&pages).map_err(err)?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

#[tauri::command]
pub async fn pages_duplicate(app: tauri::AppHandle, doc: DocId, pages: Vec<u32>) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        s.duplicate_pages(&pages).map_err(err)?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

/// Inserts `count` empty pages of `w` x `h` points before index `at`.
#[tauri::command]
pub async fn pages_insert_blank(
    app: tauri::AppHandle,
    doc: DocId,
    at: u32,
    w: f32,
    h: f32,
    count: u32,
) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        s.insert_blank(at, w, h, count).map_err(err)?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

/// Inserts a page for each picture file, before index `at`.
#[tauri::command]
pub async fn pages_insert_images(
    app: tauri::AppHandle,
    doc: DocId,
    at: u32,
    paths: Vec<String>,
) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        let mut images = Vec::new();
        for p in &paths {
            let bytes = std::fs::read(p).map_err(|e| format!("cannot read {p}: {e}"))?;
            images.push(edits_edit::image::PdfImage::from_bytes(&bytes).map_err(|e| format!("{p}: {e}"))?);
        }
        s.insert_images(at, images).map_err(err)?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

/// Inserts pages of another PDF (all of them when `pages` is null) before index `at`.
#[tauri::command]
pub async fn pages_insert_pdf(
    app: tauri::AppHandle,
    doc: DocId,
    at: u32,
    path: String,
    pages: Option<Vec<u32>>,
) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        s.insert_pdf(at, Path::new(&path), pages).map_err(|e| describe(&path, e))?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

fn describe(path: &str, e: edits_edit::EditError) -> String {
    let name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match e {
        edits_edit::EditError::Pdf(edits_edit::pdf::PdfError::Encrypted) => {
            format!("{name} is password-protected; pages cannot be taken from it yet")
        }
        edits_edit::EditError::Pdf(edits_edit::pdf::PdfError::Damaged(_)) => {
            format!("{name} is damaged; edits cannot take pages from it")
        }
        e => format!("{name}: {e}"),
    }
}

/// Crop margins of a page, as fractions of its shown size (left, top,
/// right, bottom), or null to show the whole page again.
#[derive(Deserialize)]
pub struct Crop {
    pub page: u32,
    pub margins: Option<[f32; 4]>,
}

#[tauri::command]
pub async fn pages_crop(app: tauri::AppHandle, doc: DocId, crops: Vec<Crop>) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        let mut boxes = Vec::new();
        for c in crops {
            let b = match c.margins {
                None => None,
                Some([l, t, r, b]) => {
                    if l + r > 0.95 || t + b > 0.95 || [l, t, r, b].iter().any(|v| *v < 0.0) {
                        return Err("the margins leave nothing of the page".into());
                    }
                    let u = engine.to_user_space(doc, c.page, vec![[l, t], [1.0 - r, 1.0 - b]]).map_err(err)?;
                    Some([u[0][0], u[0][1], u[1][0], u[1][1]])
                }
            };
            boxes.push((c.page, b));
        }
        s.crop_pages(&boxes).map_err(err)?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

/// Adds page numbers, headers, footers or a watermark to `pages`.
#[tauri::command]
pub async fn pages_decorate(
    app: tauri::AppHandle,
    doc: DocId,
    pages: Vec<u32>,
    decor: edits_edit::Decor,
) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        s.decorate_pages(&pages, decor).map_err(err)?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

/// Takes away what `pages_decorate` added to `pages` since the last save.
#[tauri::command]
pub async fn pages_undecorate(app: tauri::AppHandle, doc: DocId, pages: Vec<u32>) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        s.undecorate_pages(&pages).map_err(err)?;
        changed(engine, doc, s, Vec::new(), None)
    })
    .await
}

/// Writes `pages` (in that order) as a new PDF at `path`.
#[tauri::command]
pub async fn pages_export(app: tauri::AppHandle, doc: DocId, pages: Vec<u32>, path: String) -> Result<String, String> {
    with_session(app, doc, move |_, s| {
        s.export(&pages, Path::new(&path)).map_err(|e| export_error(&path, e))?;
        Ok(path)
    })
    .await
}

fn export_error(path: &str, e: edits_edit::EditError) -> String {
    match e {
        edits_edit::EditError::Pdf(edits_edit::pdf::PdfError::Damaged(_)) => {
            "this file is damaged, so its pages cannot be written to a new file".into()
        }
        e => format!("could not write {path}: {e}"),
    }
}

/// One part of a split: its pages and the name of its file.
#[derive(Deserialize)]
pub struct Part {
    pub pages: Vec<u32>,
    pub name: String,
}

/// Writes each part as its own file in `folder`. Returns the paths written.
#[tauri::command]
pub async fn pages_split(
    app: tauri::AppHandle,
    doc: DocId,
    parts: Vec<Part>,
    folder: String,
) -> Result<Vec<String>, String> {
    with_session(app, doc, move |_, s| {
        let mut out = Vec::new();
        for p in parts {
            let target = free_name(Path::new(&folder), &p.name);
            let shown = target.display().to_string();
            s.export(&p.pages, &target).map_err(|e| export_error(&shown, e))?;
            out.push(shown);
        }
        Ok(out)
    })
    .await
}

/// `folder/name.pdf`, or `name (2).pdf` and so on if that is taken. Characters
/// Windows does not allow in names are replaced.
fn free_name(folder: &Path, name: &str) -> PathBuf {
    let clean: String =
        name.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches('.');
    let clean: String = clean.chars().take(120).collect();
    let clean = if clean.is_empty() { "part".to_string() } else { clean };
    let mut p = folder.join(format!("{clean}.pdf"));
    let mut k = 2;
    while p.exists() {
        p = folder.join(format!("{clean} ({k}).pdf"));
        k += 1;
    }
    p
}

/// Combines whole files, in order, into a new PDF at `target`.
#[tauri::command]
pub async fn merge_files(paths: Vec<String>, target: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (first, rest) = paths.split_first().ok_or("no files chosen")?;
        let mut s = Session::open(Path::new(first)).map_err(|e| describe(first, e.into()))?;
        for p in rest {
            let at = s.page_count().map_err(err)?;
            s.insert_pdf(at, Path::new(p), None).map_err(|e| describe(p, e))?;
        }
        let n = s.page_count().map_err(err)?;
        s.export(&(0..n).collect::<Vec<_>>(), Path::new(&target)).map_err(|e| export_error(&target, e))?;
        Ok(target)
    })
    .await
    .map_err(err)?
}

/// Writes a new PDF at `target` with one page per picture, in order.
/// `size`: page width and height in points (turned to suit each picture),
/// or null for pages the shape of each picture.
#[tauri::command]
pub async fn images_to_pdf(paths: Vec<String>, target: String, size: Option<[f32; 2]>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut images = Vec::new();
        for p in &paths {
            let bytes = std::fs::read(p).map_err(|e| format!("cannot read {p}: {e}"))?;
            images.push(edits_edit::image::PdfImage::from_bytes(&bytes).map_err(|e| format!("{p}: {e}"))?);
        }
        edits_edit::images_to_pdf(images, size.map(|s| (s[0], s[1])), Path::new(&target))
            .map_err(|e| export_error(&target, e))?;
        Ok(target)
    })
    .await
    .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_names_are_safe_and_free() {
        let dir = std::env::temp_dir().join(format!("edits-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_name(&dir, "a/b: c?"), dir.join("a_b_ c_.pdf"));
        std::fs::write(dir.join("x.pdf"), b"").unwrap();
        assert_eq!(free_name(&dir, "x"), dir.join("x (2).pdf"));
        assert_eq!(free_name(&dir, "  "), dir.join("part.pdf"));
    }
}
