# edits: Milestones

Requirements: [PRD.md](PRD.md). Work on the current milestone only, and tick boxes as items land.

Rules for every milestone:
- It ends with an installable Windows build that can be used for real work.
- The benchmark suite (PRD section 7) passes. A speed regression blocks the milestone.
- The test corpus opens without crashes, and files saved by edits open in Chrome, Edge and Acrobat Reader.

Sizes are rough: **S** about 1 week, **M** 2 to 3 weeks, **L** 4 to 6 weeks, **XL** 8 weeks or more, for one person working with Claude Code.

| # | Milestone | Size | Release |
|---|---|---|---|
| M0 | Foundation and speed spike | S | none |
| M1 | Reader (M1.1 to M1.5) | L | 0.1 |
| M2 | Annotate and mark up | M | 0.2 |
| M3 | Organize pages | M | 0.3 |
| M4 | Forms and signing | M | 0.4 |
| M5 | OCR and PDFs from scans | L | 0.5 |
| M6 | Blank canvas editor | M | 0.6 |
| M7 | Edit existing content | XL | 0.7 |
| M8 | Arabic text editing | L | 0.8 |
| M9 | Document writer | XL | 0.9 |
| M10 | Protect, clean, export, release | M | 1.0 |

---

## M0: Foundation and speed spike

Goal: prove the architecture reaches the speed targets before building features.

- [x] Git repository, Cargo workspace, Tauri 2 app, Svelte 5 + Vite front end, CLAUDE.md
- [ ] LICENSE (Apache-2.0), NOTICE, README; public GitHub repository *(files done; repository not created yet)*
- [ ] CI on Windows: build, tests, clippy, rustfmt, front-end lint, dependency license check (`cargo-deny`) *(workflow written; runs once the repository is on GitHub)*
- [x] PDFium bundled and loaded through `pdfium-render`
- [x] Prototype: open a PDF, render tiles on a backend thread, show them in the WebView with scroll and zoom
- [x] Tile transfer through a custom URL protocol; measure against PRD section 7 *(plus IPC and WebView2 shared buffers; see m0-results.md)*
- [x] Render queue with priorities and cancellation
- [x] `edits-bench`: automated start, open, scroll, zoom measurements
- [ ] Test corpus collected (Arabic and Latin, scans, forms, encrypted, damaged, 1,000+ pages) *(synthetic and generated parts done; real-world files still needed, see corpus/README.md)*
- [x] Check `lopdf` for encryption, cleanup and object-level edits on the corpus
- [x] **Decision recorded in the PRD:** WebView tiles, or native GPU surface for the page area *(WebView tiles through WebView2 shared buffers)*

Done when: the prototype meets the start, open, scroll and zoom targets on the corpus, or the fallback is chosen and meets them.

Status 2026-11-26: open, scroll and zoom targets pass. Cold start (470 ms vs 400) and memory (350–380 MB vs 200) do not; both are WebView2 baseline costs (PRD open questions 5 and 6). Results: [m0-results.md](m0-results.md).

## M1: Reader (release 0.1)

Split into five parts. Each part ends with an installable build that is better than the last; release 0.1 ships at the end of M1.5.

Done when: it can replace the current default PDF reader for daily use, and all PRD section 7 reader targets pass.

### M1.1: Open and view (S)

- [x] Open by dialog, drag and drop, command line, and `.pdf` file association *(offered in "Open with" and Default apps; never takes over the user's default)*
- [x] Continuous, single-page and two-page views; fit width, fit page, custom zoom; rotate view *(two-page view has a right-to-left option for Arabic books)*
- [x] Go to page; keyboard navigation (Page Up/Down, Home/End)
- [x] Two-level cache: low-resolution previews and sharp tiles, bounded by memory *(previews for the whole document, filled in the background and kept as WebP; PDFium's own caches bounded by reopening the document over a memory limit)*
- [x] Password-protected files
- [x] Per-page error display for damaged files; no crash on any corpus file *(synthetic corpus; real-world files come in M1.4)*

Notes: a second double-click opens a second window until tabs arrive in M1.2. Jumping far into a long document before its previews are filled in shows the page blank for about 40 ms; afterwards about one frame.

### M1.2: Navigate (M)

- [x] Thumbnails panel, bookmarks panel *(F4; thumbnails reuse the background previews; bookmark tree marks the current section)*
- [x] Links, internal and web *(web links: http, https and mailto only, opened in the default browser; Back and Forward with Alt ← / Alt →)*
- [x] Tabs; a second launch opens its file as a tab in the running window
- [x] Recent files, restore last position *(page, position on the page, zoom, layout and rotation)*

Notes: thumbnails are shown unrotated when the view is rotated.

### M1.3: Text (M)

- [x] Text selection and copy, correct for right-to-left text *(drag, double-click word, triple-click line, Ctrl A page, Ctrl C; reading order rebuilt from glyph positions because PDFium's order reverses many Arabic words)*
- [x] Search with highlighted matches; Arabic normalization options *(Ctrl F, F3; diacritics and tatweel, letter variants and Arabic-Indic digits, case, whole words; results stream in from the current page)*
- [x] Background text index *(starts 1.5 s after opening, two pages per job at the lowest priority)*

Notes:
- Known limits of the reading-order rebuild, for M8: fully vowelled text from some producers (Chrome) can get spaces inside words; a number written as one left-to-right unit inside Arabic (a phone number with spaces) comes out with its groups reversed, because the PDF does not record the grouping; a lam-alef with tanween from Chrome can split. Regression check: `edits-bench text-check` against `tests/text`.
- Search: with the index built, 2,000 dense pages search in about 1 s (first result 4 ms). Before the index is built, text extraction costs about 3.6 ms per dense page, mostly in PDFium, so a cold search of 500 dense pages takes about 2 s; render worker processes (M1.4) would let indexing run in parallel.
- Benchmarks during M1.3 were noisy (the M1.2 build measured slower than before on the same day); rerun on a quiet machine in M1.4.

### M1.4: Speed and memory (S)

- [x] Heavy pages: render worker processes in parallel, progressive rendering that can be cancelled, larger tiles for slow pages *(up to three workers for pages over 40 ms; a render no longer wanted is stopped by ending its worker, since PDFium's pause-based progressive API needs unsafe FFI; 1024 px tiles for slow pages)*
- [ ] Decide PRD open questions 5 and 6 (cold start and memory targets) and meet the result *(decided 2026-11-27: ≤ 500 ms, ≤ 200 MB above the no-file start. Cold start passes (436 ms). Memory does not: 236–568 MB above idle after scrolling and zooming, mostly the WebView2 GPU process)*
- [ ] Benchmarks on a reference-class machine (4 cores, 8 GB) *(emulated with a 4-processor affinity mask only; real laptop still needed)*
- [x] Real-world files in the corpus; no crash on any of them *(28 files, 104 damaged copies; 149 files with the synthetic ones, none crashes or hangs)*

Notes: results and what was tried in [m1-results.md](m1-results.md). Still failing: memory; heavy pages (the real 107 MB Arabic textbook: page sharp 189 ms, zoom 133 ms; vector-heavy-5p: first page 730 ms, page sharp 132 ms); on 4 cores, cold start 557 ms and opening a 10 MB file 666 ms.

### M1.5: Shell and release 0.1 (M)

- [x] Print *(Ctrl P: all, current page or a range, 150 or 300 dpi, then the WebView2 print dialog)*
- [x] Light and dark theme; settings *(theme follows Windows by default; Ctrl ,)*
- [x] English and Arabic interface with mirrored layout *(page view keeps left-to-right geometry)*
- [x] Windows installer and auto-update *(signed NSIS installer, 6.4 MB, English and Arabic; updater reads latest.json from GitHub releases. The update itself is first exercised when 0.2 is published)*
- [x] Release 0.1 *(published 2026-11-27: https://github.com/youssef-mhmd/edits/releases/tag/v0.1.0; notes in [release-notes.md](release-notes.md))*

Notes: M1 ships with the open M1.4 items above (memory over target, heavy pages, reference machine); the owner chose to release and continue with M2.

## M2: Annotate and mark up (release 0.2)

- [x] Command history with undo and redo (used by every later milestone) *(`edits-edit`; Ctrl Z, Ctrl Y)*
- [x] Save, Save As, incremental save, safe write through a temporary file *(Ctrl S, Ctrl Shift S; changes are appended to the original bytes; encrypted files are rewritten whole by PDFium; asks before closing a tab or the window with unsaved changes)*
- [x] Autosave recovery file *(written after every change; reopening after a crash offers to restore, only if the file is unchanged)*
- [x] Highlight, underline, strikethrough, squiggly *(from the text selection, five colours, with appearance streams)*
- [x] Freehand, line, arrow, rectangle, ellipse *(pen, line, arrow, rectangle, ellipse tools; colour, width, fill)*
- [x] Sticky notes and text boxes, with Arabic shaping (`edits-text`: rustybuzz, bidi, font embedding and subsetting) *(IBM Plex Sans Arabic, OFL; subset embedded with ToUnicode; double-click to edit)*
- [x] Stamps and images *(word stamps in Arabic and English; JPEG and PNG pictures, PNG transparency kept)*
- [x] Select, move, resize, restyle, delete; annotation list panel *(click to select, drag to move, handles to resize, Delete; colour, width, fill, note; Comments tab in the side panel)*
- [x] Annotations made in other apps display and can be edited *(moved, resized, deleted and given notes keeping their own look; recoloured when edits can draw the kind; links and form fields are left alone)*

Done when: annotations saved by edits show correctly in Chrome, Edge and Acrobat Reader, and saving takes ≤ 200 ms.

Notes so far:
- Design: edits reads files with its own lazy reader (`edits-bench pdf-check`: all 73 intact corpus files match PDFium) and writes incremental updates itself; PDFium shows file + update, so the screen is what will be saved.
- Save (`edits-bench save FILE`): 12 ms (0.1 MB), 19 ms (1.4 MB, 2,000 pages), 21 ms (10 MB), 96 ms (107 MB book); 380 ms for 506 MB (copying the file). An edit shows in 1-16 ms. PDFium's own full rewrite takes 252 ms for 10 MB and 12 s for 506 MB.
- Encrypted files cannot be annotated yet (M10); damaged files only after Save As. *(Encrypted files: done in M4.)*
- Status 2026-11-27: every item is in. Save is 13 ms (0.1 MB), 29 ms (10 MB), 95 ms (107 MB): the ≤ 200 ms target passes for these; a 506 MB file takes 380 ms (copying it). Benchmarks unchanged from M1 (memory still over target, see m1-results.md). UI scenarios: tests/ui/annotate.json, draw.json, tools.json, then restore.json for crash recovery. Remaining for "done when": the owner checks `edits-bench annot-demo` output in Acrobat Reader, Chrome and Edge.
- Saved files checked with a second, independent renderer (MuPDF, local check only): shapes, ink and arrows draw from edits' appearance streams.
- Done 2026-11-27: the owner checked the `annot-demo` sample in Acrobat Reader, Chrome and Edge; every annotation shows correctly. M2 is complete.

## M3: Organize pages (release 0.3)

- [x] Page grid view with drag reorder and multi-select *("Pages" button; click, Ctrl-click, Shift-click, Ctrl+A; drag, or Ctrl+arrow to move)*
- [x] Rotate, delete, duplicate, extract *(copies keep their annotations and links; Extract saves the chosen pages as a new PDF)*
- [x] Insert from another PDF, blank pages, images *(Insert menu in the grid, or drop files on it; pictures get a page of their own shape, at most A4)*
- [x] Merge files; new PDF from pages of several files *("Combine PDFs" on the start screen; or insert pages from other files, then Save As or Extract)*
- [x] Split by range, every N pages, bookmarks
- [x] Crop pages *(margins in mm, chosen or all pages; Remove crop)*
- [x] Page numbers, headers, footers, watermarks *(drawn into the page content with the bundled Arabic font, upright on turned pages; Arabic-Indic digits optional)*
- [x] Bookmarks and links stay correct after page changes *(pages keep their objects, so bookmarks and links follow them; bookmarks to deleted pages go to the nearest page left; links to deleted pages lead nowhere; page labels are dropped when pages move)*

Done when: a 1,000-page file can be reordered and saved without the interface freezing.

Notes:
- Design: the session keeps the page list (`crates/edits-edit/src/session/pages.rs`) and rewrites the page tree's top node in the same incremental update as annotations, so undo, crash recovery and the overlay shown on screen work as for annotations. Pages from other files are copied in with what they use (`pdf/copy.rs`). Saving after a deletion writes the whole file anew with only the objects still in use (`copy::rewrite`), so deleted pages' content is gone; a damaged file that cannot be rewritten is appended to instead. After a save that changed pages, undo history starts again.
- Done 2026-11-27: on the 1,000-page, 506 MB file the grid opens with a longest frame of 42 ms; moving pages 21 ms; saving 7 ms (the work runs off the interface thread). `tests/ui/organize-large.json`.
- `edits-bench pages-check`: on every editable corpus file (72), move, turn, copy, delete and insert, then check the result as shown and as saved (page count, sizes, text, every page renders). All pass; exporting pages of the 5 damaged fuzzed files is refused rather than done badly. 1,000 pages: 52 ms for the five steps, 0.8 s to write 506 MB anew.
- Benchmarks unchanged from M2 (`edits-bench app`: images-10mb and large-1000p-500mb pass everything but memory, as before).
- Owner check done 2026-11-27: the `edits-bench pages-demo` sample shows correctly in Acrobat Reader, Chrome and Edge. M3 is complete.

## M4: Forms and signing (release 0.4)

- [x] Fill text fields, checkboxes, radio buttons, dropdowns, lists; tab order *(fields shown tinted over the page; Tab and Shift+Tab follow the file's order, page by page; undo, recovery and save as for annotations)*
- [x] Arabic input in fields *(typed in the page; the field's appearance is drawn by edits with shaped Arabic in the bundled font; right-to-left text aligns right)*
- [x] Fill and sign on flat forms: type anywhere, tick and cross marks *(text tool; ✓ and ✗ tools; click, or drag for the size)*
- [x] Signature by drawing, typing or image; saved signatures *(a transparent PNG trimmed to the ink, placed as a picture stamp; up to 12 kept on this computer in signatures.json)*
- [x] Flatten forms and annotations *(every appearance drawn into its page; links kept; undoable until saved)*

Done when: a set of real government and bank forms from the corpus can be filled, signed, saved and reopened in Acrobat Reader with values intact.

Notes:
- Forms corpus (corpus/files/forms, downloaded blank public forms): IRS W-9, W-4, 1040, SS-4, 8822; USCIS I-9, AR-11, G-1145 (AR-11 and G-1145 are protected with AES-128 and permissions); US Treasury savings bond forms SAV 1048 and SAV 5336 (radio buttons). No Egyptian forms yet: to add when the owner has blank ones.
- Protected files: edits' reader and writers now handle the standard security handler (RC4, AES-128, AES-256; `pdf/crypt.rs`, crates aes, md-5 and sha2, already in the tree through lopdf). Files that open without a password (permissions only), and files opened with their password, can be annotated, organised and filled; updates and whole-file rewrites keep the same protection. This also does part of M10 (editing protected files); setting or removing a password is still M10.
- Field appearances: edits draws text, combo box and list appearances itself (one font subset shared by all fields in an update), honouring /DA size and colour, /Q, /MaxLen comb cells, multiline, /MK background and border, /MK /R rotation. Boxes without "on" appearances get a drawn tick or dot. XFA is removed from a form once a field changes. Field formatting and calculation scripts are not run.
- `edits-bench forms-check [DIR] [KEEP]`: fills every field of every corpus form (Arabic and Latin text, ticks, last radio button, first choice), saves a copy, reads it back (values must match), renders every page in PDFium, then flattens and checks again. All 11 forms pass; a second, independent reader agrees on the values. Filling 199 fields of the 1040 takes 0.8 s in all (about 4 ms each).
- UI: `tests/ui/forms.json` (W-9: type Arabic, Tab, tick, undo, redo, save), `tests/ui/sign.json` (tick, cross, drawn signature, flatten, save), `tests/ui/password.json` (now also annotates and saves a password-protected file).
- Benchmarks: no change from 0.3 when built and measured alternately on the same day (10 MB file: first page 530 ms vs 537–545 ms; zoom p95 56 ms vs 62–73 ms). Memory still over target as before.
- Owner check done 2026-11-27: the filled, protected, flattened and signed samples show correctly in Acrobat Reader. M4 is complete.
- Also in 0.4: the window draws its own title bar (tabs beside the window buttons, as in Windows Terminal) and the toolbars are flat.

## M5: OCR and PDFs from scans (release 0.5)

- [ ] `edits-ocr` crate: ONNX Runtime through `ort`, models loaded on first use
- [ ] Port from arabic-ocr: preprocess, detector post-processing, line cropping, recognizer and CTC decoding, layout and reading order, bidi, lexicon correction
- [ ] `charset` generated from arabic-ocr's Python source
- [ ] Parity tests against stored arabic-ocr output (same text, boxes within 2 px)
- [ ] CER on arabic-ocr's test sets matches its published numbers
- [ ] Invisible text layer written to the PDF (searchable PDF)
- [ ] Background OCR: visible pages first, progress, cancel, page range
- [ ] Offer OCR when a page has no text
- [ ] Options: diacritics, digit style
- [ ] Snip a region and copy its text
- [ ] Create PDF from images: order, page size, optional OCR
- [ ] Model attribution in About and licenses

Done when: a scanned Arabic book becomes searchable, selection and copy give correct logical-order text, and OCR never blocks scrolling.

## M6: Blank canvas editor (release 0.6)

- [ ] New document: page size, orientation, margins
- [ ] Place text boxes, images, shapes, lines as real page content (not annotations)
- [ ] Text box formatting: font, size, colour, alignment, direction
- [ ] Simple tables
- [ ] Alignment guides, snapping, grouping, z-order
- [ ] Same tools work on pages of existing PDFs (add content)

Done when: a one-page flyer and a simple invoice can be made from scratch in Arabic and English.

## M7: Edit existing content (release 0.7)

- [ ] Content model: parse page content into text runs, images and paths, with positions
- [ ] Group runs into lines and paragraphs
- [ ] Edit Latin text in place: insert, delete, restyle; reflow inside the paragraph
- [ ] Font handling: use the embedded or installed font, otherwise the nearest match with a notice
- [ ] Move, resize, replace, delete images and objects
- [ ] Edit links and bookmarks
- [ ] Edits do not damage the rest of the page (visual comparison tests on the corpus)

Done when: a typo can be fixed and a sentence rewritten in typical Word-exported and LaTeX PDFs, with the result visually clean.

## M8: Arabic text editing (release 0.8)

- [ ] Rebuild logical-order Arabic text from stored glyphs (presentation forms, ligatures, missing Unicode maps)
- [ ] Edit Arabic and mixed-direction paragraphs with correct shaping and cursor movement
- [ ] Font matching for common Arabic fonts
- [ ] Edit scanned text: OCR the line, edit, cover the original and write new text in a matching font and size

Done when: the M7 test passes for Arabic PDFs from Word and from common Arabic publishing tools, and a line in a scanned page can be corrected.

## M9: Document writer (release 0.9)

- [ ] Spike: Typst as layout engine or own layout (PRD 8.6); decision recorded
- [ ] Rich text editor: paragraphs, headings, bold, italic, underline, lists, alignment, direction
- [ ] Tables and images
- [ ] Page setup, page breaks, headers, footers, page numbers
- [ ] Live paginated preview
- [ ] Export to PDF with embedded fonts and bookmarks from headings
- [ ] Editable source file format; reopen and continue

Done when: a 20-page Arabic and English report with headings, tables and images can be written and exported.

## M10: Protect, clean, export, release (release 1.0)

- [ ] Encrypt with password and permissions (AES-256); remove a known password
- [ ] Redaction that removes the underlying content
- [ ] Remove metadata and hidden data; document properties editor
- [ ] Compress
- [ ] Export pages as PNG/JPEG and text as TXT
- [ ] Accessibility and keyboard pass over the whole interface
- [ ] User guide, website page, release notes
- [ ] Code signing decision carried out
- [ ] Full corpus, benchmark and manual test pass

Done when: every v1.0 feature in PRD section 5 works and every PRD section 7 target passes.

---

## After 1.0

See PRD section 6. First in line: macOS (Apple Silicon) and Linux builds.
