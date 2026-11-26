//! Tile transport through WebView2 shared buffers (Windows).
//!
//! The render result is copied once into shared memory that the WebView's
//! renderer process maps directly, instead of being serialised through a
//! URL-scheme response. Script receives it as a `sharedbufferreceived`
//! event carrying an ArrayBuffer and `{id, w, h, render_us, queued_us}`.
//!
//! WebView2 calls must run on the UI thread, so posting goes through
//! `with_webview`, which dispatches there.

// COM calls into WebView2 are FFI.
#![allow(unsafe_code)]

use edits_core::{EngineError, Priority, RenderSpec, Tile};
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY, ICoreWebView2_17, ICoreWebView2Environment12,
};
use windows_core::{HSTRING, Interface, PCWSTR};

fn post(pw: tauri::webview::PlatformWebview, tile: &Tile, id: u64) -> windows_core::Result<()> {
    let meta = HSTRING::from(format!(
        r#"{{"id":{id},"w":{},"h":{},"render_us":{},"queued_us":{}}}"#,
        tile.w, tile.h, tile.render_us, tile.queued_us
    ));
    // SAFETY: plain COM calls on the UI thread with live interface pointers
    // from Tauri. The copy writes exactly `len` bytes into a buffer that
    // WebView2 just allocated with that size.
    unsafe {
        let env: ICoreWebView2Environment12 = pw.environment().cast()?;
        let len = tile.rgba.len();
        let buffer = env.CreateSharedBuffer(len as u64)?;
        let mut ptr: *mut u8 = std::ptr::null_mut();
        buffer.Buffer(&mut ptr)?;
        std::ptr::copy_nonoverlapping(tile.rgba.as_ptr(), ptr, len);
        let core: ICoreWebView2_17 = pw.controller().CoreWebView2()?.cast()?;
        core.PostSharedBufferToScript(&buffer, COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY, PCWSTR(meta.as_ptr()))?;
        // Script holds its own reference; this releases ours.
        buffer.Close()?;
    }
    Ok(())
}

/// Renders a tile and posts it to the calling webview as a shared buffer.
/// Resolves once posted; fails with "cancelled" if the engine dropped it.
#[tauri::command]
pub async fn render_tile_shm(
    webview: tauri::Webview,
    state: tauri::State<'_, crate::AppState>,
    spec: RenderSpec,
    priority: Priority,
    id: u64,
) -> Result<(), EngineError> {
    let (tx, mut rx) = tauri::async_runtime::channel(1);
    state.engine.render_with(spec, priority, move |r| {
        let _ = tx.blocking_send(r);
    });
    let tile = rx.recv().await.ok_or(EngineError::Stopped)??;
    webview
        .with_webview(move |pw| {
            if let Err(e) = post(pw, &tile, id) {
                log::error!("posting shared buffer failed: {e}");
            }
        })
        .map_err(|e| EngineError::Render(e.to_string()))
}
