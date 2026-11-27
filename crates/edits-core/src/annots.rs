//! Geometry for editing: page fractions (what the viewer uses: top-left
//! origin, /Rotate applied) to and from PDF user space (what the file uses).

use pdfium_render::prelude::*;

use crate::engine::EngineError;
use crate::text::PageMap;

fn map(page: &PdfPage) -> Result<PageMap, EngineError> {
    PageMap::new(page).ok_or_else(|| EngineError::Render("the page has no usable size".into()))
}

/// Page fractions to PDF user space.
pub(crate) fn to_user_space(page: &PdfPage, points: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, EngineError> {
    let m = map(page)?;
    points
        .iter()
        .map(|p| m.invert(p[0], p[1]).map(|(x, y)| [x, y]))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| EngineError::Render("the page has no usable size".into()))
}

/// PDF user space to page fractions (not clamped to the page).
pub(crate) fn to_fractions(page: &PdfPage, points: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, EngineError> {
    let m = map(page)?;
    Ok(points.iter().map(|p| m.apply_unclamped(p[0], p[1])).map(|(x, y)| [x, y]).collect())
}
