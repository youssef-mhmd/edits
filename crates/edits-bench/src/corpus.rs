//! Synthetic part of the test corpus.
//!
//! Real-world files (Arabic, scans, government forms) cannot be generated and
//! are listed in `corpus/README.md`. These synthetic files cover the sizes and
//! failure cases the benchmarks and crash checks need, and are reproducible
//! from any checkout with `edits-bench gen-corpus`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use lopdf::content::{Content, Operation};
use lopdf::encryption::crypt_filters::{Aes256CryptFilter, CryptFilter};
use lopdf::{Document, EncryptionState, EncryptionVersion, Object, ObjectId, Permissions, Stream, dictionary};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// Small deterministic generator so the corpus is identical on every machine.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

const LOREM: &str = "The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs. \
Sphinx of black quartz, judge my vow. How vexingly quick daft zebras jump. ";

struct Builder {
    doc: Document,
    pages_id: ObjectId,
    font_id: ObjectId,
    kids: Vec<Object>,
}

impl Builder {
    fn new() -> Self {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        Self { doc, pages_id, font_id, kids: Vec::new() }
    }

    fn text_ops(&self, page_no: usize, w: f32, h: f32) -> Vec<Operation> {
        let mut ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 20.into()]),
            Operation::new("Td", vec![50.into(), (h - 60.0).into()]),
            Operation::new("Tj", vec![Object::string_literal(format!("Page {page_no}"))]),
            Operation::new("Tf", vec!["F1".into(), 10.into()]),
            Operation::new("TL", vec![13.into()]),
        ];
        let chars_per_line = ((w - 100.0) / 5.0).max(10.0) as usize;
        let lines = ((h - 140.0) / 13.0).max(1.0) as usize;
        let text: String = LOREM.chars().cycle().skip(page_no * 7).take(chars_per_line * lines).collect();
        for (i, line) in text.as_bytes().chunks(chars_per_line).enumerate() {
            if i == 0 {
                ops.push(Operation::new("Td", vec![0.into(), (-30).into()]));
            }
            ops.push(Operation::new("Tj", vec![Object::string_literal(line.to_vec())]));
            ops.push(Operation::new("T*", vec![]));
        }
        ops.push(Operation::new("ET", vec![]));
        ops
    }

    /// Adds a page. `extra` content runs after the text.
    fn page(
        &mut self,
        w: f32,
        h: f32,
        rotate: i64,
        mut extra: Vec<Operation>,
        xobjects: Vec<(String, ObjectId)>,
    ) -> ObjectId {
        let n = self.kids.len() + 1;
        let mut ops = Vec::new();
        ops.append(&mut extra);
        ops.extend(self.text_ops(n, w, h));
        let content = Content { operations: ops };
        let content_id = self.doc.add_object(Stream::new(dictionary! {}, content.encode().expect("encode")));
        let mut xo = lopdf::Dictionary::new();
        for (name, id) in xobjects {
            xo.set(name, id);
        }
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), w.into(), h.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => self.font_id }, "XObject" => xo },
        };
        if rotate != 0 {
            page.set("Rotate", rotate);
        }
        let id = self.doc.add_object(page);
        self.kids.push(id.into());
        id
    }

    fn noise_image(&mut self, rng: &mut Rng, w: u32, h: u32) -> ObjectId {
        let mut data = vec![0u8; (w * h * 3) as usize];
        rng.fill(&mut data);
        // Smooth rows a little so it looks like a photo rather than static,
        // while staying incompressible enough to keep the file large.
        for px in data.chunks_mut(3) {
            px[1] = px[0] / 2 + px[1] / 4 + 64;
        }
        let mut stream = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => w as i64, "Height" => h as i64,
                "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
            },
            data,
        );
        stream.allows_compression = false;
        self.doc.add_object(stream)
    }

    fn finish(mut self) -> Document {
        let count = self.kids.len() as i64;
        self.doc.objects.insert(
            self.pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => self.kids, "Count" => count }),
        );
        let catalog = self.doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => self.pages_id });
        self.doc.trailer.set("Root", catalog);
        self.doc
    }
}

fn image_ops(name: &str, x: f32, y: f32, w: f32, h: f32) -> Vec<Operation> {
    vec![
        Operation::new("q", vec![]),
        Operation::new("cm", vec![w.into(), 0.into(), 0.into(), h.into(), x.into(), y.into()]),
        Operation::new("Do", vec![Object::Name(name.as_bytes().to_vec())]),
        Operation::new("Q", vec![]),
    ]
}

const A4: (f32, f32) = (595.0, 842.0);

fn text_doc(pages: usize) -> Document {
    let mut b = Builder::new();
    for _ in 0..pages {
        b.page(A4.0, A4.1, 0, vec![], vec![]);
    }
    let mut doc = b.finish();
    doc.compress();
    doc
}

fn image_doc(pages: usize, img_w: u32, img_h: u32, seed: u64) -> Document {
    let mut b = Builder::new();
    let mut rng = Rng(seed);
    for _ in 0..pages {
        let img = b.noise_image(&mut rng, img_w, img_h);
        let ops = image_ops("Im1", 50.0, 300.0, 495.0, 450.0);
        b.page(A4.0, A4.1, 0, ops, vec![("Im1".into(), img)]);
    }
    b.finish()
}

fn vector_heavy(pages: usize, lines_per_page: usize) -> Document {
    let mut b = Builder::new();
    let mut rng = Rng(7);
    for _ in 0..pages {
        let mut ops = vec![Operation::new("w", vec![0.3.into()])];
        for i in 0..lines_per_page {
            let r = |rng: &mut Rng, max: f32| (rng.next() % 10_000) as f32 / 10_000.0 * max;
            if i % 500 == 0 {
                let c = |rng: &mut Rng| ((rng.next() % 100) as f32 / 100.0).into();
                ops.push(Operation::new("RG", vec![c(&mut rng), c(&mut rng), c(&mut rng)]));
            }
            // Short local strokes, like a map or CAD drawing.
            let (x, y) = (r(&mut rng, A4.0), r(&mut rng, A4.1));
            ops.push(Operation::new("m", vec![x.into(), y.into()]));
            ops.push(Operation::new(
                "c",
                (0..6).map(|k| (if k % 2 == 0 { x } else { y } + r(&mut rng, 30.0) - 15.0).into()).collect(),
            ));
            ops.push(Operation::new("S", vec![]));
        }
        b.page(A4.0, A4.1, 0, ops, vec![]);
    }
    let mut doc = b.finish();
    doc.compress();
    doc
}

fn mixed_sizes() -> Document {
    let mut b = Builder::new();
    let sizes: [(f32, f32, i64); 8] = [
        (595.0, 842.0, 0),   // A4
        (612.0, 792.0, 0),   // Letter
        (842.0, 595.0, 0),   // A4 landscape
        (595.0, 842.0, 90),  // A4 rotated
        (595.0, 842.0, 270), // A4 rotated the other way
        (2384.0, 3370.0, 0), // A0
        (144.0, 144.0, 0),   // 2 inch square
        (612.0, 4000.0, 0),  // very tall strip
    ];
    for (w, h, r) in sizes {
        b.page(w, h, r, vec![], vec![]);
    }
    b.finish()
}

/// Five pages where pages 2 to 5 are each broken in a different way. The file
/// opens; each broken page must show an error or render what it can, and
/// never crash the app.
fn damaged_pages() -> Document {
    let mut b = Builder::new();
    let mut rng = Rng(5);
    b.page(A4.0, A4.1, 0, vec![], vec![]);
    // 2: content stream claims Flate compression but holds garbage.
    let mut junk = vec![0u8; 4096];
    rng.fill(&mut junk);
    let bad_content = b.doc.add_object(Stream::new(dictionary! { "Filter" => "FlateDecode" }, junk.clone()));
    let p2 = b.page(A4.0, A4.1, 0, vec![], vec![]);
    if let Ok(Object::Dictionary(d)) = b.doc.get_object_mut(p2) {
        d.set("Contents", bad_content);
    }
    // 3: the page tree points at something that is not a page.
    let not_a_page = b.doc.add_object(Object::Integer(42));
    b.kids.push(not_a_page.into());
    // 4: empty media box.
    let p4 = b.page(A4.0, A4.1, 0, vec![], vec![]);
    if let Ok(Object::Dictionary(d)) = b.doc.get_object_mut(p4) {
        d.set("MediaBox", vec![0.into(), 0.into(), 0.into(), 0.into()]);
    }
    // 5: a JPEG image whose data is garbage.
    let img = b.doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => 400, "Height" => 300,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode",
        },
        junk,
    ));
    b.page(A4.0, A4.1, 0, image_ops("Im1", 50.0, 300.0, 400.0, 300.0), vec![("Im1".into(), img)]);
    b.finish()
}

fn acroform() -> Document {
    let mut b = Builder::new();
    let page = b.page(A4.0, A4.1, 0, vec![], vec![]);
    let field = b.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx",
        "T" => Object::string_literal("name"),
        "V" => Object::string_literal("Filled value"),
        "Rect" => vec![50.into(), 50.into(), 300.into(), 80.into()],
        "P" => page, "F" => 4,
        "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
        "MK" => dictionary! { "BG" => vec![0.9.into(), 0.9.into(), 1.0.into()] },
    });
    let check = b.doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "FT" => "Btn",
        "T" => Object::string_literal("agree"),
        "V" => "Off",
        "Rect" => vec![320.into(), 50.into(), 340.into(), 70.into()],
        "P" => page, "F" => 4,
    });
    if let Ok(Object::Dictionary(d)) = b.doc.get_object_mut(page) {
        d.set("Annots", vec![field.into(), check.into()]);
    }
    let helv = b.doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let mut doc = b.finish();
    let form = doc.add_object(dictionary! {
        "Fields" => vec![field.into(), check.into()],
        "NeedAppearances" => true,
        "DA" => Object::string_literal("/Helv 12 Tf 0 g"),
        "DR" => dictionary! { "Font" => dictionary! { "Helv" => helv } },
    });
    if let Ok(root) = doc.trailer.get(b"Root").and_then(Object::as_reference)
        && let Ok(Object::Dictionary(d)) = doc.get_object_mut(root)
    {
        d.set("AcroForm", form);
    }
    doc
}

fn encrypt(mut doc: Document, user: &str, owner: &str) -> Res<Document> {
    // A fixed key keeps the file reproducible. Test data only.
    let key = [7u8; 32];
    let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
    let version = EncryptionVersion::V5 {
        encrypt_metadata: true,
        crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
        file_encryption_key: &key,
        stream_filter: b"StdCF".to_vec(),
        string_filter: b"StdCF".to_vec(),
        owner_password: owner,
        user_password: user,
        permissions: Permissions::PRINTABLE | Permissions::COPYABLE,
    };
    let state = EncryptionState::try_from(version)?;
    doc.encrypt(&state)?;
    Ok(doc)
}

fn save(mut doc: Document, dir: &Path, name: &str) -> Res<std::path::PathBuf> {
    let path = dir.join(name);
    doc.save(&path)?;
    let size = std::fs::metadata(&path)?.len();
    println!("  {name:<34} {:>8.1} MB", size as f64 / 1e6);
    Ok(path)
}

/// Writes the synthetic corpus into `dir/synthetic`. `large` adds the 500 MB file.
pub fn generate(dir: &Path, large: bool) -> Res<()> {
    let dir = dir.join("synthetic");
    std::fs::create_dir_all(&dir)?;
    println!("Writing synthetic corpus to {}", dir.display());

    let text = save(text_doc(20), &dir, "latin-text-20p.pdf")?;
    save(text_doc(2000), &dir, "latin-text-2000p.pdf")?;
    save(image_doc(10, 640, 520, 1), &dir, "images-10mb.pdf")?;
    save(vector_heavy(5, 20_000), &dir, "vector-heavy-5p.pdf")?;
    save(mixed_sizes(), &dir, "mixed-sizes-rotated.pdf")?;
    save(acroform(), &dir, "form-acroform.pdf")?;
    save(damaged_pages(), &dir, "damaged-pages-2-to-5.pdf")?;
    save(encrypt(text_doc(3), "edits", "owner")?, &dir, "encrypted-aes256-user-edits.pdf")?;
    save(encrypt(text_doc(3), "", "owner")?, &dir, "encrypted-aes256-owner-only.pdf")?;

    // Damaged files, derived from the text document.
    let bytes = std::fs::read(&text)?;
    std::fs::write(dir.join("damaged-truncated.pdf"), &bytes[..bytes.len() * 6 / 10])?;
    let mut bad_xref = bytes.clone();
    if let Some(pos) = find(&bad_xref, b"startxref") {
        // Point startxref at garbage.
        let tail = b"startxref\n12345\n%%EOF\n";
        bad_xref.truncate(pos);
        bad_xref.extend_from_slice(tail);
    }
    std::fs::write(dir.join("damaged-bad-startxref.pdf"), bad_xref)?;
    let mut garbage = b"%PDF-1.7\n".to_vec();
    let mut junk = vec![0u8; 64 * 1024];
    Rng(99).fill(&mut junk);
    garbage.extend(junk);
    std::fs::write(dir.join("damaged-garbage.pdf"), garbage)?;
    std::fs::write(dir.join("damaged-empty.pdf"), b"")?;
    println!("  damaged-*.pdf (4 files)");

    if large {
        println!("  generating 1,000-page / ~500 MB file (takes a while)...");
        save(image_doc(1000, 408, 408, 2), &dir, "large-1000p-500mb.pdf")?;
    }
    Ok(())
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).rposition(|w| w == needle)
}

/// Makes an image-only "scan" of `input`: each page rendered at 150 DPI in
/// grey, with noise, placed slightly skewed. No text layer, like a scanner's output.
pub fn make_scan(input: &Path, output: &Path) -> Res<()> {
    let engine = edits_core::Engine::start()?;
    let info = engine.open(input, None)?;
    let mut b = Builder::new();
    let mut rng = Rng(42);
    for (i, p) in info.pages.iter().enumerate() {
        let (w, h) = ((p.w / 72.0 * 150.0) as u32, (p.h / 72.0 * 150.0) as u32);
        let spec = edits_core::RenderSpec {
            doc: info.doc,
            page: i as u32,
            page_w: w,
            page_h: h,
            x: 0,
            y: 0,
            w,
            h,
            rotation: 0,
        };
        let tile = engine.render(spec, 0)?;
        let mut grey = Vec::with_capacity((tile.w * tile.h) as usize);
        for px in tile.rgba.as_chunks::<4>().0 {
            let l = (px[0] as u32 * 3 + px[1] as u32 * 6 + px[2] as u32) / 10;
            // Paper tone, toner noise and speckles.
            let n = (rng.next() % 25) as i32 - 12;
            let speck = if rng.next().is_multiple_of(4000) { -160 } else { 0 };
            grey.push((l as i32 * 92 / 100 + 10 + n + speck).clamp(0, 255) as u8);
        }
        let mut img = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => tile.w as i64, "Height" => tile.h as i64,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            grey,
        );
        img.compress()?;
        let img_id = b.doc.add_object(img);
        let angle: f32 = (if i % 2 == 0 { 0.6f32 } else { -0.4 }).to_radians();
        let (c, s) = (angle.cos(), angle.sin());
        let (pw, ph) = (p.w, p.h);
        let ops = vec![
            Operation::new("q", vec![]),
            Operation::new(
                "cm",
                vec![(pw * c).into(), (pw * s).into(), (-ph * s).into(), (ph * c).into(), 6.into(), 0.into()],
            ),
            Operation::new("Do", vec![Object::Name(b"Scan".to_vec())]),
            Operation::new("Q", vec![]),
        ];
        // A page with only the image: no text drawn.
        let content = b.doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode()?));
        let mut xo = lopdf::Dictionary::new();
        xo.set("Scan", img_id);
        let page = b.doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => b.pages_id, "Contents" => content,
            "MediaBox" => vec![0.into(), 0.into(), pw.into(), ph.into()],
            "Resources" => dictionary! { "XObject" => xo },
        });
        b.kids.push(page.into());
    }
    let doc = b.finish();
    let dir = output.parent().unwrap_or(Path::new("."));
    save(doc, dir, &output.file_name().unwrap_or_default().to_string_lossy())?;
    Ok(())
}
