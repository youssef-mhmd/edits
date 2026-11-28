<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as pickFile, save as pickSavePath } from "@tauri-apps/plugin-dialog";
  import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
  import {
    annotAdd,
    annotMove,
    annotRemove,
    annotResize,
    annotsAll,
    annotStyle,
    editRedo,
    editUndo,
    saveDoc,
    userName,
    recoveryOffer,
    recoveryRestore,
    recoveryDiscard,
    mergeFiles,
    imagesToPdf,
    pagesCrop,
    pagesDecorate,
    pagesDelete,
    pagesDuplicate,
    pagesExport,
    pagesInsertBlank,
    pagesInsertImages,
    pagesInsertPdf,
    pagesMove,
    pagesRotate,
    pagesSplit,
    pagesUndecorate,
    formFields,
    fieldSet,
    flattenDoc,
    fontFamilies,
    docMargins,
    newDocument,
    type ItemView,
    type TableData,
    type TextStyle,
    ocrAvailable,
    ocrCancel,
    ocrFirst,
    ocrRegion,
    ocrStart,
    type OcrOptions,
    type OcrProgress,
    loadSignatures,
    saveSignatures,
    type Changed,
    type Decor,
    type DocInfo,
    type AnnotStyle,
    type AnnotView,
    type MarkupKind,
    closeDoc,
    needsPassword,
    openDoc,
    outline as loadOutline,
    startup,
    transports,
    type OutlineItem,
    type SearchOptions,
    type Transport,
  } from "./api";
  import { Viewer, type FitMode, type Tool, type ViewMode } from "./viewer";
  import { benchNoFile, benchWithFile } from "./bench";
  import Sidebar from "./Sidebar.svelte";
  import PageGrid, { type InsertKind } from "./PageGrid.svelte";
  import CropDialog from "./CropDialog.svelte";
  import SplitDialog, { type Part } from "./SplitDialog.svelte";
  import MergeDialog from "./MergeDialog.svelte";
  import HeaderDialog from "./HeaderDialog.svelte";
  import OcrDialog from "./OcrDialog.svelte";
  import ItemBar from "./ItemBar.svelte";
  import TableDialog from "./TableDialog.svelte";
  import NewDocDialog from "./NewDocDialog.svelte";
  import PicturesDialog from "./PicturesDialog.svelte";
  import SignatureDialog from "./SignatureDialog.svelte";
  import Dialog from "./Dialog.svelte";
  import SearchBar from "./SearchBar.svelte";
  import PrintDialog from "./PrintDialog.svelte";
  import SettingsDialog from "./SettingsDialog.svelte";
  import { lang, t, type Key, type LangSetting } from "./i18n.svelte";
  import Icon from "./Icon.svelte";
  import { LOGO_SVG } from "./logo";
  import { applyLook, type Theme } from "./look";
  import { findUpdate, installUpdate, type Update } from "./update";
  import {
    flushRecent,
    forget,
    lastView,
    loadRecent,
    recentFiles,
    remember,
    settings,
    updateSettings,
    type RecentEntry,
  } from "./recent";

  interface Tab {
    id: number;
    path: string;
    name: string;
    viewer: Viewer | null;
    outline: OutlineItem[] | null;
    /** Unsaved changes, and whether undo and redo have anything to do. */
    dirty: boolean;
    canUndo: boolean;
    canRedo: boolean;
    /** Showing the page grid (organise pages) instead of the document. */
    organize: boolean;
    /** Text being recognised: the run and how far it is. */
    ocr: { run: number; done: number; total: number } | null;
    /** The offer to recognise text on a page without any was shown or answered. */
    ocrOffered: boolean;
  }

  const STAMP_WORDS = ["معتمد", "مسودة", "سري", "APPROVED", "DRAFT", "CONFIDENTIAL", "RECEIVED"];
  // Ribbon commands: tool, icon (icons.ts) and short label (or none).
  const TOOLS: [Tool, string, Key | null][] = [
    ["note", "note", "tool.note"],
    ["text", "textbox", "tool.text"],
    ["stamp", "stamp", "tool.stamp"],
    ["image", "picture", "tool.image"],
    ["tick", "tick", null],
    ["cross", "cross", null],
    ["ink", "pen", "tool.ink"],
    ["line", "line", null],
    ["arrow", "arrow", null],
    ["square", "rect", null],
    ["circle", "ellipse", null],
  ];
  /** Tools that add page content (not annotations). */
  const ITEM_TOOLS: [Tool, string, Key][] = [
    ["itext", "itext", "tool.text"],
    ["iimage", "picture", "tool.image"],
    ["itable", "table", "rib.table"],
    ["irect", "rect", "tool.square"],
    ["iellipse", "ellipse", "tool.circle"],
    ["iline", "line", "tool.line"],
    ["iarrow", "arrow", "tool.arrow"],
  ];
  const MODES: [ViewMode, string, Key][] = [
    ["continuous", "continuous", "bar.continuous"],
    ["single", "single", "bar.single"],
    ["two-page", "twoPage", "bar.twoPage"],
  ];
  type RibbonTab = "home" | "insert" | "comment" | "view" | "format";
  const RIBBON_TABS: RibbonTab[] = ["home", "insert", "comment", "view", "format"];

  const MARKUP_COLORS: { rgb: number[]; css: string }[] = [
    { rgb: [1, 0.92, 0.23], css: "#ffeb3b" },
    { rgb: [0.55, 0.9, 0.4], css: "#8ce666" },
    { rgb: [0.4, 0.75, 1], css: "#66bfff" },
    { rgb: [1, 0.55, 0.78], css: "#ff8cc7" },
    { rgb: [0.95, 0.3, 0.25], css: "#f24d40" },
  ];

  // Tabs hold class instances; keep the array itself reactive, not deep.
  let tabs: Tab[] = $state.raw([]);
  let activeId = $state(0);
  let nextId = 1;
  const hosts: Record<number, HTMLDivElement> = $state({});
  const active = $derived(tabs.find((t) => t.id === activeId) ?? null);
  const viewer = $derived(active?.viewer ?? null);

  // Toolbar state, copied from the active viewer on every update.
  let page = $state(0); // 0-based
  let pageInput = $state("1");
  let editingPage = $state(false);
  let zoomPct = $state(100);
  let fit: FitMode = $state("width");
  let mode: ViewMode = $state("continuous");
  let rtl = $state(false);
  let canBack = $state(false);
  let canForward = $state(false);

  let error = $state("");
  let recent: RecentEntry[] = $state.raw([]);
  let sidebarOpen = $state(false);
  let sidebarPanel: "pages" | "bookmarks" | "comments" = $state("pages");
  let findOpen = $state(false);
  let findBar: SearchBar | null = $state(null);
  let searchOptions: SearchOptions = $state(settings().searchOptions);
  let bench = false;
  let initialPath: string | null = $state(null);
  // Set while asking for a password; `path` null means the launch file.
  let pwPrompt: { path: string | null; wrong: boolean } | null = $state(null);
  let password = $state("");
  let pwField: HTMLInputElement | null = $state(null);
  let transport: Transport = $state.raw(transports.protocol);
  let printOpen = $state(false);
  let hasSel = $state(false);
  let markupColor: number[] = $state(MARKUP_COLORS[0].rgb);
  let author = $state("");
  let defaultAuthor = $state("");
  let busy = false; // a save is running
  let editQueue: Promise<void> = Promise.resolve(); // edits run one after another, in order
  let tool: Tool = $state("select");
  let drawColor: number[] = $state([0.9, 0.12, 0.1]);
  let drawWidth = $state(2);
  let drawFill = $state(false);
  let selAnnot: AnnotView | null = $state.raw(null);
  // Every annotation of the active document, for the Comments panel; null until asked for.
  let comments: AnnotView[] | null = $state.raw(null);
  /**
   * The text dialog: a note or text box being written. `a` is an existing
   * annotation; `create` one to add where the page was clicked.
   */
  let noteEdit: {
    a?: AnnotView;
    create?: { kind: "note" | "freetext"; page: number; rect: number[] };
    text: string;
  } | null = $state(null);
  let textSize = $state(12);
  // Set while offering to restore unsaved work kept after a crash.
  let restoreOffer: { tabId: number; name: string; when: number } | null = $state(null);
  let stampWord = $state(STAMP_WORDS[0]);
  // Set while asking whether to save a document before it closes.
  let unsavedPrompt: { tab: Tab; resolve: (r: "save" | "discard" | "cancel") => void } | null = $state(null);
  let settingsOpen = $state(false);
  const appWindow = getCurrentWindow();
  let maximized = $state(false);
  // Signatures kept on this computer (PNG data URLs) and the one being placed.
  let signatures: string[] = $state.raw([]);
  let signature: string | null = $state(null);
  let signMenu = $state(false);
  let signDialog = $state(false);
  let flattenAsk = $state(false);
  // Text recognition: installed, its dialog open, and the tab offered it.
  let ocrInstalled = $state(false);
  let ocrOpen = $state(false);
  let picturesOpen = $state(false);
  let newOpen = $state(false);
  // Page items: the chosen ones, installed fonts (read once), the table being edited.
  let selItems: ItemView[] = $state.raw([]);
  // The ribbon's chosen tab. Choosing page items shows their Format tab;
  // when nothing is chosen any more, the tab chosen before comes back.
  let ribbon: RibbonTab = $state("home");
  const itemTool = $derived(ITEM_TOOLS.some(([id]) => id === tool));
  let ribbonBefore: RibbonTab = "home";
  let ribbonSwitched = $state(false);
  $effect(() => {
    const chosen = selItems.length > 0;
    untrack(() => {
      if (chosen && ribbon !== "format") {
        ribbonBefore = ribbon;
        ribbon = "format";
      } else if (!chosen && ribbon === "format") ribbon = ribbonBefore;
    });
  });
  let fonts: [string, boolean][] = $state.raw([]);
  let fontsAsked = false;
  let tableEdit: { page: number; item: ItemView } | null = $state.raw(null);
  let itemText: TextStyle = $state({
    font: "",
    bold: false,
    italic: false,
    size: 12,
    color: [0, 0, 0],
    align: "start",
    rtl: null,
    line_height: 1.3,
  });

  function loadFonts() {
    if (fontsAsked) return;
    fontsAsked = true;
    void fontFamilies()
      .then((f) => (fonts = f))
      .catch(() => {});
  }
  let ocrOffer: number | null = $state(null);
  let ocrOptions: OcrOptions = $state(settings().ocr);
  let ocrSkipText = $state(settings().ocrSkipText);
  let lastShown = "";
  // Organise pages: open dialogs, with the pages they act on.
  let cropFor: number[] | null = $state(null);
  let headerFor: number[] | null = $state(null);
  let splitOpen = $state(false);
  let mergeFiles_: string[] | null = $state(null);
  /** A message with a file to show in its folder. */
  let notice: { text: string; path: string | null } | null = $state(null);
  let theme: Theme = $state("system");
  let language: LangSetting = $state("system");
  let checkUpdates = $state(true);
  let update: Update | null = $state.raw(null);
  let updatePct = $state(-1); // -1 until installing
  // Back and forward point the other way in a right-to-left interface.
  const rtlUi = $derived(lang() === "ar");

  const nameOf = (path: string) => path.split(/[\\/]/).pop() ?? path;
  const samePath = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
  // One spelling per file on Windows, so a file is never listed or opened twice.
  const onWindows = navigator.userAgent.includes("Windows");
  const normalize = (p: string) => (onWindows ? p.replace(/\//g, "\\") : p);

  /** Tabs are replaced, never mutated, so that everything derived from them updates. */
  function patchTab(id: number, patch: Partial<Tab>) {
    tabs = tabs.map((t) => (t.id === id ? { ...t, ...patch } : t));
  }

  function refresh(id: number) {
    const tab = tabs.find((t) => t.id === id);
    const v = tab?.viewer;
    if (!tab || !v || tab.id !== activeId) return;
    page = v.currentPage();
    if (!editingPage) pageInput = String(page + 1);
    fit = v.fit;
    mode = v.mode;
    rtl = v.rtl;
    canBack = v.canGoBack;
    canForward = v.canGoForward;
    // 100% is actual size: 96 CSS px per inch, 72 points per inch.
    zoomPct = Math.round(v.zoom * (72 / 96) * 100);
    if (!bench) remember(tab.path, tab.name, v.pageCount, v.viewState());
    ocrFollow(tab, v);
  }

  /**
   * While text is recognised, the pages on screen are read first. A page
   * shown without any text is offered recognition, once per document.
   */
  function ocrFollow(tab: Tab, v: Viewer) {
    const shown = v.shownPages();
    const key = `${tab.id}:${shown.join(",")}`;
    if (key === lastShown) return;
    lastShown = key;
    if (tab.ocr) {
      void ocrFirst(v.info.doc, shown).catch(() => {});
      return;
    }
    if (!ocrInstalled || tab.ocrOffered || bench) return;
    const p = shown[0];
    void v.loadText(p).then((txt) => {
      const now = tabs.find((x) => x.id === tab.id);
      if (!now || now.ocrOffered || now.ocr || now.viewer !== v || v.currentPage() !== p) return;
      if (!txt.text.trim()) {
        ocrOffer = tab.id;
        patchTab(tab.id, { ocrOffered: true });
      }
    });
  }

  /** Starts recognising text on `pages` of the active document. */
  async function startOcr(pages: number[], options: OcrOptions, skipText: boolean) {
    const tab = active;
    const v = tab?.viewer;
    if (!tab || !v) return;
    ocrOptions = options;
    ocrSkipText = skipText;
    updateSettings({ ocr: options, ocrSkipText: skipText });
    ocrOffer = null;
    const run = await ocrStart(v.info.doc, pages, v.shownPages(), options, skipText);
    patchTab(tab.id, { ocr: { run, done: 0, total: pages.length }, ocrOffered: true });
  }

  function stopOcr(tab: Tab) {
    if (tab.viewer) void ocrCancel(tab.viewer.info.doc).catch(() => {});
    patchTab(tab.id, { ocr: null });
  }

  function onOcrProgress(p: OcrProgress) {
    const tab = tabs.find((x) => x.viewer?.info.doc === p.doc);
    if (!tab || !tab.ocr || tab.ocr.run !== p.run) return;
    if (p.pages.length) tab.viewer?.forgetText(p.pages);
    if (p.status)
      patchTab(tab.id, { dirty: p.status.dirty, canUndo: p.status.can_undo, canRedo: p.status.can_redo });
    if (!p.finished) {
      patchTab(tab.id, { ocr: { run: p.run, done: p.done, total: p.total } });
      return;
    }
    patchTab(tab.id, { ocr: null });
    if (p.error) error = `${tab.name}: ${p.error}`;
    else {
      const n = p.done - p.skipped;
      notice = {
        text: p.skipped ? t("ocr.doneSkipped", { n, skipped: p.skipped }) : t("ocr.done", { n }),
        path: null,
      };
    }
  }

  /** The snip tool: reads the text in a box and copies it. */
  async function snip(v: Viewer, page: number, rect: number[]) {
    if (!ocrInstalled) {
      error = t("ocr.missing");
      return;
    }
    notice = { text: t("ocr.snipReading"), path: null };
    try {
      const text = (await ocrRegion(v.info.doc, page, rect, ocrOptions)).trim();
      if (!text) {
        notice = { text: t("ocr.snipEmpty"), path: null };
        return;
      }
      await copyText(text, v);
      const short = text.length > 80 ? `${text.slice(0, 80)}…` : text;
      notice = { text: t("ocr.snipCopied", { text: short }), path: null };
    } catch (e) {
      notice = null;
      error = String(e);
    }
  }

  /**
   * Shows tab `id`. Order matters: the old tab is deactivated while still
   * visible (it records its place), and the new one is activated only once
   * shown (a hidden element cannot be scrolled back to its place).
   */
  async function activate(id: number) {
    for (const t of tabs) if (t.id !== id) t.viewer?.setActive(false);
    activeId = id;
    await tick();
    const t = tabs.find((x) => x.id === id);
    t?.viewer?.setActive(true);
    if (t?.viewer) {
      (window as unknown as { __viewer: Viewer }).__viewer = t.viewer;
      hasSel = t.viewer.hasSelection;
      selAnnot = t.viewer.selectedAnnot;
      t.viewer.tool = tool;
      comments = null;
      if (sidebarOpen && sidebarPanel === "comments") void loadComments();
      refresh(t.id);
      t.viewer.scroller.focus();
    }
  }

  async function open(path: string | null, pw: string | null = null): Promise<Viewer | null> {
    error = "";
    path = path === null ? null : normalize(path);
    const realPath = path ?? (initialPath && normalize(initialPath));
    if (!realPath) return null;
    const existing = tabs.find((t) => samePath(t.path, realPath));
    if (existing && pw === null) {
      activate(existing.id);
      return existing.viewer;
    }
    try {
      const info = await openDoc(path, pw);
      pwPrompt = null;
      password = "";
      const id = nextId++;
      // Deactivate the visible tab while it is still visible, so it records
      // where the reader was before it is hidden.
      for (const t of tabs) t.viewer?.setActive(false);
      tabs = [
        ...tabs,
        {
          id,
          path: realPath,
          name: nameOf(realPath),
          viewer: null,
          outline: null,
          dirty: false,
          canUndo: false,
          canRedo: false,
          organize: false,
          ocr: null,
          ocrOffered: false,
        },
      ];
      activeId = id;
      await tick();
      const v = makeViewer(id, info);
      const saved = bench ? undefined : lastView(realPath);
      if (saved) v.restore(saved);
      patchTab(id, { viewer: v });
      activate(id);
      if (!bench)
        void recoveryOffer(info.doc)
          .then((when) => {
            if (when) restoreOffer = { tabId: id, name: nameOf(realPath), when };
          })
          .catch(() => {});
      void loadOutline(info.doc)
        .then((o) => patchTab(id, { outline: o }))
        .catch(() => patchTab(id, { outline: [] }));
      return v;
    } catch (e) {
      if (needsPassword(e)) {
        pwPrompt = { path, wrong: pw !== null };
        password = "";
        await tick();
        pwField?.focus();
      } else {
        error = `${nameOf(realPath)}: ${e}`;
      }
      return null;
    }
  }

  /** A viewer for tab `id` showing `info`, wired to the toolbar and the editor. */
  function makeViewer(id: number, info: DocInfo): Viewer {
    const v = new Viewer(hosts[id], info, transport);
    v.onUpdate = () => refresh(id);
    v.onExternalLink = (uri) => void openUrl(uri).catch((e) => (error = String(e)));
    v.onSelection = (has) => {
      if (id === activeId) hasSel = has;
    };
    v.onSnip = (page, rect) => void snip(v, page, rect);
    v.onItemsChanged = (c) => changed(id, c, false);
    v.onItemSelection = (items) => {
      if (id !== activeId) return;
      selItems = items;
      if (items.length) loadFonts();
    };
    v.pickPicture = async () => {
      const r = await pickFile({ filters: [{ name: "JPEG, PNG", extensions: ["jpg", "jpeg", "png"] }] });
      return typeof r === "string" ? r : null;
    };
    v.onEditTable = (page, item) => (tableEdit = { page, item });
    void docMargins(info.doc)
      .then((m) => (v.margins = m))
      .catch(() => {});
    v.onError = (e) => (error = String(e));
    v.itemDefaults = () => ({
      text: { ...itemText },
      rtl: lang() === "ar",
      stroke: drawColor,
      fill: drawFill ? drawColor : null,
      width: drawWidth,
    });
    v.onAnnotOpen = (a) => {
      if (a.kind === "note" || a.kind === "freetext") noteEdit = { a, text: a.contents };
    };
    v.onAnnotSelect = (a) => {
      if (id === activeId) selAnnot = a;
    };
    v.tool = tool;
    v.drawStyle = { width: drawWidth, fill: drawFill ? drawColor : null, color: drawColor };
    v.editor = {
      add: (n) =>
        runEdit(id, (doc) => annotAdd(doc, { ...n, color: drawColor, author: author || defaultAuthor })),
      move: (aid, dx, dy) => runEdit(id, (doc) => annotMove(doc, aid, dx, dy)),
      resize: (aid, rect) => runEdit(id, (doc) => annotResize(doc, aid, rect)),
      place: (tl, page, rect) => void place(id, tl, page, rect),
      fields: () => formFields(info.doc),
      setField: (fid, value) => runEdit(id, (doc) => fieldSet(doc, fid, value)),
    };
    return v;
  }

  /**
   * Shows tab `tabId`'s document anew after its pages changed: a new viewer
   * at the same place, and the bookmarks read again (their pages moved).
   */
  function replaceViewer(tabId: number, info: DocInfo) {
    const tab = tabs.find((t) => t.id === tabId);
    const old = tab?.viewer;
    if (!tab || !old) return;
    const state = old.viewState();
    const isActive = tabId === activeId;
    old.destroy();
    const v = makeViewer(tabId, info);
    v.restore(state);
    if (!isActive) v.setActive(false);
    patchTab(tabId, { viewer: v });
    if (isActive) {
      (window as unknown as { __viewer: Viewer }).__viewer = v;
      selAnnot = null;
      hasSel = false;
      refresh(tabId);
    }
    void loadOutline(info.doc)
      .then((o) => patchTab(tabId, { outline: o }))
      .catch(() => patchTab(tabId, { outline: [] }));
  }

  /** Asks whether to save `tab`'s changes. */
  function askUnsaved(tab: Tab): Promise<"save" | "discard" | "cancel"> {
    return new Promise((resolve) => (unsavedPrompt = { tab, resolve }));
  }

  function answerUnsaved(r: "save" | "discard" | "cancel") {
    const p = unsavedPrompt;
    unsavedPrompt = null;
    p?.resolve(r);
  }

  /** Lets `tab` close: saves or discards its changes as the reader chooses. False to keep it open. */
  async function releaseTab(tab: Tab): Promise<boolean> {
    if (!tab.dirty) return true;
    await activate(tab.id);
    const r = await askUnsaved(tab);
    if (r === "cancel") return false;
    if (r === "save") return save(tab, false);
    if (tab.viewer) await recoveryDiscard(tab.viewer.info.doc).catch(() => {});
    return true;
  }

  async function closeTab(id: number) {
    const tab = tabs.find((t) => t.id === id);
    if (!tab || !(await releaseTab(tab))) return;
    const i = tabs.findIndex((t) => t.id === id);
    if (i < 0) return;
    const t = tabs[i];
    if (t.viewer) {
      if (!bench) remember(t.path, t.name, t.viewer.pageCount, t.viewer.viewState());
      void closeDoc(t.viewer.info.doc);
      t.viewer.destroy();
    }
    tabs = tabs.filter((x) => x.id !== id);
    delete hosts[id];
    if (activeId === id) {
      const next = tabs[Math.min(i, tabs.length - 1)];
      if (next) activate(next.id);
      else {
        activeId = 0;
        recent = recentFiles();
      }
    }
  }

  /**
   * Takes in a change: undo state on the tab, pages drawn again, annotations
   * and page items read again (`items` false when the item layer made it).
   */
  function changed(tabId: number, c: Changed, items = true) {
    patchTab(tabId, { dirty: c.status.dirty, canUndo: c.status.can_undo, canRedo: c.status.can_redo });
    if (c.info) {
      replaceViewer(tabId, c.info);
      if (tabId === activeId && comments !== null) void loadComments();
      return;
    }
    const v = tabs.find((t) => t.id === tabId)?.viewer;
    v?.invalidatePages(c.pages);
    v?.reloadAnnots(c.pages);
    if (items) v?.items.reload(c.pages);
    if (tabId === activeId && comments !== null) void loadComments();
  }

  /**
   * Runs an edit of tab `tabId` after the ones before it, so they reach the
   * history in order.
   */
  function runEdit(tabId: number, fn: (doc: number) => Promise<Changed>): Promise<void> {
    const run = async () => {
      const tab = tabs.find((t) => t.id === tabId);
      const v = tab?.viewer;
      if (!tab || !v) return;
      try {
        const c = await fn(v.info.doc);
        changed(tabId, c);
      } catch (e) {
        error = String(e);
      }
    };
    editQueue = editQueue.then(run);
    return editQueue;
  }

  /** Edits the active tab (see `runEdit`). */
  function edit(fn: (tab: Tab, v: Viewer) => Promise<void>) {
    const tab = active;
    const v = tab?.viewer;
    if (!tab || !v || busy) return;
    editQueue = editQueue.then(() => fn(tab, v).catch((e) => void (error = String(e))));
  }

  function markup(kind: MarkupKind) {
    edit(async (tab, v) => {
      const parts = await v.selectionRects();
      for (const { page, rects } of parts) {
        changed(
          tab.id,
          await annotAdd(v.info.doc, {
            page,
            kind,
            rects,
            color: markupColor,
            opacity: 1,
            author: author || defaultAuthor,
          }),
        );
      }
      v.clearSelection();
    });
  }

  function undo() {
    edit(async (tab, v) => {
      changed(tab.id, await editUndo(v.info.doc));
      v.reloadFields();
    });
  }

  function redo() {
    edit(async (tab, v) => {
      changed(tab.id, await editRedo(v.info.doc));
      v.reloadFields();
    });
  }

  function setTool(t: Tool) {
    tool = t;
    if (viewer) viewer.tool = t;
    if (t === "itext" || t === "itable") loadFonts();
  }

  function syncDrawStyle() {
    if (viewer) viewer.drawStyle = { width: drawWidth, fill: drawFill ? drawColor : null, color: drawColor };
  }

  /** Restyles the selected annotation, or sets the style for drawing. */
  function restyle(style: AnnotStyle) {
    const a = selAnnot;
    if (a && viewer) {
      const id = active!.id;
      void runEdit(id, (doc) => annotStyle(doc, a.id, style));
    }
  }

  const same = (a: number[], b: number[]) =>
    a.map((v) => v.toFixed(2)).join() === b.map((v) => v.toFixed(2)).join();
  /** The colour the swatches show: the selected annotation's, the drawing tool's, or text markup's. */
  const currentColor = $derived.by(() =>
    selAnnot ? selAnnot.color : tool !== "select" ? drawColor : markupColor,
  );

  function pickColor(c: number[]) {
    const a = selAnnot;
    if (a) {
      restyle({ color: c, ...(a.fill ? { fill: c } : {}) });
      if (a.kind === "highlight" || a.kind === "underline" || a.kind === "strikeout" || a.kind === "squiggly")
        markupColor = c;
      else drawColor = c;
    } else if (tool !== "select") {
      drawColor = c;
    } else {
      markupColor = c;
      updateSettings({ markupColor: c });
    }
    syncDrawStyle();
  }

  function pickWidth(w: number) {
    drawWidth = w;
    syncDrawStyle();
    if (selAnnot && selAnnot.kind !== "other") restyle({ width: w });
  }

  function toggleFill() {
    drawFill = !drawFill;
    syncDrawStyle();
    if (selAnnot && (selAnnot.kind === "square" || selAnnot.kind === "circle"))
      restyle({ fill: selAnnot.fill ? null : selAnnot.color });
  }

  function deleteSelected() {
    const a = selAnnot;
    if (!a || !active) return;
    viewer?.selectAnnot(null);
    void runEdit(active.id, (doc) => annotRemove(doc, a.id));
  }

  /** A placing tool was used on tab `tabId`'s page `page`. */
  async function place(tabId: number, tl: Tool, page: number, rect: number[]) {
    const author_ = author || defaultAuthor;
    if (tl === "note") noteEdit = { create: { kind: "note", page, rect }, text: "" };
    else if (tl === "text") noteEdit = { create: { kind: "freetext", page, rect }, text: "" };
    else if (tl === "stamp")
      await runEdit(tabId, (doc) =>
        annotAdd(doc, {
          page,
          kind: "stamp",
          rect,
          label: stampWord,
          font_size: 20,
          color: drawColor,
          author: author_,
        }),
      );
    else if (tl === "tick" || tl === "cross") {
      // A mark 12 points across (or the box dragged), drawn as ink.
      const v = tabs.find((t) => t.id === tabId)?.viewer;
      const size = v?.info.pages[page];
      if (!size) return;
      let [x0, y0, x1, y1] = rect;
      if (Math.abs(x1 - x0) < 0.005) {
        const [hw, hh] = [6 / size.w, 6 / size.h];
        [x0, y0, x1, y1] = [x0 - hw, y0 - hh, x0 + hw, y0 + hh];
      }
      const [l, r] = [Math.min(x0, x1), Math.max(x0, x1)];
      const [tp, b] = [Math.min(y0, y1), Math.max(y0, y1)];
      const at = (fx: number, fy: number) => [l + (r - l) * fx, tp + (b - tp) * fy];
      const ink =
        tl === "tick"
          ? [[at(0.05, 0.55), at(0.38, 0.9), at(0.95, 0.1)]]
          : [
              [at(0.1, 0.1), at(0.9, 0.9)],
              [at(0.9, 0.1), at(0.1, 0.9)],
            ];
      await runEdit(tabId, (doc) =>
        annotAdd(doc, {
          page,
          kind: "ink",
          ink,
          width: Math.max(1, ((r - l) * size.w) / 8),
          color: [0, 0, 0],
          author: author_,
        }),
      );
    } else if (tl === "sign") {
      const png = signature;
      if (!png) {
        signDialog = true;
        return;
      }
      await runEdit(tabId, (doc) =>
        annotAdd(doc, { page, kind: "stamp", rect, image_data: png, color: [0, 0, 0], author: author_ }),
      );
    } else if (tl === "image") {
      const path = await pickFile({
        filters: [{ name: t("tool.image"), extensions: ["png", "jpg", "jpeg"] }],
      });
      if (typeof path !== "string") return;
      await runEdit(tabId, (doc) =>
        annotAdd(doc, { page, kind: "stamp", rect, image_path: path, color: drawColor, author: author_ }),
      );
    }
  }

  function pickTextSize(size: number) {
    textSize = size;
    const a = selAnnot;
    if (a && (a.kind === "freetext" || (a.kind === "stamp" && a.label))) restyle({ font_size: size });
  }

  function answerRestore(keep: boolean) {
    const o = restoreOffer;
    restoreOffer = null;
    const v = o && tabs.find((t) => t.id === o.tabId)?.viewer;
    if (!o || !v) return;
    if (keep) void runEdit(o.tabId, (doc) => recoveryRestore(doc)).then(() => v.reloadFields());
    else void recoveryDiscard(v.info.doc).catch(() => {});
  }

  // ------------------------------------------------------- organise pages

  function setOrganize(on: boolean, page?: number) {
    const tab = active;
    if (!tab?.viewer) return;
    if (!on && page !== undefined) tab.viewer.navigate(page);
    patchTab(tab.id, { organize: on });
    if (!on) void tick().then(() => viewer?.scroller.focus());
  }

  /** Runs a page change on the active tab; errors show and are passed on. */
  async function pageEdit(fn: (doc: number) => Promise<Changed>): Promise<void> {
    const tab = active;
    if (!tab?.viewer || busy) return;
    let failed: unknown = null;
    await runEdit(tab.id, async (doc) => {
      try {
        return await fn(doc);
      } catch (e) {
        failed = e;
        throw e;
      }
    });
    if (failed) throw failed;
  }

  const pdfFilter = [{ name: "PDF", extensions: ["pdf"] }];
  const baseName = (path: string) => nameOf(path).replace(/\.pdf$/i, "");

  async function insertPages(kind: InsertKind, at: number) {
    const v = viewer;
    if (!v) return;
    if (kind === "blank") {
      // The size of the page before (or after) it.
      const p = v.info.pages[Math.max(0, Math.min(at - 1, v.info.page_count - 1))];
      await pageEdit((doc) => pagesInsertBlank(doc, at, p.w, p.h, 1)).catch(() => {});
    } else if (kind === "images") {
      const r = await pickFile({
        multiple: true,
        filters: [{ name: t("tool.image"), extensions: ["png", "jpg", "jpeg"] }],
      });
      const paths = Array.isArray(r) ? r : typeof r === "string" ? [r] : [];
      if (paths.length) await pageEdit((doc) => pagesInsertImages(doc, at, paths)).catch(() => {});
    } else {
      const r = await pickFile({ multiple: true, filters: pdfFilter });
      const paths = Array.isArray(r) ? r : typeof r === "string" ? [r] : [];
      // Each file's pages after the previous file's.
      for (const p of paths.reverse()) await pageEdit((doc) => pagesInsertPdf(doc, at, p)).catch(() => {});
    }
  }

  async function extractPages(pages: number[]) {
    const tab = active;
    const v = viewer;
    if (!tab || !v || !pages.length) return;
    const sorted = [...pages].sort((a, b) => a - b);
    const label =
      sorted.length === 1 ? `${sorted[0] + 1}` : `${sorted[0] + 1}-${sorted[sorted.length - 1] + 1}`;
    const dir = tab.path.slice(0, tab.path.length - nameOf(tab.path).length);
    const target = await pickSavePath({
      defaultPath: `${dir}${baseName(tab.path)} (${t("org.pagesWord")} ${label}).pdf`,
      filters: pdfFilter,
    });
    if (!target) return;
    try {
      const out = await pagesExport(v.info.doc, sorted, normalize(target));
      notice = { text: t("org.extracted", { n: sorted.length, name: nameOf(out) }), path: out };
    } catch (e) {
      error = String(e);
    }
  }

  async function splitPages(parts: Part[]): Promise<string[] | null> {
    const tab = active;
    const v = viewer;
    if (!tab || !v) return null;
    const folder = await pickFile({ directory: true, title: t("split.folder") });
    if (typeof folder !== "string") return null;
    const written = await pagesSplit(v.info.doc, parts, normalize(folder));
    notice = { text: t("split.done", { n: written.length }), path: written[0] ?? null };
    return written;
  }

  async function cropPages(pages: number[], margins: number[] | null) {
    const v = viewer;
    if (!v) return;
    const crops = pages.map((page) => {
      if (!margins) return { page, margins: null };
      const p = v.info.pages[page];
      const [l, tp, r, b] = margins;
      return { page, margins: [l / p.w, tp / p.h, r / p.w, b / p.h] };
    });
    await pageEdit((doc) => pagesCrop(doc, crops));
  }

  async function combine(paths: string[]): Promise<boolean> {
    const first = paths[0];
    const dir = first.slice(0, first.length - nameOf(first).length);
    const target = await pickSavePath({
      defaultPath: `${dir}${t("merge.defaultName")}.pdf`,
      filters: pdfFilter,
    });
    if (!target) return false;
    const out = await mergeFiles(paths, normalize(target));
    await open(out);
    return true;
  }

  /** Makes a new document of empty pages and opens it. */
  async function makeNew(size: number[], pages: number, margins: number[]): Promise<boolean> {
    const target = await pickSavePath({ defaultPath: `${t("new.defaultName")}.pdf`, filters: pdfFilter });
    if (!target) return false;
    const out = await newDocument(normalize(target), size, pages, margins);
    const v = await open(out);
    if (v) setTool("itext");
    return true;
  }

  /** Makes a PDF of pictures, opens it, and recognises its text if asked. */
  async function makePictures(paths: string[], size: number[] | null, ocr: boolean): Promise<boolean> {
    const first = paths[0];
    const dir = first.slice(0, first.length - nameOf(first).length);
    const target = await pickSavePath({
      defaultPath: `${dir}${t("pics.defaultName")}.pdf`,
      filters: pdfFilter,
    });
    if (!target) return false;
    const out = await imagesToPdf(paths, normalize(target), size);
    const v = await open(out);
    if (ocr && v) {
      await tick();
      await startOcr([...Array(v.pageCount).keys()], ocrOptions, false);
    }
    return true;
  }

  // ------------------------------------------------------------- signing

  function chooseSignature(png: string) {
    signature = png;
    signMenu = false;
    setTool("sign");
  }

  function addSignature(png: string, keep: boolean) {
    if (keep) {
      signatures = [png, ...signatures.filter((s) => s !== png)].slice(0, 12);
      void saveSignatures(signatures).catch((e) => (error = String(e)));
    }
    chooseSignature(png);
  }

  function forgetSignature(png: string) {
    signatures = signatures.filter((s) => s !== png);
    if (signature === png) signature = null;
    void saveSignatures(signatures).catch((e) => (error = String(e)));
  }

  function flatten() {
    flattenAsk = false;
    const tab = active;
    if (tab) void runEdit(tab.id, (doc) => flattenDoc(doc));
  }

  function saveNote() {
    const n = noteEdit;
    noteEdit = null;
    const c = n?.create;
    if (n && c && active && n.text.trim()) {
      const tabId = active.id;
      void runEdit(tabId, (doc) =>
        annotAdd(doc, {
          page: c.page,
          kind: c.kind,
          rect: c.rect,
          contents: n.text,
          font_size: textSize,
          color: c.kind === "note" ? [1, 0.85, 0.2] : drawColor,
          author: author || defaultAuthor,
        }),
      );
      viewer?.scroller.focus();
      return;
    }
    if (!n?.a) return;
    const a = n.a;
    if (active) void runEdit(active.id, (doc) => annotStyle(doc, a.id, { contents: n.text }));
    viewer?.scroller.focus();
  }

  /** Reads every annotation of the active document for the Comments panel. */
  async function loadComments() {
    const v = viewer;
    if (!v) return;
    try {
      const all = await annotsAll(v.info.doc);
      if (v === viewer) comments = all;
    } catch (e) {
      comments = [];
      error = String(e);
    }
  }

  /** Goes to an annotation from the Comments panel and selects it. */
  function showComment(a: AnnotView) {
    const v = viewer;
    if (!v) return;
    setTool("select");
    v.navigate(a.page, Math.max(0, a.rect[1] - 0.05));
    v.selectAnnot(a);
  }

  /** Saves `tab`, to a new file with `as`. Returns true when saved. */
  async function save(tab: Tab, as: boolean): Promise<boolean> {
    const v = tab.viewer;
    if (!v) return false;
    let path: string | null = null;
    if (as) {
      path = await pickSavePath({ defaultPath: tab.path, filters: [{ name: "PDF", extensions: ["pdf"] }] });
      if (!path) return false;
      path = normalize(path);
    }
    try {
      const r = await saveDoc(v.info.doc, path);
      patchTab(tab.id, {
        path: r.path,
        name: nameOf(r.path),
        dirty: r.status.dirty,
        canUndo: r.status.can_undo,
        canRedo: r.status.can_redo,
      });
      return true;
    } catch (e) {
      error = t("save.failed", { error: String(e) });
      return false;
    }
  }

  function saveActive(as: boolean) {
    const tab = active;
    if (tab && !busy) {
      busy = true;
      // After the edits already asked for.
      editQueue = editQueue.then(() => save(tab, as).then(() => {})).finally(() => (busy = false));
    }
  }

  function cycleTab(dir: number) {
    if (tabs.length < 2) return;
    const i = tabs.findIndex((t) => t.id === activeId);
    activate(tabs[(i + dir + tabs.length) % tabs.length].id);
  }

  async function openDialog() {
    const picked = await pickFile({ multiple: true, filters: [{ name: "PDF", extensions: ["pdf"] }] });
    const paths = Array.isArray(picked) ? picked : typeof picked === "string" ? [picked] : [];
    for (const p of paths) await open(p);
  }

  function submitPassword(e: SubmitEvent) {
    e.preventDefault();
    if (pwPrompt) void open(pwPrompt.path, password);
  }

  // The status bar's zoom slider runs from 20% to 500%, with 100% in the middle.
  function greeting() {
    const h = new Date().getHours();
    return t(h < 12 ? "rib.morning" : h < 18 ? "rib.afternoon" : "rib.evening");
  }

  function zoomToSlider(pct: number) {
    return Math.min(100, Math.max(0, 50 + (50 * Math.log(pct / 100)) / Math.log(5)));
  }
  function sliderToZoom(v: number) {
    return 100 * Math.pow(5, (v - 50) / 50);
  }
  function setZoomPct(pct: number) {
    viewer?.setZoom(pct / 100 / (72 / 96));
  }

  function zoomBy(k: number) {
    viewer?.setZoom(viewer.zoom * k);
  }

  function goToTyped() {
    const n = parseInt(pageInput, 10);
    if (viewer && Number.isFinite(n)) viewer.navigate(n - 1);
    editingPage = false;
    pageInput = String(page + 1);
    viewer?.scroller.focus();
  }

  function toggleSidebar() {
    sidebarOpen = !sidebarOpen;
    updateSettings({ sidebarOpen });
  }

  function setPanel(p: "pages" | "bookmarks" | "comments") {
    sidebarPanel = p;
    if (p === "comments") void loadComments();
    else updateSettings({ sidebarPanel: p });
  }

  function scrollBy(dy: number) {
    viewer?.scroller.scrollBy({ top: dy });
  }

  /** The action for a key press, or null to let the browser handle it. */
  async function openFind() {
    findOpen = true;
    await tick();
    findBar?.focus();
  }

  async function copySelection(v: Viewer) {
    const text = await v.selectedText();
    if (text) await copyText(text, v);
  }

  async function copyText(text: string, v: Viewer) {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // Fallback for when the async clipboard is refused.
      const ta = document.createElement("textarea");
      ta.value = text;
      document.body.append(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
      v.scroller.focus();
    }
  }

  function keyAction(e: KeyboardEvent, v: Viewer): (() => void) | null {
    const pageWise = v.mode !== "continuous";
    const turn = (forward: boolean) => () => (forward ? v.nextPage() : v.prevPage());
    const screen = (dir: number) => () => scrollBy(dir * v.scroller.clientHeight * 0.9);
    if (!e.altKey && !v.hasSelection && v.items.key(e, v.currentPage())) return () => {};
    if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === "c" && v.hasSelection)
      return () => void copySelection(v);
    if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === "z") return () => undo();
    if (e.ctrlKey && (e.key.toLowerCase() === "y" || (e.shiftKey && e.key.toLowerCase() === "z")))
      return () => redo();
    if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === "a")
      return () => void v.selectPage(v.currentPage());
    if (e.key === "Escape" && v.hasSelection) return () => v.clearSelection();
    if ((e.key === "Delete" || e.key === "Backspace") && v.selectedAnnot) return () => deleteSelected();
    if (e.key === "Escape" && v.selectedAnnot) return () => v.selectAnnot(null);
    if (e.key === "Escape" && tool !== "select") return () => setTool("select");
    if (e.altKey && e.key === "ArrowLeft") return () => v.goBack();
    if (e.altKey && e.key === "ArrowRight") return () => v.goForward();
    if (e.ctrlKey && e.shiftKey && e.code === "Equal") return () => v.rotate(1);
    if (e.ctrlKey && e.shiftKey && e.code === "Minus") return () => v.rotate(-1);
    if (e.ctrlKey) {
      switch (e.key.toLowerCase()) {
        case "=":
        case "+":
          return () => zoomBy(1.25);
        case "-":
          return () => zoomBy(0.8);
        case "0":
          return () => v.fitWidth();
        case "1":
          return () => v.setZoom(96 / 72);
        case "2":
          return () => v.fitPage();
        case "g":
          return () => {
            editingPage = true;
            document.getElementById("page-input")?.focus();
          };
      }
      return null;
    }
    switch (e.key) {
      case "Home":
        return () => v.goToPage(0);
      case "End":
        return () => v.goToPage(v.pageCount - 1);
      case "PageDown":
        return pageWise ? turn(true) : screen(1);
      case "PageUp":
        return pageWise ? turn(false) : screen(-1);
      case " ":
        return pageWise ? turn(!e.shiftKey) : screen(e.shiftKey ? -1 : 1);
      case "ArrowRight":
      case "ArrowLeft": {
        // In page views without sideways scrolling, arrows turn pages, following
        // the reading direction in right-to-left two-page view.
        if (!pageWise || v.scroller.scrollWidth > v.scroller.clientWidth) return null;
        return turn((e.key === "ArrowRight") !== (v.mode === "two-page" && v.rtl));
      }
    }
    return null;
  }

  /** Keys that work with or without a document. */
  function appKeyAction(e: KeyboardEvent): (() => void) | null {
    const k = e.key.toLowerCase();
    if (e.ctrlKey && k === "o") return () => void openDialog();
    if (e.ctrlKey && k === "n") return () => (newOpen = true);
    if (e.ctrlKey && k === "w") return () => closeTab(activeId);
    if (e.ctrlKey && e.key === "Tab") return () => cycleTab(e.shiftKey ? -1 : 1);
    if (e.ctrlKey && e.key === "PageDown") return () => cycleTab(1);
    if (e.ctrlKey && e.key === "PageUp") return () => cycleTab(-1);
    if (e.key === "F4") return () => toggleSidebar();
    if (viewer && e.ctrlKey && k === "f") return () => void openFind();
    if (viewer && e.ctrlKey && k === "p") return () => (printOpen = true);
    if (viewer && e.ctrlKey && k === "s") return () => saveActive(e.shiftKey);
    if (e.ctrlKey && e.key === ",") return () => (settingsOpen = true);
    if (viewer && e.key === "F3") {
      return () => (findOpen && findBar ? findBar.step(e.shiftKey ? -1 : 1) : void openFind());
    }
    return null;
  }

  function onKey(e: KeyboardEvent) {
    // Dialogs handle their own keys.
    if (printOpen || settingsOpen || pwPrompt || unsavedPrompt || noteEdit || restoreOffer) return;
    if (
      cropFor ||
      headerFor ||
      splitOpen ||
      mergeFiles_ ||
      signDialog ||
      flattenAsk ||
      ocrOpen ||
      picturesOpen ||
      newOpen ||
      tableEdit
    )
      return;
    const typing =
      e.target instanceof HTMLInputElement ||
      e.target instanceof HTMLSelectElement ||
      e.target instanceof HTMLTextAreaElement;
    // The page grid handles its own keys; only undo and redo pass through.
    const organizing = !!active?.organize;
    const undoRedo = (k: KeyboardEvent): (() => void) | null => {
      if (k.ctrlKey && !k.shiftKey && k.key.toLowerCase() === "z") return () => undo();
      if (k.ctrlKey && (k.key.toLowerCase() === "y" || (k.shiftKey && k.key.toLowerCase() === "z")))
        return () => redo();
      return null;
    };
    const action =
      appKeyAction(e) ?? (viewer && !typing ? (organizing ? undoRedo(e) : keyAction(e, viewer)) : null);
    if (action) {
      e.preventDefault();
      action();
    }
  }

  function onWheel(e: WheelEvent) {
    if (!viewer || !e.ctrlKey) return;
    e.preventDefault();
    const r = viewer.scroller.getBoundingClientRect();
    viewer.setZoom(viewer.zoom * Math.exp(-e.deltaY / 500), e.clientX - r.left, e.clientY - r.top);
  }

  function describeRecent(r: RecentEntry) {
    const at =
      typeof r.page === "number"
        ? t("app.recentPage", { page: r.page + 1, pages: r.pages })
        : t("app.recentPages", { pages: r.pages });
    return `${at} · ${new Date(r.opened).toLocaleDateString(lang())}`;
  }

  function changeSettings(s: { theme: Theme; language: LangSetting; checkUpdates: boolean }) {
    theme = s.theme;
    language = s.language;
    checkUpdates = s.checkUpdates;
    applyLook(theme, language);
    updateSettings(s);
  }

  async function checkForUpdate(): Promise<string | null> {
    const u = await findUpdate();
    if (u) update = u;
    return u?.version ?? null;
  }

  /** How often a running edits looks for an update again. */
  const UPDATE_EVERY_MS = 4 * 60 * 60 * 1000;
  let updateTimer = 0;

  /** Looks for an update now and then every few hours, while the setting is on. */
  function watchUpdates() {
    const look = () => {
      // Nothing to do while one is shown or being installed; never in tests.
      if (!checkUpdates || update || navigator.webdriver) return;
      void checkForUpdate().catch(() => {});
    };
    setTimeout(look, 5000);
    clearInterval(updateTimer);
    updateTimer = window.setInterval(look, UPDATE_EVERY_MS);
  }

  async function install() {
    if (!update) return;
    updatePct = 0;
    try {
      await installUpdate(update, (pct) => (updatePct = pct));
    } catch (e) {
      updatePct = -1;
      error = t("up.failed", { error: String(e) });
    }
  }

  onMount(() => {
    // Measured as "usable window" in bench mode: first frame after mount.
    let windowReady = 0;
    requestAnimationFrame(() => (windowReady = performance.timeOrigin + performance.now()));

    void (async () => {
      const s = await startup();
      void loadSignatures()
        .then((l) => (signatures = Array.isArray(l) ? l.filter((x) => typeof x === "string") : []))
        .catch(() => {});
      void userName()
        .then((n) => (defaultAuthor = n))
        .catch(() => {});
      bench = s.bench;
      transport = transports[s.transport];
      initialPath = s.file;
      if (!bench) {
        await loadRecent();
        recent = recentFiles();
        sidebarOpen = settings().sidebarOpen;
        sidebarPanel = settings().sidebarPanel;
        searchOptions = settings().searchOptions;
        theme = settings().theme;
        language = settings().language;
        checkUpdates = settings().checkUpdates;
        author = settings().author;
        if (Array.isArray(settings().markupColor)) markupColor = settings().markupColor;
        ocrOptions = settings().ocr;
        ocrSkipText = settings().ocrSkipText;
        applyLook(theme, language);
        // Look for an update once start-up is well over, then every few hours.
        watchUpdates();
      }
      const v = s.file ? await open(null) : null;
      if (s.bench) {
        while (!windowReady) await new Promise((r) => requestAnimationFrame(r));
        if (v) await benchWithFile(s, windowReady, v, transport);
        else await benchNoFile(s, windowReady);
      }
    })();

    // The maximise button shows restore while maximised.
    const syncMax = () =>
      void appWindow
        .isMaximized()
        .then((m) => (maximized = m))
        .catch(() => {});
    syncMax();
    const unlistenResize = appWindow.onResized(syncMax);
    const unlistenDrop = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "drop") {
        const paths = e.payload.paths;
        if (active?.organize && viewer) {
          // Into the document being organised, at the end.
          const at = viewer.info.page_count;
          const pdfs = paths.filter((p) => p.toLowerCase().endsWith(".pdf"));
          const pics = paths.filter((p) => /\.(png|jpe?g)$/i.test(p));
          void (async () => {
            if (pics.length) await pageEdit((doc) => pagesInsertImages(doc, at, pics)).catch(() => {});
            for (const p of pdfs.reverse())
              await pageEdit((doc) => pagesInsertPdf(doc, at, p)).catch(() => {});
          })();
          return;
        }
        for (const p of paths.filter((p) => p.toLowerCase().endsWith(".pdf"))) void open(p);
      }
    });
    // Files from a second launch (double-click while running).
    const unlistenFiles = listen<string[]>("open-files", (e) => {
      for (const p of e.payload) void open(p);
    });
    const unlistenOcr = listen<OcrProgress>("ocr-progress", (e) => onOcrProgress(e.payload));
    void ocrAvailable()
      .then((a) => (ocrInstalled = a))
      .catch(() => {});
    // Save where the reader is, then let the window close. This handler must
    // never stop the close: it cannot throw, and it waits at most half a
    // second (positions are also saved continuously while reading).
    const unlistenClose = getCurrentWindow().onCloseRequested(async (event) => {
      if (bench) return;
      // Unsaved changes: keep the window, ask, then close it ourselves.
      if (tabs.some((t) => t.dirty)) {
        event.preventDefault();
        void (async () => {
          for (const tab of tabs.filter((t) => t.dirty)) if (!(await releaseTab(tab))) return;
          for (const tab of tabs) patchTab(tab.id, { dirty: false });
          await getCurrentWindow().close();
        })().catch((e) => (error = String(e)));
        return;
      }
      try {
        for (const t of tabs)
          if (t.viewer) remember(t.path, t.name, t.viewer.pageCount, t.viewer.viewState());
        await Promise.race([flushRecent(), new Promise((r) => setTimeout(r, 500))]);
      } catch {
        // Closing matters more than the last position.
      }
    });
    return () => {
      void unlistenDrop.then((f) => f());
      void unlistenResize.then((f) => f());
      void unlistenFiles.then((f) => f());
      void unlistenOcr.then((f) => f());
      clearInterval(updateTimer);
      void unlistenClose.then((f) => f());
    };
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="app">
  <header>
    <!-- The window's own title bar: quick commands, tabs, space to drag the window, its buttons. -->
    <div class="titlebar" data-tauri-drag-region>
      <span class="brand" data-tauri-drag-region>
        <!-- eslint-disable-next-line svelte/no-at-html-tags -- generated by scripts/gen-brand.py -->
        <span class="logo" aria-hidden="true">{@html LOGO_SVG}</span>
        {#if !tabs.length}<span class="app-name" data-tauri-drag-region>edits</span>{/if}
      </span>
      {#if viewer}
        <div class="qat" role="toolbar" aria-label={t("rib.quick")}>
          <button
            onclick={() => saveActive(false)}
            disabled={!active?.dirty}
            title={t("save.saveTitle")}
            aria-label={t("save.save")}
          >
            <Icon name="save" size={18} />
          </button>
          <button
            disabled={!active?.canUndo}
            onclick={undo}
            title={t("edit.undoTitle")}
            aria-label={t("edit.undo")}
          >
            <Icon name="undo" size={18} />
          </button>
          <button
            disabled={!active?.canRedo}
            onclick={redo}
            title={t("edit.redoTitle")}
            aria-label={t("edit.redo")}
          >
            <Icon name="redo" size={18} />
          </button>
        </div>
      {/if}
      <div class="tabbar" role="tablist" aria-label={t("app.documents")} data-tauri-drag-region>
        {#each tabs as tab (tab.id)}
          <div
            class="tab"
            class:active={tab.id === activeId}
            role="tab"
            tabindex="0"
            aria-selected={tab.id === activeId}
            title={tab.path}
            onclick={() => activate(tab.id)}
            onkeydown={(e) => e.key === "Enter" && activate(tab.id)}
            onauxclick={(e) => e.button === 1 && closeTab(tab.id)}
          >
            <span class="tab-icon" aria-hidden="true">PDF</span>
            <span class="tab-name" dir="auto">{tab.name}</span>
            {#if tab.dirty}<span class="dirty" title={t("unsaved.marker")}>●</span>{/if}
            <button
              class="tab-close"
              aria-label={t("app.closeTab", { name: tab.name })}
              title={t("app.closeTabTitle")}
              onclick={(e) => {
                e.stopPropagation();
                closeTab(tab.id);
              }}
            >
              <Icon name="close" size={14} />
            </button>
          </div>
        {/each}
        {#if tabs.length}
          <button
            class="tab-new"
            onclick={openDialog}
            title={t("app.openTitle")}
            aria-label={t("app.openPdf")}
          >
            <Icon name="plus" size={16} />
          </button>
        {/if}
      </div>
      <span class="drag" data-tauri-drag-region></span>
      <button
        class="title-btn"
        onclick={() => (settingsOpen = true)}
        title={t("bar.settingsTitle")}
        aria-label={t("bar.settings")}
      >
        <Icon name="settings" size={18} />
      </button>
      <div class="winctl">
        <button
          tabindex="-1"
          aria-label={t("win.minimize")}
          title={t("win.minimize")}
          onclick={() => void appWindow.minimize()}
        >
          &#xE921;
        </button>
        <button
          tabindex="-1"
          aria-label={maximized ? t("win.restore") : t("win.maximize")}
          title={maximized ? t("win.restore") : t("win.maximize")}
          onclick={() => void appWindow.toggleMaximize()}
        >
          {String.fromCharCode(maximized ? 0xe923 : 0xe922)}
        </button>
        <button
          class="close"
          tabindex="-1"
          aria-label={t("win.close")}
          title={t("win.close")}
          onclick={() => void appWindow.close()}
        >
          &#xE8BB;
        </button>
      </div>
    </div>
    {#if viewer}
      <!-- The ribbon: its tabs, then the chosen tab's commands on a card. -->
      <div class="ribbon">
        <div class="rtabs" role="tablist" aria-label={t("rib.tabs")}>
          {#each RIBBON_TABS as id (id)}
            {#if id !== "format" || selItems.length}
              <button
                class="rtab"
                class:on={ribbon === id}
                class:contextual={id === "format"}
                role="tab"
                aria-selected={ribbon === id}
                onclick={() => {
                  ribbonSwitched = true;
                  ribbon = id;
                }}
              >
                {t(`rib.${id}` as "rib.home")}
              </button>
            {/if}
          {/each}
        </div>
        <div class="ctxbar" class:switched={ribbonSwitched} role="toolbar" aria-label={t("ctx.label")}>
          {#if active?.organize}
            <button
              class="big active"
              aria-pressed="true"
              onclick={() => setOrganize(false, page)}
              title={t("org.openTitle")}
            >
              <Icon name="pages" />{t("org.open")}
            </button>
            <span class="sep"></span>
            <button class="big" onclick={() => (printOpen = true)} title={t("bar.printTitle")}>
              <Icon name="print" />{t("bar.print")}
            </button>
          {:else}
            <!-- Home -->
            <div class="panel" hidden={ribbon !== "home"}>
              <button class="big" onclick={() => setOrganize(true)} title={t("org.openTitle")}>
                <Icon name="pages" />{t("org.open")}
              </button>
              <button
                class="big"
                class:active={sidebarOpen}
                aria-pressed={sidebarOpen}
                onclick={toggleSidebar}
                title={t("bar.panelTitle")}
              >
                <Icon name="panel" />{t("bar.panel")}
              </button>
              <span class="sep"></span>
              <button
                class="big"
                class:active={tool === "select"}
                aria-pressed={tool === "select"}
                title={t("tool.select")}
                aria-label={t("tool.select")}
                onclick={() => setTool("select")}
              >
                <Icon name="select" /><span class="lbl">{t("rib.select")}</span>
              </button>
              <button
                class="big"
                class:active={tool === "snip"}
                aria-pressed={tool === "snip"}
                title={t("tool.snip")}
                aria-label={t("tool.snip")}
                onclick={() => setTool("snip")}
              >
                <Icon name="snip" /><span class="lbl">{t("rib.snip")}</span>
              </button>
              <button class="big" onclick={() => void openFind()} title={t("rib.findTitle")}>
                <Icon name="find" />{t("rib.find")}
              </button>
              <span class="sep"></span>
              <span class="marks" role="group" aria-label={t("mark.color")}>
                {#each MARKUP_COLORS.concat([{ rgb: [0, 0, 0], css: "#000" }]) as c (c.css)}
                  <button
                    class="swatch"
                    class:active={same(currentColor, c.rgb)}
                    aria-pressed={same(currentColor, c.rgb)}
                    style:background={c.css}
                    aria-label={c.css}
                    title={t("mark.color")}
                    onclick={() => pickColor(c.rgb)}
                  ></button>
                {/each}
              </span>
              <button
                disabled={!hasSel}
                onclick={() => markup("highlight")}
                title={hasSel ? t("mark.highlight") : t("mark.hint")}
                aria-label={t("mark.highlight")}
              >
                <span
                  class="mk hl"
                  style:background={`rgb(${markupColor.map((c) => Math.round(c * 255)).join(" ")})`}
                >
                  A
                </span>
              </button>
              <button
                disabled={!hasSel}
                onclick={() => markup("underline")}
                title={hasSel ? t("mark.underline") : t("mark.hint")}
                aria-label={t("mark.underline")}
              >
                <span class="mk ul">A</span>
              </button>
              <button
                disabled={!hasSel}
                onclick={() => markup("strikeout")}
                title={hasSel ? t("mark.strikeout") : t("mark.hint")}
                aria-label={t("mark.strikeout")}
              >
                <span class="mk so">A</span>
              </button>
              <button
                disabled={!hasSel}
                onclick={() => markup("squiggly")}
                title={hasSel ? t("mark.squiggly") : t("mark.hint")}
                aria-label={t("mark.squiggly")}
              >
                <span class="mk sq">A</span>
              </button>
              <span class="sep"></span>
              {#if ocrInstalled}
                <button
                  class="big"
                  onclick={() => (ocrOpen = true)}
                  disabled={!!active?.ocr}
                  title={t("ocr.buttonTitle")}
                >
                  <Icon name="ocr" />{t("ocr.button")}
                </button>
              {/if}
              <button class="big" onclick={() => (printOpen = true)} title={t("bar.printTitle")}>
                <Icon name="print" />{t("bar.print")}
              </button>
              <button
                class="big"
                onclick={() => saveActive(false)}
                disabled={!active?.dirty}
                title={t("save.saveTitle")}
              >
                <Icon name="save" />{t("save.save")}
              </button>
            </div>
            <!-- Comment: annotations -->
            <div class="panel" hidden={ribbon !== "comment"}>
              {#each TOOLS as [id, icon, label] (id)}
                <button
                  class="big"
                  class:active={tool === id}
                  aria-pressed={tool === id}
                  title={t(`tool.${id}` as "tool.select")}
                  aria-label={t(`tool.${id}` as "tool.select")}
                  onclick={() => setTool(id as Tool)}
                >
                  <Icon name={icon} />{#if label}<span class="lbl">{t(label)}</span>{/if}
                </button>
                {#if id === "cross" || id === "circle"}<span class="sep"></span>{/if}
              {/each}
              <span class="menu sign">
                <button
                  class="big"
                  class:active={tool === "sign"}
                  aria-pressed={tool === "sign"}
                  aria-haspopup="menu"
                  aria-expanded={signMenu}
                  title={t("sig.menuTitle")}
                  onclick={() => (signatures.length ? (signMenu = !signMenu) : (signDialog = true))}
                >
                  <Icon name="sign" />{t("sig.sign")}
                </button>
                {#if signMenu}
                  <div class="menu-list" role="menu">
                    {#each signatures as png (png)}
                      <span class="sig-item">
                        <button role="menuitem" class="sig-pick" onclick={() => chooseSignature(png)}>
                          <img src={png} alt={t("sig.saved")} />
                        </button>
                        <button
                          class="sig-forget"
                          aria-label={t("sig.forget")}
                          title={t("sig.forget")}
                          onclick={() => forgetSignature(png)}
                        >
                          ×
                        </button>
                      </span>
                    {/each}
                    <button
                      role="menuitem"
                      onclick={() => {
                        signMenu = false;
                        signDialog = true;
                      }}
                    >
                      {t("sig.new")}
                    </button>
                  </div>
                {/if}
              </span>
              <span class="sep"></span>
              <button class="big" onclick={() => (flattenAsk = true)} title={t("flat.title")}>
                <Icon name="flatten" />{t("flat.button")}
              </button>
            </div>
            <!-- Insert: page content -->
            <div class="panel" hidden={ribbon !== "insert"}>
              {#each ITEM_TOOLS as [id, icon, label] (id)}
                <button
                  class="big"
                  class:active={tool === id}
                  aria-pressed={tool === id}
                  title={t(`tool.${id}` as "tool.select")}
                  aria-label={t(`tool.${id}` as "tool.select")}
                  onclick={() => setTool(id as Tool)}
                >
                  <Icon name={icon} /><span class="lbl">{t(label)}</span>
                </button>
                {#if id === "itable"}<span class="sep"></span>{/if}
              {/each}
            </div>
            <!-- View -->
            <div class="panel" hidden={ribbon !== "view"}>
              <button
                class="big"
                class:active={fit === "width"}
                aria-pressed={fit === "width"}
                onclick={() => viewer?.fitWidth()}
                title={t("bar.fitWidthTitle")}
              >
                <Icon name="fitWidth" />{t("bar.fitWidth")}
              </button>
              <button
                class="big"
                class:active={fit === "page"}
                aria-pressed={fit === "page"}
                onclick={() => viewer?.fitPage()}
                title={t("bar.fitPageTitle")}
              >
                <Icon name="fitPage" />{t("bar.fitPage")}
              </button>
              <span class="sep"></span>
              {#each MODES as [m, icon, label] (m)}
                <button
                  class="big"
                  class:active={mode === m}
                  aria-pressed={mode === m}
                  onclick={() => viewer?.setMode(m)}
                  title={t("bar.layout")}
                >
                  <Icon name={icon} />{t(label)}
                </button>
              {/each}
              {#if mode === "two-page"}
                <button
                  class="big"
                  class:active={rtl}
                  aria-pressed={rtl}
                  onclick={() => viewer?.setRtl(!rtl)}
                  title={t("bar.rtlTitle")}
                >
                  <Icon name="rtl" />{t("bar.rtl")}
                </button>
              {/if}
              <span class="sep"></span>
              <button
                class="big"
                onclick={() => viewer?.rotate(-1)}
                title={t("bar.rotateLeftTitle")}
                aria-label={t("bar.rotateLeft")}
              >
                <Icon name="rotateLeft" /><span class="lbl">{t("bar.rotateLeft")}</span>
              </button>
              <button
                class="big"
                onclick={() => viewer?.rotate(1)}
                title={t("bar.rotateRightTitle")}
                aria-label={t("bar.rotateRight")}
              >
                <Icon name="rotateRight" /><span class="lbl">{t("bar.rotateRight")}</span>
              </button>
              <span class="sep"></span>
              <button
                disabled={!canBack}
                onclick={() => viewer?.goBack()}
                title={t("bar.backTitle")}
                aria-label={t("bar.back")}
              >
                <Icon name={rtlUi ? "forward" : "back"} />
              </button>
              <button
                disabled={!canForward}
                onclick={() => viewer?.goForward()}
                title={t("bar.forwardTitle")}
                aria-label={t("bar.forward")}
              >
                <Icon name={rtlUi ? "back" : "forward"} />
              </button>
            </div>
            <!-- Format: the chosen page items -->
            {#if selItems.length && viewer}
              <div class="panel" hidden={ribbon !== "format"}>
                <ItemBar
                  items={selItems}
                  layer={viewer.items}
                  {fonts}
                  onstyle={(st) => (itemText = { ...itemText, ...st })}
                  onedittable={(it) => (tableEdit = { page: viewer?.currentPage() ?? 0, item: it })}
                />
              </div>
            {/if}
            {#if (tool !== "select" && !itemTool) || selAnnot}<span class="sep"></span>{/if}
            {#if selAnnot}
              <span class="ctx-kind">
                {t(`kind.${selAnnot.kind}` as "kind.other")}{#if selAnnot.author}<span
                    class="ctx-by"
                    dir="auto"
                  >
                    · {selAnnot.author}
                  </span>{/if}
              </span>
            {/if}
            {#if (!selAnnot && tool !== "select" && tool !== "snip" && !itemTool) || (selAnnot && ["ink", "line", "square", "circle"].includes(selAnnot.kind))}
              <label class="ctx-width">
                {t("ctx.width")}
                <select
                  value={String(selAnnot ? selAnnot.width : drawWidth)}
                  onchange={(e) => pickWidth(Number(e.currentTarget.value))}
                >
                  {#each [0.5, 1, 2, 3, 5, 8] as w (w)}<option value={String(w)}>{w} pt</option>{/each}
                </select>
              </label>
            {/if}
            {#if (!selAnnot && (tool === "square" || tool === "circle")) || (selAnnot && (selAnnot.kind === "square" || selAnnot.kind === "circle"))}
              <button
                class:active={selAnnot ? !!selAnnot.fill : drawFill}
                aria-pressed={selAnnot ? !!selAnnot.fill : drawFill}
                onclick={toggleFill}
              >
                {t("ctx.fill")}
              </button>
            {/if}
            {#if (!selAnnot && (tool === "text" || tool === "stamp")) || (selAnnot && (selAnnot.kind === "freetext" || (selAnnot.kind === "stamp" && selAnnot.label)))}
              <label class="ctx-width">
                {t("ctx.size")}
                <select
                  value={String(selAnnot ? selAnnot.font_size : textSize)}
                  onchange={(e) => pickTextSize(Number(e.currentTarget.value))}
                >
                  {#each [8, 10, 12, 14, 18, 24, 32, 48] as sz (sz)}<option value={String(sz)}>
                      {sz} pt
                    </option>{/each}
                </select>
              </label>
            {/if}
            {#if !selAnnot && tool === "stamp"}
              <label class="ctx-width">
                {t("ctx.stampWord")}
                <select bind:value={stampWord}>
                  {#each STAMP_WORDS as w (w)}<option value={w}>{w}</option>{/each}
                </select>
              </label>
            {/if}
            {#if !selAnnot && (tool === "note" || tool === "text" || tool === "stamp" || tool === "image" || tool === "tick" || tool === "cross" || tool === "sign")}
              <span class="ctx-hint">{t(`hint.${tool}` as "hint.note")}</span>
            {/if}
            {#if selAnnot}
              <button onclick={() => selAnnot && (noteEdit = { a: selAnnot, text: selAnnot.contents })}>
                {selAnnot.kind === "freetext" ? t("ctx.text") : t("ctx.note")}
              </button>
              <button onclick={deleteSelected} title={t("ctx.deleteTitle")}>{t("ctx.delete")}</button>
            {/if}
          {/if}
        </div>
      </div>
    {/if}
  </header>
  {#if update}
    <div class="update" role="status">
      {#if updatePct >= 0}
        <span>{t("up.downloading", { pct: updatePct })}</span>
      {:else}
        <span>{t("up.available", { version: update.version })}</span>
        <button class="primary" onclick={install}>{t("up.install")}</button>
        <button onclick={() => (update = null)}>{t("up.later")}</button>
      {/if}
    </div>
  {/if}
  {#if ocrOffer !== null && active && ocrOffer === active.id && !active.ocr}
    <div class="update" role="status">
      <span>{t("ocr.offer")}</span>
      <button
        class="primary"
        onclick={() => {
          ocrOffer = null;
          ocrOpen = true;
        }}
      >
        {t("ocr.offerButton")}
      </button>
      <button onclick={() => (ocrOffer = null)} aria-label={t("app.dismiss")}>×</button>
    </div>
  {/if}
  <div class="body">
    {#if viewer && active && sidebarOpen && !active.organize}
      {#key viewer}
        <Sidebar
          {viewer}
          {page}
          outline={active.outline}
          panel={sidebarPanel}
          onpanel={setPanel}
          {comments}
          selected={selAnnot?.id ?? null}
          oncomment={showComment}
        />
      {/key}
    {/if}
    <main onwheel={onWheel}>
      {#each tabs as tab (tab.id)}
        <div class="doc" hidden={tab.id !== activeId} bind:this={hosts[tab.id]}></div>
      {/each}
      {#if viewer && active?.organize}
        {#key active.id}
          <PageGrid
            {viewer}
            start={page}
            onmove={(pages, before) => pageEdit((doc) => pagesMove(doc, pages, before)).catch(() => {})}
            onrotate={(pages, q) => pageEdit((doc) => pagesRotate(doc, pages, q)).catch(() => {})}
            ondelete={(pages) => pageEdit((doc) => pagesDelete(doc, pages)).catch(() => {})}
            onduplicate={(pages) => pageEdit((doc) => pagesDuplicate(doc, pages)).catch(() => {})}
            oninsert={insertPages}
            onextract={(pages) => void extractPages(pages)}
            oncrop={(pages) => (cropFor = pages)}
            onheader={(pages) => (headerFor = pages)}
            onsplit={() => (splitOpen = true)}
            ondone={(p) => setOrganize(false, p)}
          />
        {/key}
      {/if}
      {#if cropFor && viewer}
        <CropDialog
          chosen={cropFor}
          count={viewer.pageCount}
          oncrop={cropPages}
          onclose={() => (cropFor = null)}
        />
      {/if}
      {#if headerFor && viewer}
        <HeaderDialog
          chosen={headerFor}
          count={viewer.pageCount}
          ondecorate={(pages, d: Decor) => pageEdit((doc) => pagesDecorate(doc, pages, d))}
          onremove={(pages) => pageEdit((doc) => pagesUndecorate(doc, pages))}
          onclose={() => (headerFor = null)}
        />
      {/if}
      {#if splitOpen && viewer && active}
        <SplitDialog
          count={viewer.pageCount}
          outline={active.outline}
          base={baseName(active.path)}
          onsplit={splitPages}
          onclose={() => (splitOpen = false)}
        />
      {/if}
      {#if signDialog}
        <SignatureDialog ondone={addSignature} onclose={() => (signDialog = false)} />
      {/if}
      {#if flattenAsk}
        <Dialog
          title={t("flat.title")}
          ok={t("flat.ok")}
          onok={flatten}
          oncancel={() => (flattenAsk = false)}
        >
          <p class="note">{t("flat.body")}</p>
        </Dialog>
      {/if}
      {#if ocrOpen && viewer}
        <OcrDialog
          current={page}
          count={viewer.pageCount}
          options={ocrOptions}
          skipText={ocrSkipText}
          onstart={startOcr}
          onclose={() => (ocrOpen = false)}
        />
      {/if}
      {#if tableEdit && viewer && tableEdit.item.kind.type === "table"}
        {@const te = tableEdit}
        {@const kind = te.item.kind as { type: "table" } & TableData}
        <TableDialog
          table={kind}
          ondone={(tb) => viewer?.setTable(te.page, te.item.id, tb)}
          onclose={() => (tableEdit = null)}
        />
      {/if}
      {#if newOpen}
        <NewDocDialog onmake={makeNew} onclose={() => (newOpen = false)} />
      {/if}
      {#if picturesOpen}
        <PicturesDialog canOcr={ocrInstalled} onmake={makePictures} onclose={() => (picturesOpen = false)} />
      {/if}
      {#if mergeFiles_}
        <MergeDialog initial={mergeFiles_} onmerge={combine} onclose={() => (mergeFiles_ = null)} />
      {/if}
      {#if notice}
        <div class="toast notice" role="status">
          <span dir="auto">{notice.text}</span>
          {#if notice.path}
            {@const where = notice.path}
            <button onclick={() => void revealItemInDir(where).catch((e) => (error = String(e)))}>
              {t("org.showFile")}
            </button>
          {/if}
          <button onclick={() => (notice = null)} aria-label={t("app.dismiss")}>×</button>
        </div>
      {/if}
      {#if viewer && active && findOpen && !active.organize}
        {#key viewer}
          <SearchBar
            bind:this={findBar}
            {viewer}
            options={searchOptions}
            onoptions={(o) => {
              searchOptions = o;
              updateSettings({ searchOptions: o });
            }}
            onclose={() => {
              findOpen = false;
              viewer?.scroller.focus();
            }}
          />
        {/key}
      {/if}
      {#if error}
        <div class="toast" role="alert">
          <span>{error}</span>
          <button onclick={() => (error = "")} aria-label={t("app.dismiss")}>×</button>
        </div>
      {/if}
      {#if pwPrompt}
        <div class="overlay">
          <form class="password" onsubmit={submitPassword}>
            <p dir="auto">
              {t("pw.protected", { name: pwPrompt.path ? nameOf(pwPrompt.path) : nameOf(initialPath ?? "") })}
            </p>
            <label>
              {t("pw.password")}
              <input
                type="password"
                bind:this={pwField}
                bind:value={password}
                autocomplete="off"
                onkeydown={(e) => e.key === "Escape" && (pwPrompt = null)}
              />
            </label>
            {#if pwPrompt.wrong}<p class="error" role="alert">{t("pw.wrong")}</p>{/if}
            <div class="actions">
              <button type="submit">{t("pw.open")}</button>
              <button type="button" onclick={() => (pwPrompt = null)}>{t("common.cancel")}</button>
            </div>
          </form>
        </div>
      {:else if !tabs.length}
        <div class="start">
          <div class="start-inner">
            <h1>{greeting()}</h1>
            <h2>{t("rib.start")}</h2>
            <div class="cards">
              <button class="card primary-card" onclick={openDialog}>
                <span class="card-art"><Icon name="open" size={34} /></span>
                {t("app.openPdf")}
              </button>
              <button class="card" onclick={() => (newOpen = true)}>
                <span class="card-art"><Icon name="newDoc" size={34} /></span>
                {t("new.open")}
              </button>
              <button class="card" onclick={() => (mergeFiles_ = [])}>
                <span class="card-art"><Icon name="merge" size={34} /></span>
                {t("merge.open")}
              </button>
              <button class="card" onclick={() => (picturesOpen = true)}>
                <span class="card-art"><Icon name="pictures" size={34} /></span>
                {t("pics.open")}
              </button>
            </div>
            <p class="hint">{t("app.openHint")}</p>
            {#if recent.length}
              <h2>{t("app.recent")}</h2>
              <ul class="recent">
                {#each recent.slice(0, 12) as r (r.path)}
                  <li>
                    <button class="recent-open" onclick={() => open(r.path)} title={r.path}>
                      <span class="file-icon" aria-hidden="true">PDF</span>
                      <span class="recent-name" dir="auto">{r.name}</span>
                      <span class="recent-meta">{describeRecent(r)}</span>
                      <span class="recent-path" dir="auto">{r.path}</span>
                    </button>
                    <button
                      class="recent-forget"
                      aria-label={t("app.forget", { name: r.name })}
                      title={t("app.forgetTitle")}
                      onclick={() => {
                        forget(r.path);
                        recent = recentFiles();
                      }}
                    >
                      ×
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </div>
        </div>
      {/if}
      {#if restoreOffer}
        <div class="overlay">
          <div class="password" role="alertdialog" aria-modal="true" aria-labelledby="restore-title">
            <p id="restore-title" dir="auto">
              <strong>{t("restore.title", { name: restoreOffer.name })}</strong>
            </p>
            <p>{t("restore.body", { when: new Date(restoreOffer.when).toLocaleString(lang()) })}</p>
            <div class="actions">
              <!-- svelte-ignore a11y_autofocus -->
              <button class="primary" autofocus onclick={() => answerRestore(true)}>
                {t("restore.keep")}
              </button>
              <button onclick={() => answerRestore(false)}>{t("restore.discard")}</button>
            </div>
          </div>
        </div>
      {/if}
      {#if noteEdit}
        <div class="overlay">
          <form
            class="password"
            onsubmit={(e) => {
              e.preventDefault();
              saveNote();
            }}
          >
            <label>
              {noteEdit.create?.kind === "freetext" || noteEdit.a?.kind === "freetext"
                ? t("ctx.textTitle")
                : t("ctx.noteTitle")}
              <!-- svelte-ignore a11y_autofocus -->
              <textarea
                rows="5"
                dir="auto"
                autofocus
                bind:value={noteEdit.text}
                onkeydown={(e) => {
                  if (e.key === "Escape") noteEdit = null;
                  if (e.key === "Enter" && e.ctrlKey) saveNote();
                }}></textarea>
            </label>
            <div class="actions">
              <button type="submit" class="primary">{t("ctx.noteSave")}</button>
              <button type="button" onclick={() => (noteEdit = null)}>{t("common.cancel")}</button>
            </div>
          </form>
        </div>
      {/if}
      {#if unsavedPrompt}
        <div class="overlay">
          <div class="password" role="alertdialog" aria-modal="true" aria-labelledby="unsaved-title">
            <p id="unsaved-title" dir="auto">
              <strong>{t("unsaved.title", { name: unsavedPrompt.tab.name })}</strong>
            </p>
            <p>{t("unsaved.body")}</p>
            <div class="actions">
              <!-- svelte-ignore a11y_autofocus -->
              <button class="primary" autofocus onclick={() => answerUnsaved("save")}>
                {t("unsaved.save")}
              </button>
              <button onclick={() => answerUnsaved("discard")}>{t("unsaved.discard")}</button>
              <button onclick={() => answerUnsaved("cancel")}>{t("common.cancel")}</button>
            </div>
          </div>
        </div>
      {/if}
      {#if printOpen && viewer}
        <PrintDialog
          {viewer}
          {transport}
          dpi={settings().printDpi}
          ondpi={(dpi) => updateSettings({ printDpi: dpi })}
          onclose={() => {
            printOpen = false;
            viewer?.scroller.focus();
          }}
        />
      {/if}
      {#if settingsOpen}
        <SettingsDialog
          {theme}
          {language}
          {checkUpdates}
          {author}
          {defaultAuthor}
          onauthor={(a) => {
            author = a;
            updateSettings({ author: a });
          }}
          onchange={changeSettings}
          oncheck={checkForUpdate}
          onclose={() => {
            settingsOpen = false;
            viewer?.scroller.focus();
          }}
        />
      {/if}
    </main>
  </div>
  {#if viewer}
    <!-- The status bar: page, text recognition, zoom. -->
    <footer>
      <label class="pageno">
        {t("rib.page")}
        <input
          id="page-input"
          inputmode="numeric"
          aria-label={t("bar.pageNumber")}
          bind:value={pageInput}
          onfocus={(e) => {
            editingPage = true;
            (e.currentTarget as HTMLInputElement).select();
          }}
          onblur={() => {
            editingPage = false;
            pageInput = String(page + 1);
          }}
          onkeydown={(e) => {
            if (e.key === "Enter") goToTyped();
            if (e.key === "Escape") (e.currentTarget as HTMLInputElement).blur();
          }}
          title={t("bar.goTo")}
        />
        <span>{t("rib.of", { pages: viewer.pageCount })}</span>
      </label>
      {#if active?.dirty}<span class="st-dirty">{t("unsaved.marker")}</span>{/if}
      {#if active?.ocr}
        {@const run = active.ocr}
        {@const tab = active}
        <span class="ocr-run" role="status">
          <progress max={run.total} value={run.done}></progress>
          {t("ocr.running", { done: Math.min(run.done + 1, run.total), total: run.total })}
          <button onclick={() => stopOcr(tab)}>{t("ocr.stop")}</button>
        </span>
      {/if}
      <span class="grow"></span>
      {#each MODES as [m, icon, label] (m)}
        <button
          class="st-btn"
          class:active={mode === m}
          aria-pressed={mode === m}
          onclick={() => viewer?.setMode(m)}
          title={t(label)}
          aria-label={t(label)}
        >
          <Icon name={icon} size={16} />
        </button>
      {/each}
      <span class="sep"></span>
      <button
        class="st-btn"
        onclick={() => zoomBy(0.8)}
        title={t("bar.zoomOutTitle")}
        aria-label={t("bar.zoomOut")}
      >
        <Icon name="minus" size={16} />
      </button>
      <input
        class="zoom-slider"
        type="range"
        min="0"
        max="100"
        step="any"
        aria-label={t("rib.zoom")}
        value={zoomToSlider(zoomPct)}
        oninput={(e) => setZoomPct(sliderToZoom(Number(e.currentTarget.value)))}
        style:--fill={`${zoomToSlider(zoomPct)}%`}
      />
      <button
        class="st-btn"
        onclick={() => zoomBy(1.25)}
        title={t("bar.zoomInTitle")}
        aria-label={t("bar.zoomIn")}
      >
        <Icon name="plus" size={16} />
      </button>
      <button class="zoom" dir="ltr" onclick={() => viewer?.fitWidth()} title={t("bar.fitWidthTitle")}>
        {zoomPct}%
      </button>
    </footer>
  {/if}
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  header {
    flex: none;
    user-select: none;
  }
  /* Title bar: brand colour in the light theme; the chosen document's tab
     joins the ribbon below it. */
  .titlebar {
    display: flex;
    align-items: stretch;
    height: 42px;
    background: var(--title-bg);
    color: var(--title-text);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-inline: 12px 6px;
  }
  /* The brand symbol on a small tile (docs/brand.md). */
  .logo {
    display: grid;
    place-content: center;
    width: 24px;
    height: 24px;
    border-radius: 7px;
    background: var(--paper);
    color: var(--brand);
    box-shadow: 0 1px 2px var(--tab-shadow);
  }
  :global(:root[data-theme="dark"]) .logo {
    background: linear-gradient(135deg, var(--brand-light), var(--brand-dark));
    color: var(--on-accent);
  }
  .app-name {
    font-weight: 600;
  }
  .qat {
    display: flex;
    align-items: center;
    gap: 2px;
    padding-inline-end: 6px;
    margin-inline-end: 4px;
    border-inline-end: 1px solid var(--title-hover);
    margin-block: 9px;
  }
  .qat button,
  .title-btn,
  .tab-new {
    display: grid;
    place-content: center;
    height: 30px;
    min-width: 30px;
    padding: 0 6px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: inherit;
  }
  .title-btn {
    align-self: center;
    margin-inline-end: 4px;
  }
  .qat button:hover:not(:disabled),
  .title-btn:hover,
  .tab-new:hover {
    background: var(--title-hover);
  }
  .qat button:disabled {
    opacity: 0.45;
  }
  .tabbar {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    min-width: 0;
    padding: 6px 12px 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tab-new {
    align-self: center;
    margin-top: 4px;
  }
  .drag {
    flex: 1;
    min-width: 40px;
  }
  .winctl {
    display: flex;
    flex: none;
  }
  .winctl button {
    width: 46px;
    height: 100%;
    border: 0;
    border-radius: 0;
    background: transparent;
    color: inherit;
    font-family: "Segoe Fluent Icons", "Segoe MDL2 Assets";
    font-size: 10px;
    padding: 0;
  }
  .winctl button:hover {
    background: var(--title-hover);
  }
  .winctl button.close:hover {
    background: var(--close-hover);
    color: var(--on-accent);
  }
  .tab {
    position: relative;
    display: flex;
    align-items: center;
    gap: 7px;
    height: 34px;
    max-width: 230px;
    min-width: 90px;
    padding: 0 6px 0 10px;
    border-radius: 9px 9px 0 0;
    color: var(--tab-text);
    cursor: default;
    transition:
      background-color 0.15s,
      color 0.15s;
  }
  .tab:hover {
    background: var(--tab-hover);
  }
  .tab.active {
    background: var(--tab-active);
    color: var(--text);
    font-weight: 600;
  }
  /* Curves where the chosen tab meets the ribbon. */
  .tab.active::before,
  .tab.active::after {
    content: "";
    position: absolute;
    bottom: 0;
    width: 9px;
    height: 9px;
    pointer-events: none;
  }
  .tab.active::before {
    left: -9px;
    background: radial-gradient(circle at 0 0, transparent 9px, var(--tab-active) 9.5px);
  }
  .tab.active::after {
    right: -9px;
    background: radial-gradient(circle at 100% 0, transparent 9px, var(--tab-active) 9.5px);
  }
  .tab:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .tab-icon,
  .file-icon {
    flex: none;
    padding: 1px 3px;
    border-radius: 3px;
    background: var(--pdf-badge);
    color: var(--on-accent);
    font-size: 8px;
    font-weight: 700;
    letter-spacing: 0.02em;
  }
  .tab-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tab-close {
    display: grid;
    place-content: center;
    width: 22px;
    height: 22px;
    border: 0;
    background: none;
    color: inherit;
    padding: 0;
    border-radius: 5px;
    opacity: 0.7;
  }
  .tab-close:hover {
    background: var(--control-hover) !important;
    opacity: 1;
  }
  .tab:not(.active) .tab-close:hover {
    background: var(--title-hover) !important;
  }
  /* The ribbon: tab names, then a card of commands. */
  .ribbon {
    background: var(--bar);
    padding: 0 8px 8px;
    border-bottom: 1px solid var(--edge);
  }
  .rtabs {
    display: flex;
    gap: 2px;
    padding: 4px;
  }
  .rtab {
    position: relative;
    padding: 5px 12px 7px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--muted);
    transition:
      color 0.15s,
      background-color 0.15s;
  }
  .rtab:hover {
    background: var(--control-hover);
    color: var(--text);
  }
  .rtab.on {
    color: var(--accent);
    font-weight: 600;
  }
  .rtab::after {
    content: "";
    position: absolute;
    left: 50%;
    bottom: 2px;
    width: 0;
    height: 3px;
    border-radius: 2px;
    background: currentColor;
    transform: translateX(-50%);
    transition: width 0.2s ease;
  }
  .rtab.on::after {
    width: calc(100% - 22px);
  }
  .rtab.contextual,
  .rtab.contextual.on {
    color: var(--format);
  }
  .ctxbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 6px;
    min-height: 48px;
    padding: 4px 10px;
    background: var(--ribbon);
    border-radius: 10px;
    /* The shadow stays inside the ribbon's padding: one reaching over the
       page view makes scrolling repaint it. */
    box-shadow:
      0 1px 2px var(--ribbon-shadow),
      0 1px 5px var(--ribbon-shadow);
    white-space: nowrap;
  }
  .panel {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }
  /* Only when the reader changes tab: an animation as a document opens
     delays its first pages. */
  .ctxbar.switched .panel {
    animation: panel-in 0.18s ease;
  }
  .panel[hidden] {
    display: none;
  }
  @keyframes panel-in {
    from {
      opacity: 0;
      transform: translateY(-3px);
    }
  }
  /* Flat ribbon buttons: no frame until pointed at. */
  .ctxbar button:not(.primary):not(.swatch),
  header select,
  footer button {
    border-color: transparent;
    background: transparent;
    border-radius: 6px;
  }
  .ctxbar button:not(.primary):not(.swatch) {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 36px;
    transition:
      background-color 0.12s,
      transform 0.08s;
  }
  .ctxbar button:not(.primary):not(.swatch):hover:not(:disabled),
  header select:hover,
  footer button:hover:not(:disabled) {
    background: var(--control-hover);
  }
  .ctxbar button:not(.swatch):active:not(:disabled) {
    transform: scale(0.96);
  }
  .ctxbar button.active,
  footer button.active {
    background: var(--selected) !important;
    color: var(--accent);
    border-color: transparent;
  }
  .ctxbar button.big {
    padding: 4px 10px;
  }
  .grow {
    flex: 1;
  }
  .ctx-hint {
    color: var(--muted);
  }
  .ctx-kind {
    font-weight: 600;
  }
  .ctx-by {
    font-weight: 400;
    color: var(--muted);
  }
  .ctx-width {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
  }
  .ocr-run {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-inline-start: 8px;
  }
  .ocr-run progress {
    width: 90px;
    accent-color: var(--accent);
  }
  textarea {
    width: min(420px, 80vw);
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 6px 8px;
    font: inherit;
    resize: vertical;
  }
  .dirty {
    color: var(--accent);
    font-size: 10px;
  }
  .tab:not(.active) .dirty {
    color: inherit;
  }
  .marks {
    display: flex;
    gap: 4px;
    padding-inline: 4px;
  }
  .swatch {
    width: 18px;
    height: 18px;
    padding: 0;
    border-radius: 50%;
    border: 2px solid var(--ribbon);
    box-shadow: 0 0 0 1px var(--border);
    transition: transform 0.12s;
  }
  .swatch:hover {
    transform: scale(1.15);
  }
  .swatch.active {
    box-shadow: 0 0 0 2px var(--accent);
  }
  .mk {
    display: inline-block;
    min-width: 16px;
    padding: 0 3px;
    line-height: 18px;
    font-size: 14px;
    font-weight: 600;
  }
  .mk.hl {
    color: #000;
    border-radius: 3px;
  }
  .mk.ul {
    text-decoration: underline;
    text-decoration-thickness: 2px;
    text-decoration-color: var(--hue-blue);
  }
  .mk.so {
    text-decoration: line-through;
    text-decoration-thickness: 2px;
    text-decoration-color: var(--hue-red);
  }
  .mk.sq {
    text-decoration: underline wavy;
    text-decoration-color: var(--hue-green);
  }
  .update {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 8px 8px 0;
    padding: 6px 8px 6px 14px;
    background: var(--note-bg);
    border: 1px solid var(--note-border);
    border-radius: 10px;
    flex: none;
  }
  .update button:not(.primary) {
    border-color: transparent;
    background: transparent;
    border-radius: 6px;
  }
  .update + .update {
    margin-top: 6px;
  }
  .update + .body,
  .update ~ .body {
    margin-top: 8px;
  }
  .sep {
    flex: none;
    width: 1px;
    height: 26px;
    margin-inline: 4px;
    background: var(--border);
    opacity: 0.7;
  }
  /* Status bar. */
  footer {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 30px;
    padding: 0 10px;
    flex: none;
    background: var(--bar);
    border-top: 1px solid var(--edge);
    color: var(--muted);
    font-size: 12px;
    user-select: none;
    white-space: nowrap;
  }
  footer .sep {
    height: 16px;
  }
  .st-btn {
    display: grid;
    place-content: center;
    width: 26px;
    height: 24px;
    padding: 0 !important;
    color: inherit;
  }
  .st-dirty {
    margin-inline-start: 8px;
    padding: 1px 8px;
    border-radius: 10px;
    background: var(--accent-soft);
    color: var(--accent);
  }
  .pageno {
    display: flex;
    align-items: center;
    gap: 6px;
    font-variant-numeric: tabular-nums;
  }
  .pageno input {
    width: 5ch;
    height: 22px;
    padding: 0 4px;
    text-align: center;
    border-color: transparent;
    background: var(--field-bg);
    border-radius: 5px;
    font-size: 12px;
  }
  .zoom-slider {
    appearance: none;
    width: 110px;
    height: 4px;
    margin: 0 4px;
    padding: 0;
    border: 0;
    border-radius: 2px;
    background: linear-gradient(to right, var(--accent) var(--fill), var(--border) var(--fill));
    cursor: pointer;
  }
  :global(html[dir="rtl"]) .zoom-slider {
    background: linear-gradient(to left, var(--accent) var(--fill), var(--border) var(--fill));
  }
  .zoom-slider::-webkit-slider-thumb {
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--ribbon);
    border: 2px solid var(--accent);
    transition: transform 0.1s;
  }
  .zoom-slider:hover::-webkit-slider-thumb {
    transform: scale(1.2);
  }
  .zoom {
    min-width: 6ch;
    padding: 2px 6px !important;
    text-align: center;
    font-size: 12px;
    color: inherit;
    font-variant-numeric: tabular-nums;
  }
  button,
  select,
  input {
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 3px 8px;
    font: inherit;
  }
  button {
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  button:hover:not(:disabled),
  select:hover {
    background: var(--control-hover);
  }
  button.active {
    border-color: var(--accent);
    background: var(--selected);
  }
  button:focus-visible,
  select:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  main {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  .doc {
    position: absolute;
    inset: 0;
  }
  .doc[hidden] {
    display: none;
  }
  main :global(.scroller:focus) {
    outline: none;
  }
  .overlay {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    background: var(--overlay);
    z-index: 10;
  }
  .password {
    display: grid;
    gap: 12px;
    min-width: 320px;
    padding: 20px 24px;
    background: var(--bar);
    border: 1px solid var(--border);
    border-radius: 8px;
    color: var(--text);
  }
  .password p {
    margin: 0;
  }
  .password label {
    display: grid;
    gap: 4px;
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
  .error {
    color: var(--error);
  }
  .toast {
    position: absolute;
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    display: flex;
    gap: 12px;
    align-items: center;
    max-width: min(640px, 90%);
    padding: 8px 8px 8px 14px;
    background: var(--toast-bg);
    border: 1px solid var(--toast-border);
    border-radius: 6px;
    color: var(--toast-text);
    z-index: 11;
  }
  .menu {
    position: relative;
  }
  .menu-list {
    position: absolute;
    top: 100%;
    inset-inline-start: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 220px;
    padding: 6px;
    background: var(--control);
    border: 1px solid var(--border);
    border-radius: 6px;
    box-shadow: 0 4px 12px var(--popup-shadow);
  }
  .sig-item {
    display: flex;
    gap: 4px;
    align-items: stretch;
  }
  .sig-pick {
    flex: 1;
    background: var(--paper) !important;
    padding: 4px;
  }
  .sig-pick img {
    display: block;
    max-width: 200px;
    max-height: 50px;
    margin: auto;
  }
  .toast.notice {
    background: var(--bar);
    border-color: var(--border);
    color: var(--text);
  }

  .start {
    position: absolute;
    inset: 0;
    overflow: auto;
    background: radial-gradient(1200px 400px at 10% -10%, var(--accent-soft), transparent 70%), var(--bg);
  }
  .start-inner {
    max-width: 860px;
    margin: 44px auto;
    padding: 0 24px;
    animation: panel-in 0.25s ease;
  }
  h1 {
    margin: 0 0 24px;
    font-size: 28px;
    font-weight: 600;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 14px;
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 10px;
    padding: 10px 10px 12px;
    border: 1px solid var(--edge);
    border-radius: 12px;
    background: var(--ribbon);
    text-align: start;
    font-weight: 600;
    box-shadow: 0 1px 3px var(--ribbon-shadow);
    transition:
      transform 0.15s,
      box-shadow 0.15s,
      border-color 0.15s;
  }
  .card:hover {
    background: var(--ribbon) !important;
    transform: translateY(-2px);
    border-color: var(--accent);
    box-shadow: 0 6px 18px var(--ribbon-shadow);
  }
  .card-art {
    display: grid;
    place-content: center;
    height: 92px;
    border-radius: 8px;
    background: var(--bar);
  }
  .primary-card .card-art {
    background: linear-gradient(135deg, var(--brand-light), var(--brand-dark));
    color: var(--on-accent);
    --hue-yellow: var(--on-accent);
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
    padding: 6px 14px;
  }
  .primary:hover {
    background: var(--primary-hover) !important;
  }
  .hint {
    margin: 14px 0 0;
    color: var(--muted);
  }
  h2 {
    margin: 32px 0 10px;
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
  }
  .recent {
    list-style: none;
    margin: 0;
    padding: 4px;
    border: 1px solid var(--edge);
    border-radius: 12px;
    background: var(--ribbon);
  }
  .recent li {
    display: flex;
    align-items: stretch;
    border-radius: 8px;
    transition: background-color 0.12s;
  }
  .recent li:hover {
    background: var(--control-hover);
  }
  .recent-open {
    flex: 1;
    min-width: 0;
    display: grid;
    grid-template-columns: auto 1fr auto;
    grid-template-areas:
      "icon name meta"
      "icon path path";
    align-items: center;
    column-gap: 12px;
    row-gap: 2px;
    padding: 8px 10px;
    background: none;
    border: 0;
    text-align: start;
  }
  .recent-open .file-icon {
    grid-area: icon;
    padding: 5px 4px;
    font-size: 9px;
    border-radius: 4px;
  }
  .recent-open:hover {
    background: none !important;
  }
  .recent-name {
    grid-area: name;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .recent-meta {
    grid-area: meta;
  }
  .recent-path {
    grid-area: path;
  }
  .recent-meta,
  .recent-path {
    color: var(--muted);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .recent-forget {
    background: none;
    border: 0;
    color: var(--muted);
    visibility: hidden;
  }
  .recent li:hover .recent-forget,
  .recent-forget:focus-visible {
    visibility: visible;
  }
</style>
