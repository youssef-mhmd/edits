<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { closeDoc, openDoc, startup, transports, type Transport } from "./api";
  import { Viewer } from "./viewer";
  import { benchNoFile, benchWithFile } from "./bench";

  let host: HTMLElement;
  let viewer: Viewer | null = $state(null);
  let page = $state(1);
  let zoomPct = $state(100);
  let fitting = $state(true);
  let error = $state("");
  let fileName = $state("");
  let transport: Transport = transports.protocol;

  function refresh() {
    if (!viewer) return;
    page = viewer.currentPage() + 1;
    fitting = viewer.fitsWidth;
    // 100% is actual size: 96 CSS px per inch, 72 points per inch.
    zoomPct = Math.round(viewer.zoom * (72 / 96) * 100);
  }

  async function open(path: string | null) {
    error = "";
    try {
      const info = await openDoc(path);
      if (viewer) {
        void closeDoc(viewer.info.doc);
        viewer.destroy();
      }
      viewer = new Viewer(host, info, transport);
      viewer.onUpdate = refresh;
      if (path) fileName = path.split(/[\\/]/).pop() ?? path;
      return viewer;
    } catch (e) {
      error = String(e);
      return null;
    }
  }

  function zoomBy(k: number) {
    viewer?.setZoom(viewer.zoom * k);
  }

  function onKey(e: KeyboardEvent) {
    if (!viewer) return;
    if (e.ctrlKey && (e.key === "=" || e.key === "+")) {
      e.preventDefault();
      zoomBy(1.25);
    } else if (e.ctrlKey && e.key === "-") {
      e.preventDefault();
      zoomBy(0.8);
    } else if (e.ctrlKey && e.key === "0") {
      e.preventDefault();
      viewer.fitWidth();
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
      if (s.file) fileName = s.file.split(/[\\/]/).pop() ?? s.file;
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
    <span class="name">{fileName || "edits"}</span>
    {#if viewer}
      <span class="sep"></span>
      <span>{page} / {viewer.info.page_count}</span>
      <span class="sep"></span>
      <button onclick={() => zoomBy(0.8)} title="Zoom out (Ctrl -)">−</button>
      <span class="zoom">{zoomPct}%</span>
      <button onclick={() => zoomBy(1.25)} title="Zoom in (Ctrl +)">+</button>
      <button
        class:active={fitting}
        aria-pressed={fitting}
        onclick={() => viewer?.fitWidth()}
        title="Fit width, follows the window size (Ctrl 0)"
      >
        Fit width
      </button>
    {/if}
  </header>
  <main bind:this={host} onwheel={onWheel}>
    {#if !viewer}
      <div class="empty">
        {#if error}<p class="error">{error}</p>{/if}
        <p>Drop a PDF here, or start with <code>edits file.pdf</code>.</p>
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
    height: 36px;
    padding: 0 12px;
    background: var(--bar);
    border-bottom: 1px solid #000;
    user-select: none;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 40ch;
  }
  .sep {
    width: 1px;
    height: 18px;
    background: #444;
  }
  .zoom {
    min-width: 4ch;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  button {
    background: #333338;
    color: var(--text);
    border: 1px solid #444;
    border-radius: 4px;
    padding: 2px 8px;
    font: inherit;
    cursor: pointer;
  }
  button:hover {
    background: #3d3d44;
  }
  button.active {
    border-color: var(--accent);
    background: #2d3f55;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
  }
  main {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    text-align: center;
    color: var(--muted);
  }
  .error {
    color: #ff8a80;
  }
</style>
