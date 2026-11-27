# edits: Product Requirements

Status: draft 3, 2026-11-27. M0 spike done; rendering decision recorded in 8.2. Cold start and memory targets decided (section 7, open questions 5 and 6).
Plan and progress: [milestones.md](milestones.md).

## 1. Summary

**edits** is a desktop PDF app: a very fast reader that can also edit PDFs and create new ones. It works offline, and it reads Arabic scans using the models from the owner's `arabic-ocr` project.

The long-term goal is to cover what Adobe Acrobat does. Acrobat is decades of work, so this document splits that goal into a v1.0 that is realistic to build and a list of later work (section 6).

## 2. Goals

1. **Speed first.** The app must feel instant when opening, scrolling, zooming and searching. Section 7 sets measurable targets, and every milestone is checked against them.
2. **Read** any PDF comfortably, including very large ones.
3. **Edit** PDFs: mark up, organize pages, fill forms, change existing text and images.
4. **Create** PDFs: from images and scans, from other PDFs, on a blank canvas, and in a document writer.
5. **Arabic as a first-class language**: right-to-left text, correct letter shaping, OCR for scans, Arabic user interface.
6. **Offline and private.** No file ever leaves the computer.

## 3. Not goals for v1.0

- macOS and Linux builds. Windows 10/11 x64 comes first. The code stays portable so the other two can follow (section 6).
- Cloud storage, accounts, sharing links, real-time collaboration.
- Mobile apps.
- Handwriting OCR and Nastaliq script (arabic-ocr does not support them yet).
- Converting PDF to Word/Excel/PowerPoint with layout kept.

## 4. Users

- People who read and mark up PDFs daily and find current readers slow or heavy.
- People who work with Arabic documents, especially scans, where most PDF tools fail at search, copy and editing.
- People who need occasional edits (fix a typo, reorder pages, sign, fill a form) without paying for Acrobat.

## 5. Features for v1.0

Each feature lists the milestone that delivers it.

### 5.1 Reader (M1)
- Open PDFs by file dialog, drag and drop, double-click in Explorer (file association), and command line.
- Continuous scroll, single page, two-page view; fit width / fit page / custom zoom; rotate view.
- Thumbnails panel, bookmarks (outline) panel, page number jump.
- Select and copy text, including right-to-left text in the correct order.
- Search in the document with all matches highlighted; Arabic search ignores diacritics and tatweel and treats letter variants (أ إ آ ا, ى ي, ة ه) as equal by option.
- Clickable links, internal and web.
- Several documents in tabs; recent files; reopen at the last position.
- Password-protected files.
- Print.
- Light and dark theme; English and Arabic interface, with the layout mirrored for Arabic.

### 5.2 Annotate and mark up (M2)
- Highlight, underline, strikethrough, squiggly.
- Freehand drawing, lines, arrows, rectangles, ellipses.
- Sticky notes and text boxes (Arabic and Latin).
- Stamps and inserted images.
- Annotation list panel; edit, move, resize, delete; colour and thickness.
- Undo and redo for everything.
- Annotations are saved as standard PDF annotations, so other readers show them.

### 5.3 Organize pages (M3)
- Page grid view: reorder by drag, rotate, delete, duplicate, extract.
- Insert pages from another PDF, blank pages, or images.
- Merge several files; split by range, by every N pages, or by bookmarks.
- Crop pages; page numbers, headers, footers, watermarks.

### 5.4 Forms and signing (M4)
- Fill interactive forms: text fields, checkboxes, radio buttons, dropdowns, lists.
- "Fill and sign" for flat forms with no fields: type anywhere, tick marks.
- Signature by drawing, typing, or image; saved for reuse.
- Flatten forms and annotations.

### 5.5 OCR, from arabic-ocr (M5)
- Recognize scanned pages (Arabic, Persian, Urdu, plus the Latin and digits the model covers) and add an invisible text layer, so scans become searchable and their text can be selected and copied.
- Runs in the background, page by page, visible pages first. Cancellable.
- Automatic offer when a page has no text. Whole document or page range.
- Options carried over from arabic-ocr: keep or remove diacritics, digit style.
- Copy text from any region of a page (snip and read).

### 5.6 Create PDFs (M5, M6, M9)
- **From images and scans** (M5): pick or drop images, order them, choose page size, optionally run OCR, save.
- **From other PDFs** (M3): pick pages from several files into a new one.
- **Blank canvas** (M6): new document with empty pages; place text boxes, images, shapes, lines and tables freely; alignment guides and snapping.
- **Document writer** (M9): a simple word processor. Flowing paragraphs, headings, lists, tables, images, page breaks, headers and footers, page numbers, right-to-left paragraphs. Exports to PDF. Keeps its own editable source file so the document can be reopened and changed.

### 5.7 Edit existing content (M7, M8)
- Edit text that is already in the PDF: click a line or paragraph, type, delete, change font, size and colour.
- Move, resize, replace and delete images and other page objects.
- Edit links and bookmarks.
- **Arabic text editing** (M8) with correct shaping and right-to-left order.
- **Edit scanned text** (M8): OCR a scanned line, change it, and write it back over the scan in a matching font.

Honest limit: a PDF stores positioned glyphs, not paragraphs, and often embeds only the letters the document used. Edits therefore work as follows:
- If the original font is installed or fully embedded, the edit uses it.
- If not, the app picks the closest available font and tells the user.
- Reflow happens inside the edited paragraph only. Text does not flow to the next page.

### 5.8 Protect, clean and export (M10)
- Password and permission encryption (AES-256); remove a known password.
- Redaction that really removes the content under the box, not only covers it.
- Remove metadata and hidden data.
- Compress (downsample images, remove unused objects).
- Export pages as PNG/JPEG; export text as TXT.
- Document properties editor.

## 6. After v1.0

Ordered by expected value. Each becomes its own milestone later.

1. macOS (Apple Silicon) and Linux builds.
2. Form designer (create and edit form fields).
3. Certificate-based digital signatures and signature validation.
4. Compare two PDFs.
5. PDF/A export and validation; accessibility tags.
6. Export to Word; import from Word and images of tables.
7. Batch actions (apply an operation to a folder), Bates numbering.
8. Comments import/export, measuring tools, layers, portfolios, JavaScript in forms, preflight.

## 7. Performance targets

Measured on a mid-range laptop (4 cores, 8 GB RAM, SSD), release build. A benchmark suite runs these automatically from M0 on, and a regression blocks the milestone.

| Measure | Target |
|---|---|
| Cold start to usable window (no file) | ≤ 500 ms *(was 400; WebView2 start-up alone is about 400 ms)* |
| Double-click a 10 MB PDF to first page shown | ≤ 600 ms |
| Open a 1,000-page, 500 MB PDF to first page shown | ≤ 1 s, without loading the whole file |
| Scrolling | 60 fps; a page never shows blank (low-resolution preview first, sharp within 100 ms) |
| Zoom step to sharp page | ≤ 100 ms |
| Search a 500-page text PDF, first result | ≤ 200 ms; all results ≤ 2 s |
| Save after adding annotations | ≤ 200 ms (incremental save) |
| Memory, one document open | ≤ 200 MB above the app with no document open *(all processes, WebView2 included; that baseline is about 160 MB)* |
| OCR one A4 page at 300 DPI | ≤ 1.5 s on 4 cores (same as arabic-ocr today), never blocking the interface |
| Installer size | ≤ 60 MB, including OCR models |

## 8. Technical design

The stack below is the plan. M0 is a spike that confirms or changes the items marked *(verify in M0)*.

### 8.1 Stack
- **Shell:** Tauri 2. Rust backend, web front end in the system WebView2. Small installer, fast start.
- **Front end:** TypeScript + Svelte 5, built with Vite. Chosen for small bundles and low runtime cost.
- **PDF engine:** PDFium (Google's engine from Chrome, BSD/Apache license) through the `pdfium-render` crate. Used for rendering, text extraction, search, forms, annotations, page operations and page-object editing.
- **Low-level PDF work** that PDFium does not cover (encryption, real redaction, cleanup, compression): `lopdf`. Verified in M0: it loads, decrypts (AES-256), cleans and saves every valid corpus file. It reads whole files into memory, so it is never used on the open or render path ([m0-results.md](m0-results.md)).
- **Text shaping and layout for writing text:** `rustybuzz` (HarfBuzz port), `unicode-bidi`, `fontdb`, and a font subsetter. Arabic text is never written without shaping.
- **OCR:** ONNX Runtime through the `ort` crate.
- **Not used:** MuPDF (AGPL license, which would force the whole app to be AGPL; see section 10.1).

### 8.2 How rendering stays fast
- The interface never waits for the engine. All PDF work runs on backend threads.
- Pages are rendered as tiles at the current zoom. Only visible tiles and a small margin are rendered. Requests have priorities and are cancelled when the user scrolls past.
- Two-level cache: low-resolution page previews (kept for the whole document) and sharp tiles (bounded by memory).
- Files are memory-mapped and pages are loaded on demand, so file size does not affect open time.
- PDFium allows only one thread at a time. One dedicated render thread with a priority queue does all PDFium work in the app's process. Pages that are slow to draw (a render over 40 ms) go to render worker processes, up to three, in parallel (M1.4). A worker that crashes or hangs on a page loses only that page; a render that is no longer wanted is stopped by ending its worker.
- Text and search indexes are built in the background after the first page is shown.

**Main risk:** moving page bitmaps from Rust into the WebView. The plan was a custom URL protocol that streams tiles to `<img>`/canvas elements, with a native GPU surface (wgpu) for the page area as the fallback.

**Decision (M0, 2026-11-26): WebView tiles, delivered through WebView2 shared buffers.** Measurements are in [m0-results.md](m0-results.md).
- The custom URL protocol works but costs about 10 ms per 1 MB tile, which puts zoom-to-sharp at 146 ms (target 100). Tauri's binary IPC is no faster.
- WebView2 shared buffers (`ICoreWebView2Environment12::CreateSharedBuffer`, `PostSharedBufferToScript`) cut that to about 1.5 ms. Zoom-to-sharp is then 60–80 ms, pages are sharp 20 ms after scrolling into view, and scrolling holds the display rate with no blank pages.
- The native surface is not needed for speed. It would also not fix the two targets M0 misses (cold start, memory), because both come from WebView2 itself, which the rest of the interface keeps using.
- The URL protocol stays as the transport on platforms without WebView2 (macOS, Linux; section 6).

### 8.3 Code layout
```
crates/
  edits-core/     document model, PDFium wrapper, render queue, caches, save
  edits-text/     shaping, bidi, fonts, text search and normalization
  edits-edit/     annotations, page ops, forms, content editing, undo/redo
  edits-ocr/      Rust port of the arabic-ocr pipeline
  edits-writer/   document writer layout and PDF export
  edits-bench/    benchmark suite for section 7
src-tauri/        Tauri app, commands, tile protocol
ui/               Svelte front end
models/           OCR .onnx files (copied from arabic-ocr releases)
docs/             this PRD, milestones
```
The core crates do not depend on Tauri, so they can be tested and benchmarked alone.

### 8.4 Editing model
- Every change is a command with undo and redo, held in one history per document.
- The original file is never modified until the user saves. Autosave writes a recovery file.
- Annotations and form values use incremental save (append to the file). Structural edits use a full rewrite.
- Writes go to a temporary file and then replace the original, so a crash cannot corrupt a document.

### 8.5 arabic-ocr integration
arabic-ocr (at `C:\Users\jousef\Desktop\arabic-ocr`) is Python: an ONNX detector and recognizer, with pre- and post-processing in Python (`preprocess`, `detect`, `recognize/ctc`, `layout`, `bidi`, `correct`, `charset`, `export/searchable_pdf`).

- **Reused as is:** the released `.onnx` models and `lexicon.tsv.gz`. No retraining.
- **Ported to Rust** in `edits-ocr`: the processing steps listed above. Shipping Python would add about 300 MB and slow start, so no Python is shipped.
- **Parity tests:** a fixed set of pages is run through arabic-ocr (Python) once and the results stored. The Rust port must produce the same text, and boxes within 2 px, on every page. Accuracy must match arabic-ocr's published CER figures.
- **Rules carried over:** text is kept in logical order; lines are read whole, never split into letters; `charset` stays the single source of truth and is generated from the Python file, not retyped.
- **License:** models are CC BY-SA 4.0. The app must show the attribution from arabic-ocr's `NOTICE` file in its About screen and licenses file.
- Model improvements in arabic-ocr reach edits by copying new `.onnx` files, as long as input and output shapes stay the same.

### 8.6 Document writer
The writer needs a layout engine that breaks text into lines and pages, including right-to-left text. Two candidates, compared in a short spike at the start of M9:
1. **Typst as a library** (Rust, good Arabic support, produces PDF directly). The editor model is converted to Typst and the preview is the real PDF output.
2. **Own layout** on top of `edits-text`. More control, much more work.

Recommendation: option 1, unless the spike shows the edit-to-preview delay is above 100 ms.

## 9. Quality
- Test corpus: a collection of real PDFs (Arabic and Latin, scanned and digital, forms, encrypted, damaged, very large). Every file must open without a crash, and every file saved by edits must open in Chrome, Edge and Acrobat Reader.
- Unit tests in each crate; end-to-end tests that drive the app.
- Malformed PDFs are common and can be hostile. The app must never crash or hang on a bad file; parsing errors are shown per page.
- Benchmarks from section 7 run on every milestone.

## 10. Distribution
- Windows installer (NSIS through Tauri), per-user install, no admin rights needed.
- Registers as a handler for `.pdf`; the user chooses whether to make it the default.
- Auto-update through the Tauri updater.
- Code signing: not at first, so Windows SmartScreen shows a warning. Decide before v1.0.

### 10.1 License
edits is fully open source. Decided 2026-11-15.

- **Code:** Apache-2.0, the same as arabic-ocr *(proposed; open question 4)*.
- **OCR models:** CC BY-SA 4.0, unchanged from arabic-ocr, with its attribution.
- **Dependencies** must have licenses compatible with Apache-2.0: MIT, BSD, Apache-2.0, MPL-2.0 and similar. GPL and AGPL libraries are not used, because they would force the whole app under their license. This is why PDFium is used and MuPDF is not.
- Bundled fonts must allow redistribution (for example the SIL Open Font License).
- A `NOTICE` file and an in-app licenses screen list every dependency, model and font. CI checks dependency licenses (`cargo-deny`).
- Development is public on GitHub from M0: source, issues and releases.

## 11. Risks

| Risk | Effect | Response |
|---|---|---|
| Bitmap transfer to the WebView is too slow | Misses the speed goal | Decided in M0; native surface fallback (8.2) |
| Editing existing text, especially Arabic | Largest and least predictable feature | Split in two milestones (M7 Latin, M8 Arabic); clear limits in 5.7; ships after the reader is already useful |
| "Everything Acrobat does" has no end | v1.0 never ships | Fixed v1.0 scope in section 5; the rest is section 6 |
| Rust OCR port differs from Python | Lower accuracy | Parity tests in 8.5 |
| PDFium single-thread limit | Slow on heavy pages | Priority queue, cancellation, worker processes |
| Document writer grows into a second product | Delays v1.0 | Scope fixed in 5.6; M9 can move after v1.0 if needed |

## 12. Open questions

1. Is the Arabic interface needed in the first release, or can English come first?
2. Should the document writer (M9) be part of v1.0, or the first release after it? It is the one item that could be moved to ship sooner.
3. Is a paid code-signing certificate acceptable for the v1.0 release?
4. Is Apache-2.0 the right license (section 10.1), or is another open-source license preferred?
5. ~~Cold start target~~ **Decided 2026-11-27:** ≤ 500 ms. WebView2 start-up alone takes about 400 ms, so 400 ms was not reachable as measured.
6. ~~Memory target~~ **Decided 2026-11-27:** counted for all processes, WebView2 included, as growth over the app with no document open: ≤ 200 MB above that baseline (about 160 MB).
