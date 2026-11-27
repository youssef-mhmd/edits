//! Filling forms (PRD 5.4): the fields of a document for the front end,
//! with widget boxes in page fractions, and setting their values.

use edits_core::DocId;
use edits_edit::forms::{Field, FieldValue};
use serde::Serialize;

use crate::edit::{Changed, changed, with_session};

/// A widget of a field: its page and box (left, top, right, bottom in page
/// fractions), and for check boxes and radio buttons its "on" state.
#[derive(Serialize)]
pub struct WidgetOut {
    pub page: u32,
    pub rect: [f32; 4],
    pub on: Option<String>,
}

#[derive(Serialize)]
pub struct FieldOut {
    /// Object number and generation: the field's id.
    pub id: (u32, u16),
    #[serde(flatten)]
    pub field: Field,
    pub boxes: Vec<WidgetOut>,
}

/// The document's form fields, in tab order (page by page, as the file lists them).
#[tauri::command]
pub async fn form_fields(app: tauri::AppHandle, doc: DocId) -> Result<Vec<FieldOut>, String> {
    with_session(app, doc, move |engine, s| {
        let mut out = Vec::new();
        for f in s.fields().map_err(|e| e.to_string())? {
            let mut boxes = Vec::new();
            for w in &f.views {
                let r = w.rect;
                let p =
                    engine.to_fractions(doc, w.page, vec![[r[0], r[1]], [r[2], r[3]]]).map_err(|e| e.to_string())?;
                let rect = [p[0][0].min(p[1][0]), p[0][1].min(p[1][1]), p[0][0].max(p[1][0]), p[0][1].max(p[1][1])];
                boxes.push(WidgetOut { page: w.page, rect, on: w.on.clone() });
            }
            out.push(FieldOut { id: f.id, field: f.field, boxes });
        }
        // Tab order: by page, then as the file lists them.
        out.sort_by_key(|f| f.boxes.first().map_or(u32::MAX, |b| b.page));
        Ok(out)
    })
    .await
}

#[tauri::command]
pub async fn field_set(
    app: tauri::AppHandle,
    doc: DocId,
    id: (u32, u16),
    value: FieldValue,
) -> Result<Changed, String> {
    with_session(app, doc, move |engine, s| {
        let pages = s.set_field(id, value).map_err(|e| e.to_string())?;
        changed(engine, doc, s, pages, None)
    })
    .await
}
