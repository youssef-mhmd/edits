# Release notes

## 0.4.0 (not yet released)

Forms and signing.

- Fill in forms: text boxes, tick boxes, choice buttons, drop-down lists and lists, in Arabic or English. Tab moves to the next field.
- Forms without fields: type anywhere, and add ✓ and ✗ marks.
- Sign: draw your signature, type it, or use a picture (white paper is made transparent). Signatures are kept on this computer for next time.
- Flatten: make annotations and filled-in fields a fixed part of the page.
- Protected PDFs (with a password, or only restricted, as many government forms are) can now be filled, annotated and organised; they stay protected when saved.

## 0.3.0 (2026-11-27)

Organise pages.

- A page grid ("Pages"): choose pages with click, Ctrl-click or Shift-click, and drag them to a new place.
- Turn, copy and delete pages. Copies keep their annotations and links.
- Insert blank pages, pictures, or pages from another PDF (or drop files on the grid).
- Crop pages by margins.
- Add page numbers, headers, footers and a watermark, in Arabic or English, with Arabic-Indic digits if you like.
- Extract pages to a new file; split a document every N pages, by page ranges or at its bookmarks; combine several PDFs into one.
- Everything can be undone. When pages are deleted, saving writes the file anew so their content is really gone.

## 0.2.0 (2026-11-27)

Annotate and mark up.

- Highlight, underline, strike through or squiggly-underline selected text, in five colours.
- Draw with a pen; add lines, arrows, rectangles and ellipses (colour, width, fill).
- Sticky notes, and text boxes in Arabic or English (letters joined and ordered correctly; double-click to edit).
- Stamps (معتمد, مسودة, سري, APPROVED, DRAFT, CONFIDENTIAL, RECEIVED) and pictures (JPEG, PNG).
- Select any annotation, including ones made in other apps: move, resize, recolour, add a note, delete. The Comments tab in the side panel lists them all.
- Undo and redo (Ctrl Z, Ctrl Y), as far back as you like, also after saving.
- Save (Ctrl S) adds your changes to the end of the file, leaving the original content as it was; Save As (Ctrl Shift S). edits asks before closing with unsaved changes, and after a crash offers to restore them.
- Not yet: annotating password-protected files.

## 0.1.0 (2026-11-27)

The first release: a fast PDF reader for Windows, in English and Arabic.

**Reading**
- Opens PDFs by double-click (edits appears in "Open with" and Default apps; it never takes over your default), drag and drop, Ctrl O or the command line. Several files open as tabs.
- Continuous, single-page and two-page views, with right-to-left spreads for Arabic books. Fit width, fit page, zoom, rotate.
- Thumbnails and bookmarks panel (F4), links, Back and Forward (Alt ← / Alt →), go to page (Ctrl G).
- Remembers recent files and where you were in each.
- Password-protected files. Damaged pages show a message instead of stopping the document.

**Text**
- Select and copy text, in the right order for Arabic and mixed text.
- Find (Ctrl F) with Arabic options: ignore diacritics and tatweel, treat letter variants as equal.

**Print**
- Ctrl P: all pages, the current page or a range, at standard (150 dpi) or high (300 dpi) quality, then the Windows print dialog.

**Look and language**
- Light and dark theme, or the same as Windows. English or Arabic interface; Arabic mirrors the layout.
- Settings: Ctrl ,

**Speed**
- Pages that are slow to draw (very detailed drawings and maps) are drawn by up to three helper processes in parallel, and a page that crashes or hangs while drawing no longer affects the rest of the app.

**Updates**
- edits checks for a new version a few seconds after it starts (can be turned off in Settings) and installs it when you choose.

**Known limits**
- Memory: with a document open and after scrolling and zooming, edits and its WebView2 processes use 240–570 MB more than with no document open (target: 200 MB). Most of it is the WebView's GPU process; the most is for large image-heavy books.
- Pages with tens of thousands of vector paths still take more than 100 ms to become sharp after a zoom.
- The installer is not code-signed yet, so Windows SmartScreen shows a warning the first time.
