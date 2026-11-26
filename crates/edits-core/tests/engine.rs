use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use edits_core::{Engine, EngineError, RenderSpec};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};

/// Writes a PDF whose pages are white with a black square in the top-left
/// 100x100 pt corner. `sizes` are (width, height) in points.
fn make_pdf(name: &str, sizes: &[(i64, i64)]) -> PathBuf {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut kids = Vec::new();
    for &(w, h) in sizes {
        let ops = vec![
            Operation::new("rg", vec![0.into(), 0.into(), 0.into()]),
            Operation::new("re", vec![0.into(), (h - 100).into(), 100.into(), 100.into()]),
            Operation::new("f", vec![]),
        ];
        let content = doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()));
        kids.push(Object::from(doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content,
            "MediaBox" => vec![0.into(), 0.into(), w.into(), h.into()],
        })));
    }
    let count = kids.len() as i64;
    doc.objects
        .insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", root);
    let dir = std::env::temp_dir().join(format!("edits-core-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    doc.save(&path).unwrap();
    path
}

fn px(tile: &edits_core::Tile, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * tile.w + x) * 4) as usize;
    tile.rgba[i..i + 4].try_into().unwrap()
}

#[test]
fn opens_and_reports_page_sizes() {
    let engine = Engine::start().expect("PDFium available (run scripts/fetch-pdfium)");
    let path = make_pdf("sizes.pdf", &[(200, 300), (400, 100)]);
    let info = engine.open(&path, None).unwrap();
    assert_eq!(info.page_count, 2);
    assert_eq!((info.pages[0].w, info.pages[0].h), (200.0, 300.0));
    assert_eq!((info.pages[1].w, info.pages[1].h), (400.0, 100.0));
}

#[test]
fn renders_tiles_in_page_coordinates() {
    let engine = Engine::start().unwrap();
    let path = make_pdf("tiles.pdf", &[(200, 200)]);
    let doc = engine.open(&path, None).unwrap().doc;
    // Page at 2x: 400x400 px; black square covers the top-left 200x200 px.
    let full = RenderSpec { doc, page: 0, page_w: 400, page_h: 400, x: 0, y: 0, w: 400, h: 400 };
    let tile = engine.render(full, 0).unwrap();
    assert_eq!((tile.w, tile.h), (400, 400));
    assert_eq!(tile.rgba.len(), 400 * 400 * 4);
    assert_eq!(px(&tile, 10, 10), [0, 0, 0, 255]);
    assert_eq!(px(&tile, 390, 390), [255, 255, 255, 255]);

    // A tile at (150, 150) sees black in its top-left 50x50 only.
    let t = engine.render(RenderSpec { x: 150, y: 150, w: 100, h: 100, ..full }, 0).unwrap();
    assert_eq!(px(&t, 10, 10), [0, 0, 0, 255]);
    assert_eq!(px(&t, 60, 60), [255, 255, 255, 255]);
}

#[test]
fn edge_tiles_are_clipped_to_the_page() {
    let engine = Engine::start().unwrap();
    let path = make_pdf("edge.pdf", &[(100, 100)]);
    let doc = engine.open(&path, None).unwrap().doc;
    let t = engine
        .render(RenderSpec { doc, page: 0, page_w: 300, page_h: 300, x: 256, y: 256, w: 256, h: 256 }, 0)
        .unwrap();
    assert_eq!((t.w, t.h), (44, 44));
    assert_eq!(t.rgba.len(), 44 * 44 * 4);
}

#[test]
fn rejects_bad_requests() {
    let engine = Engine::start().unwrap();
    let path = make_pdf("bad.pdf", &[(100, 100)]);
    let doc = engine.open(&path, None).unwrap().doc;
    let ok = RenderSpec { doc, page: 0, page_w: 100, page_h: 100, x: 0, y: 0, w: 100, h: 100 };
    assert!(matches!(engine.render(RenderSpec { page: 5, ..ok }, 0), Err(EngineError::PageOutOfRange(5))));
    assert!(matches!(engine.render(RenderSpec { doc: 999, ..ok }, 0), Err(EngineError::UnknownDoc(999))));
    assert!(matches!(engine.render(RenderSpec { w: 0, ..ok }, 0), Err(EngineError::InvalidSpec(_))));
    assert!(matches!(engine.render(RenderSpec { x: 100, ..ok }, 0), Err(EngineError::InvalidSpec(_))));
    assert!(engine.open(path.with_file_name("missing.pdf"), None).is_err());
}

#[test]
fn damaged_file_is_an_error_not_a_crash() {
    let engine = Engine::start().unwrap();
    let path = std::env::temp_dir().join(format!("edits-garbage-{}.pdf", std::process::id()));
    std::fs::write(&path, b"%PDF-1.7\nthis is not a pdf at all").unwrap();
    assert!(matches!(engine.open(&path, None), Err(EngineError::Open(_))));
}

#[test]
fn cancel_where_fails_queued_jobs_with_cancelled() {
    let engine = Engine::start().unwrap();
    let path = make_pdf("cancel.pdf", &[(500, 500); 4]);
    let doc = engine.open(&path, None).unwrap().doc;
    let (tx, rx) = mpsc::channel();
    // Many large renders so some are still queued when we cancel.
    for i in 0..40 {
        let tx = tx.clone();
        let spec = RenderSpec { doc, page: i % 4, page_w: 4000, page_h: 4000, x: 0, y: 0, w: 2048, h: 2048 };
        engine.render_with(spec, 1, move |r| tx.send(r.map(|_| ())).unwrap());
    }
    let cancelled = engine.cancel_where(|s| s.doc == doc);
    drop(tx);
    let results: Vec<_> = rx.iter().take(40).collect();
    assert_eq!(results.len(), 40, "every job replies exactly once");
    let n_cancelled = results.iter().filter(|r| matches!(r, Err(EngineError::Cancelled))).count();
    assert_eq!(n_cancelled, cancelled);
    assert!(cancelled > 0);
}

#[test]
fn higher_priority_runs_first() {
    let engine = Engine::start().unwrap();
    let path = make_pdf("prio.pdf", &[(500, 500)]);
    let doc = engine.open(&path, None).unwrap().doc;
    let (tx, rx) = mpsc::channel();
    // Occupy the thread, then queue low before high.
    let big = RenderSpec { doc, page: 0, page_w: 4000, page_h: 4000, x: 0, y: 0, w: 4000, h: 4000 };
    let tx0 = tx.clone();
    engine.render_with(big, 0, move |_| tx0.send("big").unwrap());
    std::thread::sleep(Duration::from_millis(5));
    let small = RenderSpec { w: 10, h: 10, ..big };
    let tx1 = tx.clone();
    engine.render_with(small, 1, move |_| tx1.send("low").unwrap());
    let tx2 = tx.clone();
    engine.render_with(small, 9, move |_| tx2.send("high").unwrap());
    drop(tx);
    let order: Vec<_> = rx.iter().collect();
    assert_eq!(order, ["big", "high", "low"]);
}

#[test]
fn password_protected() {
    use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
    use lopdf::{EncryptionState, EncryptionVersion, Permissions};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    let plain = make_pdf("plain-for-enc.pdf", &[(100, 100)]);
    let mut doc = Document::load(&plain).unwrap();
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let state = EncryptionState::try_from(EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        file_encryption_key: &[3u8; 32],
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: "owner",
        user_password: "secret",
        permissions: Permissions::all(),
    })
    .unwrap();
    doc.encrypt(&state).unwrap();
    let path = plain.with_file_name("enc.pdf");
    doc.save(&path).unwrap();

    let engine = Engine::start().unwrap();
    assert!(matches!(engine.open(&path, None), Err(EngineError::PasswordRequired)));
    assert!(matches!(engine.open(&path, Some("wrong".into())), Err(EngineError::PasswordRequired)));
    assert_eq!(engine.open(&path, Some("secret".into())).unwrap().page_count, 1);
}
