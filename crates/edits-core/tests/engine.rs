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
    let full = RenderSpec { doc, page: 0, page_w: 400, page_h: 400, x: 0, y: 0, w: 400, h: 400, rotation: 0 };
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
        .render(RenderSpec { doc, page: 0, page_w: 300, page_h: 300, x: 256, y: 256, w: 256, h: 256, rotation: 0 }, 0)
        .unwrap();
    assert_eq!((t.w, t.h), (44, 44));
    assert_eq!(t.rgba.len(), 44 * 44 * 4);
}

#[test]
fn rejects_bad_requests() {
    let engine = Engine::start().unwrap();
    let path = make_pdf("bad.pdf", &[(100, 100)]);
    let doc = engine.open(&path, None).unwrap().doc;
    let ok = RenderSpec { doc, page: 0, page_w: 100, page_h: 100, x: 0, y: 0, w: 100, h: 100, rotation: 0 };
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
        let spec =
            RenderSpec { doc, page: i % 4, page_w: 4000, page_h: 4000, x: 0, y: 0, w: 2048, h: 2048, rotation: 0 };
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
    let big = RenderSpec { doc, page: 0, page_w: 4000, page_h: 4000, x: 0, y: 0, w: 4000, h: 4000, rotation: 0 };
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

#[test]
fn view_rotation_turns_the_page_clockwise() {
    let engine = Engine::start().unwrap();
    // 300 x 200 pt page, black square in its top-left 100 x 100 pt.
    let path = make_pdf("rotate.pdf", &[(300, 200)]);
    let doc = engine.open(&path, None).unwrap().doc;
    // A quarter turn clockwise: on screen 200 wide, 300 tall; the square is now top-right.
    let spec = RenderSpec { doc, page: 0, page_w: 200, page_h: 300, x: 0, y: 0, w: 200, h: 300, rotation: 1 };
    let t = engine.render(spec, 0).unwrap();
    assert_eq!((t.w, t.h), (200, 300));
    assert_eq!(px(&t, 150, 50), [0, 0, 0, 255]);
    assert_eq!(px(&t, 50, 50), [255, 255, 255, 255]);
    // Half turn: bottom-right.
    let t = engine.render(RenderSpec { page_w: 300, page_h: 200, w: 300, h: 200, rotation: 2, ..spec }, 0).unwrap();
    assert_eq!(px(&t, 250, 150), [0, 0, 0, 255]);
    assert_eq!(px(&t, 50, 50), [255, 255, 255, 255]);
    // Three quarters: bottom-left.
    let t = engine.render(RenderSpec { rotation: 3, ..spec }, 0).unwrap();
    assert_eq!(px(&t, 50, 250), [0, 0, 0, 255]);
    assert_eq!(px(&t, 150, 50), [255, 255, 255, 255]);
}

#[test]
fn a_broken_page_does_not_stop_the_document() {
    // Three pages; the middle entry in the page tree is not a page at all.
    let path = make_pdf("broken-middle.pdf", &[(200, 300), (200, 300)]);
    let mut doc = Document::load(&path).unwrap();
    let not_a_page = doc.add_object(Object::Integer(42));
    let pages_id = doc.catalog().unwrap().get(b"Pages").unwrap().as_reference().unwrap();
    if let Ok(Object::Dictionary(d)) = doc.get_object_mut(pages_id) {
        let mut kids = d.get(b"Kids").unwrap().as_array().unwrap().clone();
        kids.insert(1, not_a_page.into());
        d.set("Kids", kids);
        d.set("Count", 3);
    }
    let path = path.with_file_name("broken-middle-out.pdf");
    doc.save(&path).unwrap();

    let engine = Engine::start().unwrap();
    let info = engine.open(&path, None).expect("document opens despite one bad page");
    let spec =
        |page| RenderSpec { doc: info.doc, page, page_w: 100, page_h: 150, x: 0, y: 0, w: 100, h: 150, rotation: 0 };
    // Good pages render; the bad one fails alone, whatever index PDFium gives it.
    let results: Vec<bool> = (0..info.page_count).map(|p| engine.render(spec(p), 0).is_ok()).collect();
    assert!(results.iter().filter(|ok| **ok).count() >= 2, "{results:?}");
}

#[test]
fn reopening_under_memory_pressure_keeps_rendering_correct() {
    let engine = Engine::start().unwrap();
    // A 1 MB limit forces a reopen at every memory check.
    engine.set_memory_limit_mb(1);
    let path = make_pdf("reopen.pdf", &[(200, 200); 60]);
    let doc = engine.open(&path, None).unwrap().doc;
    for round in 0..2 {
        for page in 0..60 {
            let spec = RenderSpec { doc, page, page_w: 200, page_h: 200, x: 0, y: 0, w: 200, h: 200, rotation: 0 };
            let t = engine.render(spec, 0).unwrap_or_else(|e| panic!("round {round} page {page}: {e}"));
            assert_eq!(px(&t, 10, 10), [0, 0, 0, 255]);
            assert_eq!(px(&t, 150, 150), [255, 255, 255, 255]);
        }
    }
}

/// Three 200 x 300 pt pages with bookmarks and links:
/// - outline: "Chapter 1" -> page 1 (with child "Section 1.1" -> top of page 2), "Chapter 2" -> page 3 at y = 150 pt
/// - page 1: a link at the top-left to page 3 at y = 150 pt, a web link at the bottom,
///   and a javascript: link that must be ignored.
pub fn make_nav_pdf(name: &str) -> PathBuf {
    let path = make_pdf(name, &[(200, 300), (200, 300), (200, 300)]);
    let mut doc = Document::load(&path).unwrap();
    let pages: Vec<lopdf::ObjectId> = doc.get_pages().values().copied().collect();
    let (p1, p2, p3) = (pages[0], pages[1], pages[2]);
    let xyz = |p: lopdf::ObjectId, y: i64| -> Object {
        vec![p.into(), "XYZ".into(), Object::Null, y.into(), Object::Null].into()
    };
    let outlines = doc.new_object_id();
    let (o1, c1, o2) = (doc.new_object_id(), doc.new_object_id(), doc.new_object_id());
    doc.objects
        .insert(outlines, dictionary! { "Type" => "Outlines", "First" => o1, "Last" => o2, "Count" => 2 }.into());
    doc.objects.insert(
        o1,
        dictionary! {
            "Title" => Object::string_literal("Chapter 1"), "Parent" => outlines, "Next" => o2,
            "First" => c1, "Last" => c1, "Count" => 1,
            "Dest" => vec![p1.into(), "Fit".into()],
        }
        .into(),
    );
    doc.objects.insert(
        c1,
        dictionary! { "Title" => Object::string_literal("Section 1.1"), "Parent" => o1, "Dest" => xyz(p2, 300) }.into(),
    );
    doc.objects.insert(o2, dictionary! { "Title" => Object::string_literal("Chapter 2"), "Parent" => outlines, "Prev" => o1, "Dest" => xyz(p3, 150) }.into());
    let internal = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![20.into(), 250.into(), 80.into(), 280.into()],
        "Dest" => xyz(p3, 150),
    });
    let web = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![100.into(), 20.into(), 180.into(), 40.into()],
        "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("https://example.com/a") },
    });
    let script = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => vec![100.into(), 60.into(), 180.into(), 80.into()],
        "A" => dictionary! { "S" => "URI", "URI" => Object::string_literal("javascript:alert(1)") },
    });
    if let Ok(Object::Dictionary(d)) = doc.get_object_mut(p1) {
        d.set("Annots", vec![internal.into(), web.into(), script.into()]);
    }
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    if let Ok(Object::Dictionary(d)) = doc.get_object_mut(root) {
        d.set("Outlines", outlines);
    }
    let out = path.with_file_name(format!("nav-{name}"));
    doc.save(&out).unwrap();
    out
}

#[test]
fn reads_outline_tree() {
    use edits_core::Target;
    let engine = Engine::start().unwrap();
    let doc = engine.open(make_nav_pdf("outline.pdf"), None).unwrap().doc;
    let outline = engine.outline(doc).unwrap();
    assert_eq!(outline.len(), 2);
    assert_eq!(outline[0].title, "Chapter 1");
    assert_eq!(outline[0].target, Some(Target::Page { page: 0, y: None }));
    assert_eq!(outline[0].children.len(), 1);
    assert_eq!(outline[0].children[0].title, "Section 1.1");
    assert_eq!(outline[0].children[0].target, Some(Target::Page { page: 1, y: Some(0.0) }));
    match &outline[1].target {
        Some(Target::Page { page: 2, y: Some(y) }) => assert!((y - 0.5).abs() < 0.01, "y = {y}"),
        other => panic!("unexpected target {other:?}"),
    }
    // A document without bookmarks.
    let plain = engine.open(make_pdf("no-outline.pdf", &[(100, 100)]), None).unwrap().doc;
    assert!(engine.outline(plain).unwrap().is_empty());
}

#[test]
fn reads_links_and_drops_unsafe_ones() {
    use edits_core::Target;
    let engine = Engine::start().unwrap();
    let doc = engine.open(make_nav_pdf("links.pdf"), None).unwrap().doc;
    let links = engine.links(doc, 0, 0).unwrap();
    assert_eq!(links.len(), 2, "{links:?}");
    let internal = links.iter().find(|l| matches!(l.target, Target::Page { .. })).unwrap();
    // Rect 20..80 x 250..280 pt (from the bottom) on a 200 x 300 page.
    let [l, t, r, b] = internal.rect;
    assert!((l - 0.10).abs() < 0.01 && (r - 0.40).abs() < 0.01, "{:?}", internal.rect);
    assert!((t - 20.0 / 300.0).abs() < 0.01 && (b - 50.0 / 300.0).abs() < 0.01, "{:?}", internal.rect);
    assert!(matches!(internal.target, Target::Page { page: 2, y: Some(y) } if (y - 0.5).abs() < 0.01));
    assert!(links.iter().any(|l| l.target == Target::Uri { uri: "https://example.com/a".into() }));
    assert!(engine.links(doc, 1, 0).unwrap().is_empty());
}

/// One 300 x 200 pt page with two lines of Helvetica text.
fn make_text_pdf(name: &str) -> PathBuf {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let ops = vec![
        Operation::new("BT", vec![]),
        Operation::new("Tf", vec!["F1".into(), 12.into()]),
        Operation::new("Td", vec![20.into(), 150.into()]),
        Operation::new("Tj", vec![Object::string_literal("Hello world")]),
        Operation::new("Td", vec![0.into(), (-20).into()]),
        Operation::new("Tj", vec![Object::string_literal("Second line, hello again")]),
        Operation::new("ET", vec![]),
    ];
    let content = doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content,
        "MediaBox" => vec![0.into(), 0.into(), 300.into(), 200.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }),
    );
    let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", root);
    let dir = std::env::temp_dir().join(format!("edits-core-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    doc.save(&path).unwrap();
    path
}

#[test]
fn page_text_has_lines_and_boxes() {
    let engine = Engine::start().unwrap();
    let doc = engine.open(make_text_pdf("text.pdf"), None).unwrap().doc;
    let t = engine.page_text(doc, 0, 0).unwrap();
    assert_eq!(t.text, "Hello world\nSecond line, hello again");
    assert_eq!(t.boxes.len(), t.text.encode_utf16().count() * 4);
    // "H" starts 20 pt from the left of a 300 pt page, in the upper half.
    let [l, top, r, b] = [t.boxes[0], t.boxes[1], t.boxes[2], t.boxes[3]];
    assert!((l - 20.0 / 300.0).abs() < 0.01, "{l}");
    assert!(r > l && b > top && b < 0.5, "{:?}", &t.boxes[..4]);
    // The inserted line break has no box.
    let nl = t.text.chars().position(|c| c == '\n').unwrap();
    assert_eq!(&t.boxes[nl * 4..nl * 4 + 4], &[0.0; 4]);
}

#[test]
fn index_and_search() {
    use edits_core::SearchOptions;
    let engine = Engine::start().unwrap();
    let doc = engine.open(make_text_pdf("search.pdf"), None).unwrap().doc;
    assert_eq!(engine.index_text(doc, 0, 10, 0).unwrap(), 0);
    let hits = engine.search_pages(doc, vec![0], "hello".into(), SearchOptions::default(), 0).unwrap();
    assert_eq!(hits, vec![(0, vec![[0, 5], [25, 30]])]);
    let case = SearchOptions { match_case: true, ..SearchOptions::default() };
    assert_eq!(engine.search_pages(doc, vec![0], "hello".into(), case, 0).unwrap(), vec![(0, vec![[25, 30]])]);
    // A line break matches a space.
    let hits = engine.search_pages(doc, vec![0], "world second".into(), SearchOptions::default(), 0).unwrap();
    assert_eq!(hits, vec![(0, vec![[6, 18]])]);
}

// ---------------------------------------------------------------- workers

/// A page with very many small vector paths: slow for PDFium to draw.
fn make_heavy_pdf(name: &str) -> PathBuf {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut ops = vec![Operation::new("w", vec![0.2.into()])];
    for i in 0..60_000i64 {
        let (x, y) = ((i * 37) % 590, (i * 53) % 830);
        ops.push(Operation::new("m", vec![x.into(), y.into()]));
        ops.push(Operation::new("l", vec![(x + 5).into(), (y + 3).into()]));
        ops.push(Operation::new("S", vec![]));
    }
    // A black square in the top-left corner, as in `make_pdf`.
    ops.push(Operation::new("rg", vec![0.into(), 0.into(), 0.into()]));
    ops.push(Operation::new("re", vec![0.into(), 742.into(), 100.into(), 100.into()]));
    ops.push(Operation::new("f", vec![]));
    let content = doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }),
    );
    let root = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", root);
    let dir = std::env::temp_dir().join(format!("edits-core-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    doc.save(&path).unwrap();
    path
}

fn worker_cmd() -> edits_core::worker::WorkerCommand {
    edits_core::worker::WorkerCommand {
        exe: env!("CARGO_BIN_EXE_edits-render-worker").into(),
        args: vec![edits_core::worker::WORKER_ARG.into()],
    }
}

/// Renders once on the render thread so the page is marked heavy. Returns
/// the spec used, or None if this machine draws the page too fast to count.
fn mark_heavy(engine: &Engine, doc: u32) -> Option<RenderSpec> {
    let spec = RenderSpec { doc, page: 0, page_w: 595, page_h: 842, x: 0, y: 0, w: 512, h: 512, rotation: 0 };
    let t = engine.render(spec, 1).unwrap();
    engine.is_heavy(doc, 0).then_some(spec).or_else(|| {
        eprintln!("page rendered in {} us; not heavy here, skipping", t.render_us);
        None
    })
}

#[test]
fn heavy_pages_render_in_workers_with_the_same_pixels() {
    let engine = Engine::start().unwrap();
    engine.enable_workers(worker_cmd(), 2);
    let doc = engine.open(make_heavy_pdf("heavy-a.pdf"), None).unwrap().doc;
    let Some(spec) = mark_heavy(&engine, doc) else { return };
    let on_thread = engine.render(spec, 1).unwrap();
    assert!(engine.workers() >= 1, "a worker took the heavy render");
    // Several at once: they run in parallel and all come back right.
    let (tx, rx) = mpsc::channel();
    for x in [0, 512] {
        for y in [0, 512] {
            let tx = tx.clone();
            engine.render_with(RenderSpec { x, y, ..spec }, 1, move |r| tx.send(r).unwrap());
        }
    }
    drop(tx);
    let tiles: Vec<_> = rx.iter().map(|r| r.unwrap()).collect();
    assert_eq!(tiles.len(), 4);
    let first = tiles.iter().find(|t| t.spec.x == 0 && t.spec.y == 0).unwrap();
    assert_eq!(first.rgba, on_thread.rgba);
    assert_eq!(px(first, 10, 10), [0, 0, 0, 255]);
}

#[test]
fn cancelling_a_running_heavy_render_stops_it() {
    let engine = Engine::start().unwrap();
    engine.enable_workers(worker_cmd(), 1);
    let doc = engine.open(make_heavy_pdf("heavy-b.pdf"), None).unwrap().doc;
    let Some(spec) = mark_heavy(&engine, doc) else { return };
    // Warm the worker up (start, open the file) so the next render is running when cancelled.
    engine.render(spec, 1).unwrap();
    let full = RenderSpec { page_w: 2380, page_h: 3368, w: 2380, h: 3368, ..spec };
    let (tx, rx) = mpsc::channel();
    let start = std::time::Instant::now();
    engine.render_with(full, 1, move |r| tx.send(r).unwrap());
    std::thread::sleep(Duration::from_millis(50));
    engine.cancel_where(|s| s.doc == doc);
    let r = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(r, Err(EngineError::Cancelled) | Ok(_)), "{r:?}");
    if r.is_err() {
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    // A new worker takes the next render.
    assert_eq!(px(&engine.render(spec, 1).unwrap(), 10, 10), [0, 0, 0, 255]);
}

#[test]
fn heavy_pages_still_render_when_workers_cannot_start() {
    let engine = Engine::start().unwrap();
    engine.enable_workers(edits_core::worker::WorkerCommand { exe: "no-such-worker.exe".into(), args: vec![] }, 2);
    let doc = engine.open(make_heavy_pdf("heavy-c.pdf"), None).unwrap().doc;
    let Some(spec) = mark_heavy(&engine, doc) else { return };
    let t = engine.render(spec, 1).unwrap();
    assert_eq!(px(&t, 10, 10), [0, 0, 0, 255]);
}

// ------------------------------------------------------------ annotations

/// Average colour of a tile region, to see whether a highlight is drawn.
fn mean(tile: &edits_core::Tile, x0: u32, y0: u32, x1: u32, y1: u32) -> [u32; 3] {
    let mut s = [0u64; 3];
    let mut n = 0u64;
    for y in y0..y1 {
        for x in x0..x1 {
            let p = px(tile, x, y);
            for k in 0..3 {
                s[k] += p[k] as u64;
            }
            n += 1;
        }
    }
    s.map(|v| (v / n.max(1)) as u32)
}

#[test]
fn edits_show_through_the_overlay_and_survive_saving() {
    use edits_edit::{Annot, Kind, Session};
    let engine = Engine::start().unwrap();
    let path = make_text_pdf("annot.pdf");
    let original = std::fs::read(&path).unwrap();
    let doc = engine.open(&path, None).unwrap().doc;
    // Page is 300 x 200 pt; render it at 600 x 400 px.
    let spec = RenderSpec { doc, page: 0, page_w: 600, page_h: 400, x: 0, y: 0, w: 600, h: 400, rotation: 0 };
    // A band across the top half (fractions, top-left origin).
    let corners = engine.to_user_space(doc, 0, vec![[0.1, 0.1], [0.9, 0.1], [0.1, 0.4], [0.9, 0.4]]).unwrap();
    assert!((corners[0][0] - 30.0).abs() < 0.5 && (corners[0][1] - 180.0).abs() < 0.5, "{corners:?}");
    let back = engine.to_fractions(doc, 0, vec![corners[0]]).unwrap();
    assert!((back[0][0] - 0.1).abs() < 1e-3 && (back[0][1] - 0.1).abs() < 1e-3);
    let quad: [f32; 8] = corners.concat().try_into().unwrap();
    let mut a = Annot::new(0, Kind::Highlight);
    a.quads = vec![quad];
    a.fit_rect();
    a.color = [1.0, 0.85, 0.0];
    let band = |t: &edits_core::Tile| mean(t, 100, 60, 500, 140);
    let white = band(&engine.render(spec, 1).unwrap());
    assert!(white[2] > 230, "{white:?}");

    let mut s = Session::open(&path).unwrap();
    assert_eq!(s.page_count().unwrap(), 1);
    let (id, _) = s.add(a).unwrap();
    engine.set_overlay(doc, s.tail()).unwrap();
    let shown = band(&engine.render(spec, 1).unwrap());
    assert!(shown[2] < 100 && shown[0] > 200, "highlighted band is yellow: {shown:?}");

    s.undo().unwrap();
    engine.set_overlay(doc, s.tail()).unwrap();
    assert_eq!(band(&engine.render(spec, 1).unwrap()), white);
    s.redo().unwrap();
    engine.set_overlay(doc, s.tail()).unwrap();
    assert!(band(&engine.render(spec, 1).unwrap())[2] < 100);

    // Save while PDFium has the file open, then reopen it plain.
    s.save(&path).unwrap();
    engine.reload(doc, None).unwrap();
    assert!(std::fs::read(&path).unwrap().starts_with(&original), "incremental: the original bytes are kept");
    assert!(band(&engine.render(spec, 1).unwrap())[2] < 100, "drawn from the saved file");

    // Another reader sees one highlight with its appearance stream.
    let l = Document::load(&path).unwrap();
    let page = l.get_pages()[&1];
    let annots = l.get_object(page).unwrap().as_dict().unwrap().get(b"Annots").unwrap().as_array().unwrap().clone();
    assert_eq!(annots.len(), 1);
    let ad = l.get_object(annots[0].as_reference().unwrap()).unwrap().as_dict().unwrap();
    assert_eq!(ad.get(b"Subtype").unwrap().as_name().unwrap(), b"Highlight");
    assert!(ad.get(b"AP").is_ok());

    // Delete after saving, save again: the page has no annotations left.
    s.remove(id).unwrap();
    engine.set_overlay(doc, s.tail()).unwrap();
    assert_eq!(band(&engine.render(spec, 1).unwrap()), white);
    s.save(&path).unwrap();
    engine.reload(doc, None).unwrap();
    assert_eq!(band(&engine.render(spec, 1).unwrap()), white);
}
