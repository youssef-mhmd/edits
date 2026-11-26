<script lang="ts">
  import { onMount, tick } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open as pickFile } from "@tauri-apps/plugin-dialog";
  import { closeDoc, needsPassword, openDoc, startup, transports, type Transport } from "./api";
  import { Viewer, type FitMode, type ViewMode } from "./viewer";
  import { benchNoFile, benchWithFile } from "./bench";

  let host: HTMLElement;
  let viewer: Viewer | null = $state(null);
  let page = $state(1);
  let pageInput = $state("1");
  let editingPage = $state(false);
  let zoomPct = $state(100);
  let fit: FitMode = $state("width");
  let mode: ViewMode = $state("continuous");
  let rtl = $state(false);
  let error = $state("");
  let fileName = $state("");
  // Set while asking for a password; `path` null means the launch file.
  let pwPrompt: { path: string | null; wrong: boolean } | null = $state(null);
  let password = $state("");
  let pwField: HTMLInputElement | null = $state(null);
  let transport: Transport = transports.protocol;

  const nameOf = (path: string) => path.split(/[\\/]/).pop() ?? path;

  function refresh() {
    if (!viewer) return;
    page = viewer.currentPage() + 1;
    if (!editingPage) pageInput = String(page);
    fit = viewer.fit;
    mode = viewer.mode;
    rtl = viewer.rtl;
    // 100% is actual size: 96 CSS px per inch, 72 points per inch.
    zoomPct = Math.round(viewer.zoom * (72 / 96) * 100);
  }

  async function open(path: string | null, pw: string | null = null): Promise<Viewer | null> {
    error = "";
    try {
      const info = await openDoc(path, pw);
      if (viewer) {
        void closeDoc(viewer.info.doc);
        viewer.destroy();
      }
      pwPrompt = null;
      password = "";
      viewer = new Viewer(host, info, transport);
      viewer.onUpdate = refresh;
      // For automated UI tests over the DevTools protocol.
      (window as unknown as { __viewer: Viewer }).__viewer = viewer;
      if (path) fileName = nameOf(path);
      viewer.scroller.focus();
      return viewer;
    } catch (e) {
      if (needsPassword(e)) {
        pwPrompt = { path, wrong: pw !== null };
        password = "";
        await tick();
        pwField?.focus();
      } else {
        error = String(e);
      }
      return null;
    }
  }

  async function openDialog() {
    const picked = await pickFile({ multiple: false, filters: [{ name: "PDF", extensions: ["pdf"] }] });
    if (typeof picked === "string") await open(picked);
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
    if (viewer && Number.isFinite(n)) viewer.goToPage(n - 1);
    editingPage = false;
    pageInput = String(page);
    viewer?.scroller.focus();
  }

  function scrollBy(dy: number) {
    viewer?.scroller.scrollBy({ top: dy });
  }

  /** The action for a key press, or null to let the browser handle it. */
  function keyAction(e: KeyboardEvent, v: Viewer): (() => void) | null {
    const pageWise = v.mode !== "continuous";
    const page = (forward: boolean) => () => (forward ? v.nextPage() : v.prevPage());
    const screen = (dir: number) => () => scrollBy(dir * v.scroller.clientHeight * 0.9);
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
        return pageWise ? page(true) : screen(1);
      case "PageUp":
        return pageWise ? page(false) : screen(-1);
      case " ":
        return pageWise ? page(!e.shiftKey) : screen(e.shiftKey ? -1 : 1);
      case "ArrowRight":
      case "ArrowLeft": {
        // In page views without sideways scrolling, arrows turn pages, following
        // the reading direction in right-to-left two-page view.
        if (!pageWise || v.scroller.scrollWidth > v.scroller.clientWidth) return null;
        return page((e.key === "ArrowRight") !== (v.mode === "two-page" && v.rtl));
      }
    }
    return null;
  }

  function onKey(e: KeyboardEvent) {
    if (e.ctrlKey && e.key.toLowerCase() === "o") {
      e.preventDefault();
      void openDialog();
      return;
    }
    const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement;
    if (!viewer || typing) return;
    const action = keyAction(e, viewer);
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

  onMount(() => {
    // Measured as "usable window" in bench mode: first frame after mount.
    let windowReady = 0;
    requestAnimationFrame(() => (windowReady = performance.timeOrigin + performance.now()));

    void (async () => {
      const s = await startup();
      transport = transports[s.transport];
      if (s.file) fileName = nameOf(s.file);
      const v = s.file ? await open(null) : null;
      if (s.bench) {
        while (!windowReady) await new Promise((r) => requestAnimationFrame(r));
        if (v) await benchWithFile(s, windowReady, v, transport);
        else await benchNoFile(s, windowReady);
      }
    })();

    const unlisten = getCurrentWebview().onDragDropEvent((e) => {
      if (e.payload.type === "drop") {
        const pdf = e.payload.paths.find((p) => p.toLowerCase().endsWith(".pdf"));
        if (pdf) void open(pdf);
      }
    });
    return () => void unlisten.then((f) => f());
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="app">
  <header>
    <button onclick={openDialog} title="Open a PDF (Ctrl O)">Open</button>
    <span class="name" title={fileName}>{fileName || "edits"}</span>
    {#if viewer}
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
            pageInput = String(page);
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
    {/if}
  </header>
  <main bind:this={host} onwheel={onWheel}>
    {#if !viewer}
      <div class="empty">
        {#if pwPrompt}
          <form class="password" onsubmit={submitPassword}>
            <p>
              <strong>{pwPrompt.path ? nameOf(pwPrompt.path) : fileName}</strong> is protected by a password.
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
        {:else}
          {#if error}<p class="error" role="alert">{error}</p>{/if}
          <p>Open a PDF with <button onclick={openDialog}>Open</button> or Ctrl O, or drop one here.</p>
        {/if}
      </div>
    {/if}
  </main>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
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
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 32ch;
    min-width: 6ch;
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
  button:hover,
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
  main {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  main :global(.scroller:focus) {
    outline: none;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    text-align: center;
    color: var(--muted);
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
    text-align: start;
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
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
