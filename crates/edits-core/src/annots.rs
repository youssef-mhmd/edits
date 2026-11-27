//! PDFium's copy of a document's annotations (the "layer", see
//! `edits_edit::edits`): created and hidden here so pages render with the
//! unsaved edits. Saving writes the edits into the file and reopens the
//! document, which starts the layer again from the file.

use edits_edit::{Annot, AnnotId, Kind, LayerOp};
use pdfium_render::prelude::*;

use crate::docs::{OpenDoc, describe};
use crate::engine::EngineError;
use crate::text::PageMap;

/// Page fractions (top-left origin, /Rotate applied) to PDF user space.
pub(crate) fn to_user_space(page: &PdfPage, points: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, EngineError> {
    let map = PageMap::new(page).ok_or_else(|| EngineError::Render("the page has no usable size".into()))?;
    points
        .iter()
        .map(|p| map.invert(p[0], p[1]).map(|(x, y)| [x, y]))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| EngineError::Render("the page has no usable size".into()))
}

fn color(c: [f32; 3], opacity: f32) -> PdfColor {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    PdfColor::new(b(c[0]), b(c[1]), b(c[2]), b(opacity))
}

/// Creates `a` on its page and returns its index among the page's annotations.
fn create(od: &mut OpenDoc, a: &Annot) -> Result<u32, EngineError> {
    let err = |e: PdfiumError| EngineError::Render(describe(&e));
    let page = od.page_mut(a.page)?;
    let annots = page.annotations_mut();
    let points = |q: &[f32; 8]| PdfQuadPoints::new_from_values(q[0], q[1], q[2], q[3], q[4], q[5], q[6], q[7]);
    let rect = PdfRect::new_from_values(a.rect[1], a.rect[0], a.rect[3], a.rect[2]);
    let c = color(a.color, a.opacity);
    macro_rules! markup {
        ($create:ident) => {{
            let mut n = annots.$create().map_err(err)?;
            for q in &a.quads {
                n.attachment_points_mut().create_attachment_point_at_end(points(q)).map_err(err)?;
            }
            n.set_bounds(rect).map_err(err)?;
            n.set_stroke_color(c).map_err(err)?;
            n.set_is_printed(true).map_err(err)?;
            if !a.contents.is_empty() {
                n.set_contents(&a.contents).map_err(err)?;
            }
            if !a.author.is_empty() {
                n.set_creator(&a.author).map_err(err)?;
            }
        }};
    }
    match a.kind {
        Kind::Highlight => markup!(create_highlight_annotation),
        Kind::Underline => markup!(create_underline_annotation),
        Kind::StrikeOut => markup!(create_strikeout_annotation),
        Kind::Squiggly => markup!(create_squiggly_annotation),
    }
    Ok(page.annotations().len().saturating_sub(1) as u32)
}

fn set_hidden(od: &mut OpenDoc, page: u32, index: u32, hidden: bool) -> Result<(), EngineError> {
    let err = |e: PdfiumError| EngineError::Render(describe(&e));
    let page = od.page_mut(page)?;
    let mut annot = page.annotations().get(index as PdfPageAnnotationIndex).map_err(err)?;
    annot.set_is_hidden(hidden).map_err(err)
}

/// Applies layer operations. Returns, for each created annotation, its index
/// or the reason it could not be created. The document is marked changed.
pub(crate) fn apply(od: &mut OpenDoc, ops: &[LayerOp]) -> Vec<(AnnotId, Result<u32, EngineError>)> {
    od.dirty = true;
    let mut created = Vec::new();
    for op in ops {
        match op {
            LayerOp::Create { id, annot } => created.push((*id, create(od, annot))),
            LayerOp::SetHidden { page, index, hidden } => {
                if let Err(e) = set_hidden(od, *page, *index, *hidden) {
                    log::warn!(
                        "cannot {} annotation {index} on page {page}: {e}",
                        if *hidden { "hide" } else { "show" }
                    );
                }
            }
        }
    }
    created
}
