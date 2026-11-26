//! Tile transfer to the WebView.
//!
//! Two transports, so M0 can measure both:
//! - `tile` URL scheme: `GET http://tile.localhost/{doc}/{page}/{page_w}/{page_h}/{x}/{y}/{w}/{h}[/{rotation}]?p={priority}`.
//!   Body is raw RGBA; size and timings are in `x-*` headers.
//! - `render_tile_ipc` command: same render, returned as a binary IPC response
//!   with a 16-byte header (w, h, render_us, queued_us as little-endian u32).

use edits_core::{Engine, EngineError, Priority, RenderSpec, Tile};
use tauri::UriSchemeResponder;
use tauri::http::{Request, Response, StatusCode};

fn parse(request: &Request<Vec<u8>>) -> Option<(RenderSpec, Priority)> {
    let uri = request.uri();
    let nums: Vec<u32> = uri.path().trim_matches('/').split('/').map(|s| s.parse().ok()).collect::<Option<_>>()?;
    // An optional ninth segment is the view rotation in quarter turns.
    let (&[doc, page, page_w, page_h, x, y, w, h], rest) = nums.split_first_chunk::<8>()?;
    let rotation = match rest {
        [] => 0,
        [r] => u8::try_from(*r).ok()?,
        _ => return None,
    };
    let priority = uri
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("p=")))
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    Some((RenderSpec { doc, page, page_w, page_h, x, y, w, h, rotation }, priority))
}

fn response(status: StatusCode, body: Vec<u8>, tile: Option<&Tile>) -> Response<Vec<u8>> {
    let mut b = Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Expose-Headers", "x-w, x-h, x-render-us, x-queued-us")
        .header("Cache-Control", "no-store")
        .header("Content-Type", "application/octet-stream");
    if let Some(t) = tile {
        b = b
            .header("x-w", t.w)
            .header("x-h", t.h)
            .header("x-render-us", t.render_us)
            .header("x-queued-us", t.queued_us);
    }
    b.body(body).unwrap_or_default()
}

pub fn handle(engine: &Engine, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let Some((spec, priority)) = parse(&request) else {
        responder.respond(response(StatusCode::BAD_REQUEST, b"bad tile path".to_vec(), None));
        return;
    };
    engine.render_with(spec, priority, move |result| {
        let r = match result {
            Ok(mut tile) => {
                let body = std::mem::take(&mut tile.rgba);
                response(StatusCode::OK, body, Some(&tile))
            }
            // 409: the client moved on; not an error worth logging.
            Err(EngineError::Cancelled) => response(StatusCode::CONFLICT, Vec::new(), None),
            Err(e) => response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string().into_bytes(), None),
        };
        responder.respond(r);
    });
}

#[tauri::command]
pub async fn render_tile_ipc(
    state: tauri::State<'_, crate::AppState>,
    spec: RenderSpec,
    priority: Priority,
) -> Result<tauri::ipc::Response, EngineError> {
    let (tx, rx) = tauri::async_runtime::channel(1);
    state.engine.render_with(spec, priority, move |r| {
        let _ = tx.blocking_send(r);
    });
    let mut rx = rx;
    let tile = rx.recv().await.ok_or(EngineError::Stopped)??;
    let mut body = Vec::with_capacity(16 + tile.rgba.len());
    for v in [tile.w, tile.h, tile.render_us as u32, tile.queued_us as u32] {
        body.extend_from_slice(&v.to_le_bytes());
    }
    body.extend_from_slice(&tile.rgba);
    Ok(tauri::ipc::Response::new(body))
}
