# Release notes

## 0.6.0 (2026-11-28)

Make documents from scratch, and a new look.

- New document (Ctrl N): choose the page size (A4, A5, A3, Letter, Legal or your own), portrait or landscape, margins and the number of pages.
- Add text boxes, pictures, rectangles, ellipses, lines, arrows and tables to any page, as real page content. They stay editable when you open the file again in edits.
- Text boxes: any font on your computer that may be embedded, or IBM Plex Sans Arabic; size, bold, italic, colour, alignment and direction. Arabic and English mix in one box.
- Tables: rows grow to fit their text, a heading row, right-to-left tables.
- Guides and snapping to the page edges, the middle, the margins and other items (hold Alt to turn it off); group (Ctrl G), bring forward and send back, line up, and nudge with the arrow keys.
- A new look: a ribbon with Home, Insert, Comment and View tabs (and Format when page items are chosen), a status bar with the page number, page layout and a zoom slider, a start page with large buttons and your recent files, and a new settings window.
- A new icon for edits, and PDF files that edits opens get their own icon.

## 0.5.0 (2026-11-28)

Text recognition for scanned pages.

- Recognise text: scanned pages in Arabic, Persian or Urdu become searchable, and their text can be selected and copied. The text is added to the pages, invisible, over the picture, and is saved with the file.
- It works in the background: the pages you are looking at are read first, you can keep reading, and Stop ends it at any time. Undo takes the text away until you save.
- Choose the pages, keep or drop diacritics, and choose how digits come out. Pages that already have text are left alone.
- When a page has no text, edits offers to recognise it.
- Copy text from an area: draw a box over part of a page and its text is copied.
- Make a PDF from pictures (JPEG and PNG): in order, on A4, Letter, A5 or pages the shape of each picture, with text recognised if you like.
- Copying voweled Arabic text keeps each diacritic on its own letter.

## 0.4.0 (2026-11-27)

Forms and signing.

- Fill in forms: text boxes, tick boxes, choice buttons, drop-down lists and lists, in Arabic or English. Tab moves to the next field.
- Forms without fields: type anywhere, and add ✓ and ✗ marks.
- Sign: draw your signature, type it, or use a picture (white paper is made transparent). Signatures are kept on this computer for next time.
- Flatten: make annotations and filled-in fields a fixed part of the page.
- A cleaner window: tabs sit in the title bar beside the window buttons, and the toolbars are flat.
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
