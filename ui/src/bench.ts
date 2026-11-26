// Bench mode script (`edits --bench-out FILE [PDF]`). Measures what PRD
// section 7 asks for, as seen in the real window, and reports to Rust.

import { benchReport, type RenderSpec, type Startup, type TileData, type Transport } from "./api";
import type { Viewer } from "./viewer";

const now = () => performance.timeOrigin + performance.now();
const frame = () => new Promise<number>((r) => requestAnimationFrame(r));

/** Waits until `cond` holds, checking every frame. Returns ms waited, or null on timeout. */
async function until(cond: () => boolean, timeoutMs: number): Promise<number | null> {
  const t0 = performance.now();
  while (!cond()) {
    if (performance.now() - t0 > timeoutMs) return null;
    await frame();
  }
  return performance.now() - t0;
}

const allSharp = (v: Viewer) => v.pagesInView(0).every((i) => v.isSharp(i));

/** Where start-up time goes, all in Unix ms. */
function startupTimes(s: Startup, windowReady: number) {
  const nav = performance.getEntriesByType("navigation")[0] as PerformanceNavigationTiming | undefined;
  return {
    window_ready_unix_ms: windowReady,
    main_unix_ms: s.main_unix_ms,
    setup_unix_ms: s.setup_unix_ms,
    navigation_start_unix_ms: performance.timeOrigin,
    dom_content_loaded_unix_ms: performance.timeOrigin + (nav?.domContentLoadedEventEnd ?? 0),
  };
}

export async function benchNoFile(s: Startup, windowReady: number) {
  await benchReport(startupTimes(s, windowReady));
}

async function transferBench(v: Viewer, transport: Transport) {
  const base: RenderSpec = {
    doc: v.info.doc,
    page: 0,
    page_w: 1024,
    page_h: 1024,
    x: 0,
    y: 0,
    w: 16,
    h: 16,
    rotation: 0,
  };
  const seq = async (spec: RenderSpec, n: number) => {
    const total: number[] = [];
    const render: number[] = [];
    for (let i = 0; i < n; i++) {
      const t = await transport(spec, 5000);
      total.push(t.totalMs);
      render.push(t.renderUs / 1000);
    }
    return { total, render };
  };
  const small = await seq(base, 30);
  const big = await seq({ ...base, w: 512, h: 512 }, 30);
  const t0 = performance.now();
  await Promise.all(
    Array.from({ length: 12 }, (_, i) => transport({ ...base, x: (i % 2) * 512, w: 512, h: 512 }, 5000)),
  );
  return { small, big, parallel12_ms: performance.now() - t0 };
}

export async function benchWithFile(s: Startup, windowReady: number, v: Viewer, transport: Transport) {
  const report: Record<string, unknown> = { ...startupTimes(s, windowReady), transport: s.transport };
  const tiles: { render: number[]; queued: number[]; total: number[] } = {
    render: [],
    queued: [],
    total: [],
  };
  v.onTile = (t: TileData) => {
    tiles.render.push(t.renderUs / 1000);
    tiles.queued.push(t.queuedUs / 1000);
    tiles.total.push(t.totalMs);
  };
  report.tiles = tiles;
  try {
    // 1. First page sharp.
    if ((await until(() => allSharp(v), 10_000)) === null) throw new Error("first page never became sharp");
    report.first_page_unix_ms = now();

    // 2. Page-down steps: time until everything in view is sharp.
    const sharp: number[] = [];
    for (let step = 0; step < 20; step++) {
      v.scroller.scrollTop += v.scroller.clientHeight * 0.9;
      await frame();
      const t = await until(() => allSharp(v), 2000);
      if (t !== null) sharp.push(t);
    }
    report.sharp_after_scroll_ms = sharp;

    // 3. Continuous scroll at 3,000 CSS px/s for 3 s from the top, turning
    //    back at the end of the document: frame times and blank pages.
    v.goToPage(0);
    await until(() => allSharp(v), 2000);
    const frameMs: number[] = [];
    let blank = 0;
    let dir = 1;
    let last = await frame();
    const start = last;
    const max = v.scroller.scrollHeight - v.scroller.clientHeight;
    while (last - start < 3000) {
      const t = await frame();
      frameMs.push(t - last);
      let y = v.scroller.scrollTop + (dir * (t - last) * 3000) / 1000;
      if (y >= max || y <= 0) {
        dir = -dir;
        y = Math.min(max, Math.max(0, y));
      }
      v.scroller.scrollTop = y;
      last = t;
      if (v.pagesInView(0).some((i) => !v.hasContent(i))) blank++;
    }
    report.frame_ms = frameMs;
    report.blank_frames = blank;

    // 4. Transfer cost, isolated: fixed overhead (16x16 tile), a 1 MB tile
    //    one at a time, and 12 at once.
    report.transfer = await transferBench(v, transport);

    // 5. Zoom steps on the first page.
    v.goToPage(0);
    v.fitWidth();
    await until(() => allSharp(v), 2000);
    const zoom: number[] = [];
    for (let i = 0; i < 4; i++) {
      v.setZoom(v.zoom * 1.25);
      await frame();
      const t = await until(() => allSharp(v), 3000);
      zoom.push(t ?? 3000);
    }
    report.zoom_sharp_ms = zoom;
  } catch (e) {
    report.error = String(e);
  }
  await benchReport(report);
}
