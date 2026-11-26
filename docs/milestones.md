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
| M1 | Reader | M | 0.1 |
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

- [ ] Open by dialog, drag and drop, command line, and `.pdf` file association
- [ ] Continuous, single-page and two-page views; fit width, fit page, custom zoom; rotate view
- [ ] Two-level cache: low-resolution previews and sharp tiles, bounded by memory
- [ ] Thumbnails panel, bookmarks panel, go to page
- [ ] Text selection and copy, correct for right-to-left text
- [ ] Search with highlighted matches; Arabic normalization options
- [ ] Background text index
- [ ] Links, internal and web
- [ ] Tabs, recent files, restore last position
- [ ] Password-protected files
- [ ] Print
- [ ] Light and dark theme; settings
- [ ] English and Arabic interface with mirrored layout
- [ ] Per-page error display for damaged files; no crash on any corpus file
- [ ] Windows installer and auto-update

Done when: it can replace the current default PDF reader for daily use, and all PRD section 7 reader targets pass.

## M2: Annotate and mark up (release 0.2)

- [ ] Command history with undo and redo (used by every later milestone)
- [ ] Save, Save As, incremental save, safe write through a temporary file
- [ ] Autosave recovery file
- [ ] Highlight, underline, strikethrough, squiggly
- [ ] Freehand, line, arrow, rectangle, ellipse
- [ ] Sticky notes and text boxes, with Arabic shaping (`edits-text`: rustybuzz, bidi, font embedding and subsetting)
- [ ] Stamps and images
- [ ] Select, move, resize, restyle, delete; annotation list panel
- [ ] Annotations made in other apps display and can be edited

Done when: annotations saved by edits show correctly in Chrome, Edge and Acrobat Reader, and saving takes ≤ 200 ms.

## M3: Organize pages (release 0.3)

- [ ] Page grid view with drag reorder and multi-select
- [ ] Rotate, delete, duplicate, extract
- [ ] Insert from another PDF, blank pages, images
- [ ] Merge files; new PDF from pages of several files
- [ ] Split by range, every N pages, bookmarks
- [ ] Crop pages
- [ ] Page numbers, headers, footers, watermarks
- [ ] Bookmarks and links stay correct after page changes

Done when: a 1,000-page file can be reordered and saved without the interface freezing.

## M4: Forms and signing (release 0.4)

- [ ] Fill text fields, checkboxes, radio buttons, dropdowns, lists; tab order
- [ ] Arabic input in fields
- [ ] Fill and sign on flat forms: type anywhere, tick and cross marks
- [ ] Signature by drawing, typing or image; saved signatures
- [ ] Flatten forms and annotations

Done when: a set of real government and bank forms from the corpus can be filled, signed, saved and reopened in Acrobat Reader with values intact.

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
