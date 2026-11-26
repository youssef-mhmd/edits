//! Bookmarks (outline) and links, read from PDFium.
//!
//! Positions are fractions of the page as displayed with the page's own
//! /Rotate applied and no view rotation: (0, 0) is the top-left corner and
//! (1, 1) the bottom-right. The viewer applies its view rotation on top.

use pdfium_render::prelude::*;
use serde::Serialize;

/// Where a bookmark or internal link goes.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Target {
    /// A page in this document, optionally at a vertical position (0 = top).
    Page { page: u32, y: Option<f32> },
    /// A web address. Only http, https and mailto are passed on.
    Uri { uri: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct OutlineItem {
    pub title: String,
    pub target: Option<Target>,
    pub children: Vec<OutlineItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Link {
    /// [left, top, right, bottom] as page fractions.
    pub rect: [f32; 4],
    pub target: Target,
}

/// Outline entries beyond this are dropped: protects against hostile files.
const MAX_OUTLINE_ITEMS: usize = 20_000;
const MAX_OUTLINE_DEPTH: usize = 32;
/// Device size used for coordinate conversion; PDFium returns whole pixels.
const PRECISION: f32 = 4096.0;

/// Converts a point in PDF user space to page fractions.
fn to_fraction(page: &PdfPage, x: f32, y: f32) -> Option<(f32, f32)> {
    let config = PdfRenderConfig::new().set_fixed_size(PRECISION as Pixels, PRECISION as Pixels);
    let (dx, dy) = page.points_to_pixels(PdfPoints::new(x), PdfPoints::new(y), &config).ok()?;
    Some(((dx as f32 / PRECISION).clamp(0.0, 1.0), (dy as f32 / PRECISION).clamp(0.0, 1.0)))
}

fn safe_uri(uri: &str) -> Option<String> {
    let lower = uri.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p)).then(|| uri.trim().to_string())
}

fn destination_target(doc: &PdfDocument, dest: &PdfDestination) -> Option<Target> {
    let index = dest.page_index().ok()?;
    let page_count = doc.pages().len();
    if index >= page_count {
        return None;
    }
    let y_pt = match dest.view_settings().ok()? {
        PdfDestinationViewSettings::SpecificCoordinatesAndZoom(x, y, _) => y.map(|y| (x.map(|x| x.value), y.value)),
        PdfDestinationViewSettings::FitPageHorizontallyToWindow(y)
        | PdfDestinationViewSettings::FitBoundsHorizontallyToWindow(y) => y.map(|y| (None, y.value)),
        PdfDestinationViewSettings::FitPageToRectangle(r) => Some((Some(r.left().value), r.top().value)),
        _ => None,
    };
    let y = y_pt.and_then(|(x, y)| {
        let page = doc.pages().get(index).ok()?;
        to_fraction(&page, x.unwrap_or(0.0), y).map(|(_, fy)| fy)
    });
    Some(Target::Page { page: index as u32, y })
}

fn action_target(doc: &PdfDocument, action: &PdfAction) -> Option<Target> {
    match action {
        PdfAction::LocalDestination(a) => destination_target(doc, &a.destination().ok()?),
        PdfAction::Uri(a) => safe_uri(&a.uri().ok()?).map(|uri| Target::Uri { uri }),
        _ => None,
    }
}

fn bookmark_target(doc: &PdfDocument, b: &PdfBookmark) -> Option<Target> {
    if let Some(d) = b.destination() {
        return destination_target(doc, &d);
    }
    b.action().and_then(|a| action_target(doc, &a))
}

/// The document's bookmarks as a tree.
pub fn outline(doc: &PdfDocument) -> Vec<OutlineItem> {
    fn walk(doc: &PdfDocument, first: Option<PdfBookmark>, depth: usize, budget: &mut usize) -> Vec<OutlineItem> {
        let mut out = Vec::new();
        let mut next = first;
        while let Some(b) = next {
            if *budget == 0 {
                break;
            }
            *budget -= 1;
            let children =
                if depth < MAX_OUTLINE_DEPTH { walk(doc, b.first_child(), depth + 1, budget) } else { vec![] };
            out.push(OutlineItem {
                title: b.title().unwrap_or_default().trim().to_string(),
                target: bookmark_target(doc, &b),
                children,
            });
            next = b.next_sibling();
        }
        out
    }
    let mut budget = MAX_OUTLINE_ITEMS;
    walk(doc, doc.bookmarks().root(), 0, &mut budget)
}

/// The links on a page.
pub fn links(doc: &PdfDocument, page: &PdfPage) -> Vec<Link> {
    let mut out = Vec::new();
    for link in page.links().iter() {
        let target = link
            .destination()
            .and_then(|d| destination_target(doc, &d))
            .or_else(|| link.action().and_then(|a| action_target(doc, &a)));
        let (Some(target), Ok(r)) = (target, link.rect()) else { continue };
        let (Some((x0, y0)), Some((x1, y1))) =
            (to_fraction(page, r.left().value, r.top().value), to_fraction(page, r.right().value, r.bottom().value))
        else {
            continue;
        };
        // Page rotation can swap corners; normalise.
        let rect = [x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)];
        if rect[2] - rect[0] > 0.0 && rect[3] - rect[1] > 0.0 {
            out.push(Link { rect, target });
        }
    }
    out
}
