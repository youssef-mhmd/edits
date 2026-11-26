<script lang="ts">
  import { onMount, tick } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { open as pickFile } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import {
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
  }

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
  let transport: Transport = transports.protocol;

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
      tabs = [...tabs, { id, path: realPath, name: nameOf(realPath), viewer: null, outline: null }];
      activeId = id;
      await tick();
      const v = new Viewer(hosts[id], info, transport);
      v.onUpdate = () => refresh(id);
      v.onExternalLink = (uri) => void openUrl(uri).catch((e) => (error = String(e)));
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

  function closeTab(id: number) {
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
    if (viewer && e.key === "F3") {
      return () => (findOpen && findBar ? findBar.step(e.shiftKey ? -1 : 1) : void openFind());
    }
    return null;
  }

  function onKey(e: KeyboardEvent) {
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
    const at = typeof r.page === "number" ? `page ${r.page + 1} of ${r.pages}` : `${r.pages} pages`;
    return `${at} · ${new Date(r.opened).toLocaleDateString()}`;
  }

  onMount(() => {
    // Measured as "usable window" in bench mode: first frame after mount.
    let windowReady = 0;
    requestAnimationFrame(() => (windowReady = performance.timeOrigin + performance.now()));

    void (async () => {
      const s = await startup();
      bench = s.bench;
      transport = transports[s.transport];
      initialPath = s.file;
      if (!bench) {
        await loadRecent();
        recent = recentFiles();
        sidebarOpen = settings().sidebarOpen;
        sidebarPanel = settings().sidebarPanel;
        searchOptions = settings().searchOptions;
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
    const unlistenClose = getCurrentWindow().onCloseRequested(async () => {
      if (bench) return;
      for (const t of tabs) if (t.viewer) remember(t.path, t.name, t.viewer.pageCount, t.viewer.viewState());
      await flushRecent();
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
    <div class="tabbar" role="tablist" aria-label="Open documents">
      {#each tabs as t (t.id)}
        <div
          class="tab"
          class:active={t.id === activeId}
          role="tab"
          tabindex="0"
          aria-selected={t.id === activeId}
          title={t.path}
          onclick={() => activate(t.id)}
          onkeydown={(e) => e.key === "Enter" && activate(t.id)}
          onauxclick={(e) => e.button === 1 && closeTab(t.id)}
        >
          <span class="tab-name" dir="auto">{t.name}</span>
          <button
            class="tab-close"
            aria-label="Close {t.name}"
            title="Close (Ctrl W)"
            onclick={(e) => {
              e.stopPropagation();
              closeTab(t.id);
            }}
          >
            ×
          </button>
        </div>
      {/each}
      <button class="tab-new" onclick={openDialog} title="Open a PDF (Ctrl O)" aria-label="Open a PDF">
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
        title="Pages and bookmarks (F4)"
      >
        Panel
      </button>
      <button disabled={!canBack} onclick={() => viewer?.goBack()} title="Back (Alt ←)" aria-label="Back">
        ←
      </button>
      <button
        disabled={!canForward}
        onclick={() => viewer?.goForward()}
        title="Forward (Alt →)"
        aria-label="Forward"
      >
        →
      </button>
      <span class="sep"></span>
      <label class="pageno">
        <span class="visually-hidden">Page number</span>
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
          title="Go to page (Ctrl G)"
        />
        <span>/ {viewer.pageCount}</span>
      </label>
      <span class="sep"></span>
      <button onclick={() => zoomBy(0.8)} title="Zoom out (Ctrl -)" aria-label="Zoom out">−</button>
      <span class="zoom">{zoomPct}%</span>
      <button onclick={() => zoomBy(1.25)} title="Zoom in (Ctrl +)" aria-label="Zoom in">+</button>
      <button
        class:active={fit === "width"}
        aria-pressed={fit === "width"}
        onclick={() => viewer?.fitWidth()}
        title="Fit width, follows the window size (Ctrl 0)"
      >
        Fit width
      </button>
      <button
        class:active={fit === "page"}
        aria-pressed={fit === "page"}
        onclick={() => viewer?.fitPage()}
        title="Fit page, follows the window size (Ctrl 2)"
      >
        Fit page
      </button>
      <span class="sep"></span>
      <label>
        <span class="visually-hidden">View</span>
        <select
          value={mode}
          onchange={(e) => viewer?.setMode(e.currentTarget.value as ViewMode)}
          title="Page layout"
        >
          <option value="continuous">Continuous</option>
          <option value="single">Single page</option>
          <option value="two-page">Two pages</option>
        </select>
      </label>
      {#if mode === "two-page"}
        <button
          class:active={rtl}
          aria-pressed={rtl}
          onclick={() => viewer?.setRtl(!rtl)}
          title="Right to left: first page of each spread on the right, as in Arabic books"
        >
          RTL
        </button>
      {/if}
      <span class="sep"></span>
      <button
        onclick={() => viewer?.rotate(-1)}
        title="Rotate view left (Ctrl Shift -)"
        aria-label="Rotate left"
      >
        ⟲
      </button>
      <button
        onclick={() => viewer?.rotate(1)}
        title="Rotate view right (Ctrl Shift +)"
        aria-label="Rotate right"
      >
        ⟳
      </button>
    {:else}
      <button onclick={openDialog} title="Open a PDF (Ctrl O)">Open</button>
      <span class="name">edits</span>
    {/if}
  </header>
  <div class="body">
    {#if viewer && active && sidebarOpen}
      {#key active.id}
        <Sidebar {viewer} {page} outline={active.outline} panel={sidebarPanel} onpanel={setPanel} />
      {/key}
    {/if}
    <main onwheel={onWheel}>
      {#each tabs as t (t.id)}
        <div class="doc" hidden={t.id !== activeId} bind:this={hosts[t.id]}></div>
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
          <button onclick={() => (error = "")} aria-label="Dismiss">×</button>
        </div>
      {/if}
      {#if pwPrompt}
        <div class="overlay">
          <form class="password" onsubmit={submitPassword}>
            <p>
              <strong>{pwPrompt.path ? nameOf(pwPrompt.path) : nameOf(initialPath ?? "")}</strong> is protected
              by a password.
            </p>
            <label>
              Password
              <input type="password" bind:this={pwField} bind:value={password} autocomplete="off" />
            </label>
            {#if pwPrompt.wrong}<p class="error" role="alert">
                That password is not correct. Try again.
              </p>{/if}
            <div class="actions">
              <button type="submit">Open</button>
              <button type="button" onclick={() => (pwPrompt = null)}>Cancel</button>
            </div>
          </form>
        </div>
      {:else if !tabs.length}
        <div class="start">
          <div class="start-inner">
            <p>
              <button class="primary" onclick={openDialog}>Open a PDF</button>
              <span class="hint">or press Ctrl O, or drop files here</span>
            </p>
            {#if recent.length}
              <h2>Recent</h2>
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
                      aria-label="Remove {r.name} from recent files"
                      title="Remove from this list"
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
    background: #151517;
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
    background: #222226;
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
    background: #3d3d44;
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
    border-bottom: 1px solid #000;
    user-select: none;
    white-space: nowrap;
    overflow-x: auto;
    flex: none;
  }
  .name {
    font-weight: 600;
  }
  .sep {
    flex: none;
    width: 1px;
    height: 18px;
    background: #444;
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
    background: #333338;
    color: var(--text);
    border: 1px solid #444;
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
    background: #3d3d44;
  }
  button.active {
    border-color: var(--accent);
    background: #2d3f55;
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
    background: rgb(0 0 0 / 0.4);
    z-index: 10;
  }
  .password {
    display: grid;
    gap: 12px;
    min-width: 320px;
    padding: 20px 24px;
    background: var(--bar);
    border: 1px solid #444;
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
    color: #ff8a80;
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
    background: #3a1f1d;
    border: 1px solid #7a2e27;
    border-radius: 6px;
    color: #ffd2cc;
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
    background: #5b9de0 !important;
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
    background: #333338;
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
