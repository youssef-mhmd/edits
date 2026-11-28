//! A page in arabic-ocr's JSON schema ("arabic-ocr/1", export/to_json.py),
//! so arabic-ocr's own tools can score edits' output.

use serde_json::{Value, json};

use crate::pipeline::{Corners, Line, Page};

/// Python's round(x, n) for the values written (one decimal for boxes, four
/// for confidences). Python rounds the exact binary value; formatting does
/// the same.
fn round_to(v: f64, n: usize) -> Value {
    let s = format!("{v:.n$}");
    let r: f64 = s.parse().unwrap_or(v);
    json!(if r == 0.0 { 0.0 } else { r })
}

fn corners(c: &Corners) -> Value {
    Value::Array(c.iter().map(|p| json!([round_to(p[0] as f64, 1), round_to(p[1] as f64, 1)])).collect())
}

fn line(l: &Line) -> Value {
    json!({
        "box": corners(&l.corners),
        "text": l.text,
        "confidence": round_to(l.confidence, 4),
        "words": l.words.iter().map(|w| json!({
            "box": corners(&w.corners),
            "text": w.text,
            "confidence": round_to(w.confidence, 4),
        })).collect::<Vec<_>>(),
    })
}

pub fn page(p: &Page, index: usize) -> Value {
    json!({
        "index": index,
        "width": p.width,
        "height": p.height,
        "angle": p.angle,
        "text": p.text(),
        "blocks": p.blocks.iter().map(|b| json!({
            "box": corners(&b.corners()),
            "text": b.text(),
            "lines": b.lines.iter().map(line).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

pub fn document(pages: &[Page], file: Option<&str>) -> Value {
    json!({
        "schema": "arabic-ocr/1",
        "file": file,
        "pages": pages.iter().enumerate().map(|(i, p)| page(p, i)).collect::<Vec<_>>(),
    })
}
