<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as pickFile, save as pickSavePath } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
    annotMarkup,
    editRedo,
    editUndo,
    saveDoc,
    userName,
    type Changed,
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
  import { Viewer, type FitMode, type ViewMode } from "./viewer";
  import { benchNoFile, benchWithFile } from "./bench";
  import Sidebar from "./Sidebar.svelte";
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
  }

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
  let sidebarPanel: "pages" | "bookmarks" = $state("pages");
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
  let busy = false; // one edit at a time, in order
  // Set while asking whether to save a document before it closes.
  let unsavedPrompt: { tab: Tab; resolve: (r: "save" | "discard" | "cancel") => void } | null = $state(null);
  let settingsOpen = $state(false);
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
        },
      ];
      activeId = id;
      await tick();
      const v = new Viewer(hosts[id], info, transport);
      v.onUpdate = () => refresh(id);
      v.onExternalLink = (uri) => void openUrl(uri).catch((e) => (error = String(e)));
      v.onSelection = (has) => {
        if (id === activeId) hasSel = has;
      };
      const saved = bench ? undefined : lastView(realPath);
      if (saved) v.restore(saved);
      patchTab(id, { viewer: v });
      activate(id);
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

  /** Takes in a change: undo state on the tab, and pages drawn again. */
  function changed(tabId: number, c: Changed) {
    patchTab(tabId, { dirty: c.status.dirty, canUndo: c.status.can_undo, canRedo: c.status.can_redo });
    tabs.find((t) => t.id === tabId)?.viewer?.invalidatePages(c.pages);
  }

  /** Runs edits one after another, so they reach the history in order. */
  async function edit(fn: (tab: Tab, v: Viewer) => Promise<void>) {
    const tab = active;
    const v = tab?.viewer;
    if (!tab || !v || busy) return;
    busy = true;
    try {
      await fn(tab, v);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function markup(kind: MarkupKind) {
    void edit(async (tab, v) => {
      const parts = await v.selectionRects();
      for (const { page, rects } of parts) {
        changed(
          tab.id,
          await annotMarkup(v.info.doc, page, kind, rects, markupColor, 1, author || defaultAuthor),
        );
      }
      v.clearSelection();
    });
  }

  function undo() {
    void edit(async (tab, v) => changed(tab.id, await editUndo(v.info.doc)));
  }

  function redo() {
    void edit(async (tab, v) => changed(tab.id, await editRedo(v.info.doc)));
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
      if (r.full) error = t("save.full");
      // Pages are now drawn from the saved file.
      v.invalidatePages(v.pagesInView(800));
      return true;
    } catch (e) {
      error = t("save.failed", { error: String(e) });
      return false;
    }
  }

  function saveActive(as: boolean) {
    if (active && !busy) {
      busy = true;
      void save(active, as).finally(() => (busy = false));
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

  function setPanel(p: "pages" | "bookmarks") {
    sidebarPanel = p;
    updateSettings({ sidebarPanel: p });
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
    if (printOpen || settingsOpen || pwPrompt || unsavedPrompt) return;
    const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement;
    const action = appKeyAction(e) ?? (viewer && !typing ? keyAction(e, viewer) : null);
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

    const unlistenDrop = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "drop") {
        for (const p of e.payload.paths.filter((p) => p.toLowerCase().endsWith(".pdf"))) void open(p);
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
      void unlistenFiles.then((f) => f());
      void unlistenClose.then((f) => f());
    };
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="app">
  {#if tabs.length}
    <div class="tabbar" role="tablist" aria-label={t("app.documents")}>
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
      <button class="tab-new" onclick={openDialog} title={t("app.openTitle")} aria-label={t("app.openPdf")}>
        +
      </button>
    </div>
  {/if}
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
      <span class="sep"></span>
      <span class="marks" role="group" aria-label={t("mark.color")}>
        {#each MARKUP_COLORS as c (c.css)}
          <button
            class="swatch"
            class:active={markupColor.join() === c.rgb.join()}
            style:background={c.css}
            aria-label={c.css}
            aria-pressed={markupColor.join() === c.rgb.join()}
            title={t("mark.color")}
            onclick={() => {
              markupColor = c.rgb;
              updateSettings({ markupColor: c.rgb });
            }}
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
    {#if viewer && active && sidebarOpen}
      {#key active.id}
        <Sidebar {viewer} {page} outline={active.outline} panel={sidebarPanel} onpanel={setPanel} />
      {/key}
    {/if}
    <main onwheel={onWheel}>
      {#each tabs as tab (tab.id)}
        <div class="doc" hidden={tab.id !== activeId} bind:this={hosts[tab.id]}></div>
      {/each}
      {#if viewer && active && findOpen}
        {#key active.id}
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
  .tabbar {
    display: flex;
    align-items: stretch;
    gap: 1px;
    height: 32px;
    padding: 0 6px;
    background: var(--tabbar);
    overflow-x: auto;
    flex: none;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 220px;
    min-width: 80px;
    padding: 0 6px 0 12px;
    margin-top: 4px;
    border-radius: 6px 6px 0 0;
    color: var(--muted);
    cursor: default;
    user-select: none;
  }
  .tab:hover {
    background: var(--tab-hover);
  }
  .tab.active {
    background: var(--bar);
    color: var(--text);
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
    margin-top: 4px;
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
  .name {
    font-weight: 600;
  }
  .grow {
    flex: 1;
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
