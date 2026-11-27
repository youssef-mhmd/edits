<script lang="ts">
  import { onMount, tick } from "svelte";
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
  import SignatureDialog from "./SignatureDialog.svelte";
  import Dialog from "./Dialog.svelte";
  import SearchBar from "./SearchBar.svelte";
  import PrintDialog from "./PrintDialog.svelte";
  import SettingsDialog from "./SettingsDialog.svelte";
  import { lang, t, type LangSetting } from "./i18n.svelte";
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
  }

  const STAMP_WORDS = ["معتمد", "مسودة", "سري", "APPROVED", "DRAFT", "CONFIDENTIAL", "RECEIVED"];
  const TOOLS: [Tool, string][] = [
    ["select", "↖"],
    ["ink", "✎"],
    ["line", "╱"],
    ["arrow", "↗"],
    ["square", "▭"],
    ["circle", "◯"],
    ["note", "🗨"],
    ["text", "T"],
    ["stamp", "⎔"],
    ["image", "🖼"],
    ["tick", "✓"],
    ["cross", "✗"],
  ];

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

  /** Takes in a change: undo state on the tab, pages drawn again, annotations read again. */
  function changed(tabId: number, c: Changed) {
    patchTab(tabId, { dirty: c.status.dirty, canUndo: c.status.can_undo, canRedo: c.status.can_redo });
    if (c.info) {
      replaceViewer(tabId, c.info);
      if (tabId === activeId && comments !== null) void loadComments();
      return;
    }
    const v = tabs.find((t) => t.id === tabId)?.viewer;
    v?.invalidatePages(c.pages);
    v?.reloadAnnots(c.pages);
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
    if (!text) return;
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
    if (cropFor || headerFor || splitOpen || mergeFiles_ || signDialog || flattenAsk) return;
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
        applyLook(theme, language);
        // Look for an update once start-up is well over; never in tests.
        if (checkUpdates && !navigator.webdriver) setTimeout(() => void checkForUpdate(), 5000);
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
      void unlistenClose.then((f) => f());
    };
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="app">
  <!-- The window's own title bar: tabs, then space to drag the window, then its buttons. -->
  <div class="titlebar" data-tauri-drag-region>
    {#if !tabs.length}<span class="app-name" data-tauri-drag-region>edits</span>{/if}
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
          {#if tab.dirty}<span class="dirty" title={t("unsaved.marker")}>●</span>{/if}
          <span class="tab-name" dir="auto">{tab.name}</span>
          <button
            class="tab-close"
            aria-label={t("app.closeTab", { name: tab.name })}
            title={t("app.closeTabTitle")}
            onclick={(e) => {
              e.stopPropagation();
              closeTab(tab.id);
            }}
          >
            ×
          </button>
        </div>
      {/each}
      {#if tabs.length}
        <button class="tab-new" onclick={openDialog} title={t("app.openTitle")} aria-label={t("app.openPdf")}>
          +
        </button>
      {/if}
    </div>
    <span class="drag" data-tauri-drag-region></span>
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
  <header>
    {#if viewer}
      <button
        class:active={sidebarOpen}
        aria-pressed={sidebarOpen}
        onclick={toggleSidebar}
        title={t("bar.panelTitle")}
      >
        {t("bar.panel")}
      </button>
      <button
        class:active={active?.organize}
        aria-pressed={!!active?.organize}
        onclick={() => setOrganize(!active?.organize, active?.organize ? page : undefined)}
        title={t("org.openTitle")}
      >
        {t("org.open")}
      </button>
      <button
        disabled={!canBack}
        onclick={() => viewer?.goBack()}
        title={t("bar.backTitle")}
        aria-label={t("bar.back")}
      >
        {rtlUi ? "→" : "←"}
      </button>
      <button
        disabled={!canForward}
        onclick={() => viewer?.goForward()}
        title={t("bar.forwardTitle")}
        aria-label={t("bar.forward")}
      >
        {rtlUi ? "←" : "→"}
      </button>
      <span class="sep"></span>
      <label class="pageno">
        <span class="visually-hidden">{t("bar.pageNumber")}</span>
        <input
          id="page-input"
          inputmode="numeric"
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
        <span dir="ltr">/ {viewer.pageCount}</span>
      </label>
      <span class="sep"></span>
      <button onclick={() => zoomBy(0.8)} title={t("bar.zoomOutTitle")} aria-label={t("bar.zoomOut")}>
        −
      </button>
      <span class="zoom" dir="ltr">{zoomPct}%</span>
      <button onclick={() => zoomBy(1.25)} title={t("bar.zoomInTitle")} aria-label={t("bar.zoomIn")}>
        +
      </button>
      <button
        class:active={fit === "width"}
        aria-pressed={fit === "width"}
        onclick={() => viewer?.fitWidth()}
        title={t("bar.fitWidthTitle")}
      >
        {t("bar.fitWidth")}
      </button>
      <button
        class:active={fit === "page"}
        aria-pressed={fit === "page"}
        onclick={() => viewer?.fitPage()}
        title={t("bar.fitPageTitle")}
      >
        {t("bar.fitPage")}
      </button>
      <span class="sep"></span>
      <label>
        <span class="visually-hidden">{t("bar.view")}</span>
        <select
          value={mode}
          onchange={(e) => viewer?.setMode(e.currentTarget.value as ViewMode)}
          title={t("bar.layout")}
        >
          <option value="continuous">{t("bar.continuous")}</option>
          <option value="single">{t("bar.single")}</option>
          <option value="two-page">{t("bar.twoPage")}</option>
        </select>
      </label>
      {#if mode === "two-page"}
        <button
          class:active={rtl}
          aria-pressed={rtl}
          onclick={() => viewer?.setRtl(!rtl)}
          title={t("bar.rtlTitle")}
        >
          {t("bar.rtl")}
        </button>
      {/if}
      <span class="sep"></span>
      <button
        onclick={() => viewer?.rotate(-1)}
        title={t("bar.rotateLeftTitle")}
        aria-label={t("bar.rotateLeft")}
      >
        ⟲
      </button>
      <button
        onclick={() => viewer?.rotate(1)}
        title={t("bar.rotateRightTitle")}
        aria-label={t("bar.rotateRight")}
      >
        ⟳
      </button>
      <span class="sep"></span>
      <button onclick={() => (printOpen = true)} title={t("bar.printTitle")}>{t("bar.print")}</button>
      <button onclick={() => saveActive(false)} disabled={!active?.dirty} title={t("save.saveTitle")}>
        {t("save.save")}
      </button>
      <span class="sep"></span>
      <button
        disabled={!active?.canUndo}
        onclick={undo}
        title={t("edit.undoTitle")}
        aria-label={t("edit.undo")}
      >
        ↶
      </button>
      <button
        disabled={!active?.canRedo}
        onclick={redo}
        title={t("edit.redoTitle")}
        aria-label={t("edit.redo")}
      >
        ↷
      </button>
    {:else}
      <button onclick={openDialog} title={t("app.openTitle")}>{t("app.open")}</button>
      <span class="name">edits</span>
    {/if}
    <span class="grow"></span>
    <button
      onclick={() => (settingsOpen = true)}
      title={t("bar.settingsTitle")}
      aria-label={t("bar.settings")}
    >
      ⚙
    </button>
  </header>
  {#if viewer && !active?.organize}
    <div class="ctxbar" role="toolbar" aria-label={t("ctx.label")}>
      {#if !selAnnot || selAnnot.kind !== "other"}
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
      {/if}
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
      <span class="tools" role="group" aria-label={t("tool.group")}>
        {#each TOOLS as [id, icon] (id)}
          <button
            class:active={tool === id}
            aria-pressed={tool === id}
            title={t(`tool.${id}` as "tool.select")}
            aria-label={t(`tool.${id}` as "tool.select")}
            onclick={() => setTool(id as Tool)}
          >
            {icon}
          </button>
        {/each}
      </span>
      <span class="menu sign">
        <button
          class:active={tool === "sign"}
          aria-pressed={tool === "sign"}
          aria-haspopup="menu"
          aria-expanded={signMenu}
          title={t("sig.menuTitle")}
          onclick={() => (signatures.length ? (signMenu = !signMenu) : (signDialog = true))}
        >
          ✍ {t("sig.sign")}
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
      <button onclick={() => (flattenAsk = true)} title={t("flat.title")}>{t("flat.button")}</button>
      {#if tool !== "select" || selAnnot}<span class="sep"></span>{/if}
      {#if selAnnot}
        <span class="ctx-kind">
          {t(`kind.${selAnnot.kind}` as "kind.other")}{#if selAnnot.author}<span class="ctx-by" dir="auto">
              · {selAnnot.author}
            </span>{/if}
        </span>
      {/if}
      {#if (!selAnnot && tool !== "select") || (selAnnot && ["ink", "line", "square", "circle"].includes(selAnnot.kind))}
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
    </div>
  {/if}
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
            <p>
              <button class="primary" onclick={openDialog}>{t("app.openPdf")}</button>
              <button onclick={() => (mergeFiles_ = [])}>{t("merge.open")}</button>
              <span class="hint">{t("app.openHint")}</span>
            </p>
            {#if recent.length}
              <h2>{t("app.recent")}</h2>
              <ul class="recent">
                {#each recent.slice(0, 12) as r (r.path)}
                  <li>
                    <button class="recent-open" onclick={() => open(r.path)} title={r.path}>
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
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  /* The title bar, tabs and toolbars are one surface: tabs are pills on it. */
  .titlebar {
    display: flex;
    align-items: stretch;
    height: 40px;
    background: var(--bar);
    flex: none;
    user-select: none;
  }
  .app-name {
    align-self: center;
    padding: 0 14px;
    font-weight: 600;
    color: var(--muted);
  }
  .tabbar {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    padding: 4px 8px 0;
    overflow-x: auto;
    scrollbar-width: none;
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
    color: var(--text);
    font-family: "Segoe Fluent Icons", "Segoe MDL2 Assets";
    font-size: 10px;
    padding: 0;
  }
  .winctl button:hover {
    background: var(--control-hover);
  }
  .winctl button.close:hover {
    background: var(--close-hover);
    color: var(--on-accent);
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    max-width: 220px;
    min-width: 80px;
    padding: 0 6px 0 12px;
    border-radius: 8px;
    border: 1px solid transparent;
    color: var(--tab-text);
    cursor: default;
    user-select: none;
    transition: background-color 0.12s;
  }
  .tab:hover {
    background: var(--tab-hover);
  }
  .tab.active {
    background: var(--tab-active);
    border-color: var(--tab-border);
    color: var(--text);
    font-weight: 600;
    box-shadow: 0 1px 3px var(--tab-shadow);
  }
  .tab:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .tab-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tab-close,
  .tab-new {
    border: 0;
    background: none;
    color: inherit;
    padding: 0 4px;
    border-radius: 4px;
  }
  .tab-close:hover,
  .tab-new:hover {
    background: var(--control-hover);
  }
  .tab-new {
    height: 28px;
    padding: 0 10px;
    font-size: 16px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    padding: 0 12px;
    background: var(--bar);
    border-bottom: 1px solid var(--edge);
    user-select: none;
    white-space: nowrap;
    overflow-x: auto;
    flex: none;
  }
  /* One line under the whole top area, below the last bar. */
  header:has(+ .ctxbar) {
    border-bottom: none;
  }
  /* Flat toolbar buttons: no frame until pointed at. */
  header button:not(.primary),
  .ctxbar button:not(.primary):not(.swatch),
  header select,
  .ctxbar select {
    border-color: transparent;
    background: transparent;
    border-radius: 6px;
  }
  header button:not(.primary):hover:not(:disabled),
  .ctxbar button:not(.primary):not(.swatch):hover:not(:disabled),
  header select:hover,
  .ctxbar select:hover {
    background: var(--control-hover);
  }
  header button.active,
  .ctxbar button.active {
    background: var(--selected);
    border-color: transparent;
  }
  header input {
    border-color: transparent;
    background: var(--field-bg);
    border-radius: 6px;
  }
  .name {
    font-weight: 600;
  }
  .grow {
    flex: 1;
  }
  .ctxbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
    min-height: 34px;
    padding: 3px 12px;
    background: var(--bar);
    border-bottom: 1px solid var(--edge);
    flex: none;
    white-space: nowrap;
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
  .tools {
    display: flex;
    gap: 2px;
  }
  textarea {
    width: min(420px, 80vw);
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 6px 8px;
    font: inherit;
    resize: vertical;
  }
  .dirty {
    color: var(--accent);
    font-size: 10px;
  }
  .marks {
    display: flex;
    gap: 3px;
  }
  .swatch {
    width: 16px;
    height: 16px;
    padding: 0;
    border-radius: 50%;
    border: 1px solid var(--border);
  }
  .swatch.active {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .mk {
    display: inline-block;
    min-width: 14px;
    padding: 0 2px;
    line-height: 16px;
    font-weight: 600;
  }
  .mk.hl {
    color: #000;
    border-radius: 2px;
  }
  .mk.ul {
    text-decoration: underline;
    text-decoration-thickness: 2px;
  }
  .mk.so {
    text-decoration: line-through;
    text-decoration-thickness: 2px;
  }
  .mk.sq {
    text-decoration: underline wavy;
  }
  .update {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    background: var(--note-bg);
    border-bottom: 1px solid var(--note-border);
    flex: none;
  }
  .sep {
    flex: none;
    width: 1px;
    height: 18px;
    background: var(--border);
  }
  .pageno {
    display: flex;
    align-items: center;
    gap: 6px;
    font-variant-numeric: tabular-nums;
  }
  .pageno input {
    width: 5ch;
    text-align: center;
  }
  .zoom {
    min-width: 4ch;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  button,
  select,
  input {
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
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
  }
  .start-inner {
    max-width: 640px;
    margin: 48px auto;
    padding: 0 16px;
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
    margin-inline-start: 10px;
    color: var(--muted);
  }
  h2 {
    margin: 32px 0 8px;
    font-size: 13px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .recent {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .recent li {
    display: flex;
    align-items: stretch;
    border-radius: 6px;
  }
  .recent li:hover {
    background: var(--control);
  }
  .recent-open {
    flex: 1;
    min-width: 0;
    display: grid;
    gap: 2px;
    padding: 8px 10px;
    background: none;
    border: 0;
    text-align: start;
  }
  .recent-open:hover {
    background: none !important;
  }
  .recent-name {
    font-weight: 600;
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
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
