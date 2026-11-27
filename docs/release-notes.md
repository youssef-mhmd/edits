# Release notes

## 0.2.0 (not yet released)

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
