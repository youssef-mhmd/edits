// Calls into the Rust side.

import { invoke } from "@tauri-apps/api/core";

export interface PageSize {
  w: number;
  h: number;
}

export interface DocInfo {
  doc: number;
  page_count: number;
  pages: PageSize[];
  /** Pages whose size could not be read; they show at a substitute size. */
  broken_pages: number[];
  open_us: number;
}

export interface RenderSpec {
  doc: number;
  page: number;
  page_w: number;
  page_h: number;
  x: number;
  y: number;
  w: number;
  h: number;
  /** View rotation, clockwise quarter turns (0 to 3). */
  rotation: number;
}

export interface Startup {
  file: string | null;
  bench: boolean;
  transport: "shm" | "protocol" | "ipc";
  main_unix_ms: number;
  setup_unix_ms: number;
}

export interface TileData {
  w: number;
  h: number;
  rgba: Uint8ClampedArray<ArrayBuffer>;
  renderUs: number;
  queuedUs: number;
  /** Request start until the pixels are in JS memory. */
  totalMs: number;
  /** Frees `rgba` early when it is borrowed (shared buffer). Do not use `rgba` after. */
  release?: () => void;
}

export const startup = () => invoke<Startup>("startup");
/** Opens `path`, or the file from the command line when `path` is null. */
export const openDoc = (path: string | null, password: string | null = null) =>
  invoke<DocInfo>("open_doc", { path, password });

/** True when an open failed because the file needs a (different) password. */
export const needsPassword = (e: unknown) => String(e).includes("needs a password");
export const closeDoc = (doc: number) => invoke<void>("close_doc", { doc });
export const setViewport = (doc: number, keep: [number, number][], keepWidths: number[]) =>
  invoke<number>("set_viewport", { doc, keep, keepWidths });
export const benchReport = (report: unknown) => invoke<void>("bench_report", { report });

/** Where a bookmark or link goes. Positions are page fractions, 0 = top. */
export type Target = { kind: "page"; page: number; y: number | null } | { kind: "uri"; uri: string };

export interface OutlineItem {
  title: string;
  target: Target | null;
  children: OutlineItem[];
}

export interface Link {
  /** [left, top, right, bottom] as fractions of the unrotated page. */
  rect: [number, number, number, number];
  target: Target;
}

export const outline = (doc: number) => invoke<OutlineItem[]>("outline", { doc });
export const pageLinks = (doc: number, page: number) => invoke<Link[]>("page_links", { doc, page });
export const loadState = () => invoke<unknown>("load_state");
export const saveState = (state: unknown) => invoke<void>("save_state", { state });

const tileBase = navigator.userAgent.includes("Windows") ? "http://tile.localhost" : "tile://localhost";

/** Thrown when the engine dropped the request because the view moved on. */
export class Cancelled extends Error {}

async function viaProtocol(s: RenderSpec, priority: number): Promise<TileData> {
  const t0 = performance.now();
  const url = `${tileBase}/${s.doc}/${s.page}/${s.page_w}/${s.page_h}/${s.x}/${s.y}/${s.w}/${s.h}/${s.rotation}?p=${priority}`;
  const res = await fetch(url);
  if (res.status === 409) throw new Cancelled();
  if (!res.ok) throw new Error(await res.text());
  const h = res.headers;
  return {
    w: Number(h.get("x-w")),
    h: Number(h.get("x-h")),
    rgba: new Uint8ClampedArray(await res.arrayBuffer()),
    renderUs: Number(h.get("x-render-us")),
    queuedUs: Number(h.get("x-queued-us")),
    totalMs: performance.now() - t0,
  };
}

async function viaIpc(spec: RenderSpec, priority: number): Promise<TileData> {
  const t0 = performance.now();
  let buf: ArrayBuffer;
  try {
    buf = await invoke<ArrayBuffer>("render_tile_ipc", { spec, priority });
  } catch (e) {
    if (String(e) === "cancelled") throw new Cancelled();
    throw e;
  }
  const head = new Uint32Array(buf, 0, 4);
  return {
    w: head[0],
    h: head[1],
    renderUs: head[2],
    queuedUs: head[3],
    rgba: new Uint8ClampedArray(buf, 16),
    totalMs: performance.now() - t0,
  };
}

// WebView2 shared buffers: the tile arrives as a `sharedbufferreceived` event.
interface SharedBufferEvent extends Event {
  getBuffer(): ArrayBuffer;
  additionalData: { id: number; w: number; h: number; render_us: number; queued_us: number };
}
interface WebView2 {
  addEventListener(type: "sharedbufferreceived", cb: (e: SharedBufferEvent) => void): void;
  releaseBuffer(buf: ArrayBuffer): void;
}
const webview2 = (window as unknown as { chrome?: { webview?: WebView2 } }).chrome?.webview;
const pendingShm = new Map<number, { t0: number; resolve: (t: TileData) => void }>();
let nextShmId = 1;

webview2?.addEventListener("sharedbufferreceived", (e) => {
  const meta = e.additionalData;
  const p = pendingShm.get(meta.id);
  const buf = e.getBuffer();
  if (!p) {
    webview2.releaseBuffer(buf);
    return;
  }
  pendingShm.delete(meta.id);
  // Read straight from shared memory; the consumer calls release() after use.
  p.resolve({
    w: meta.w,
    h: meta.h,
    rgba: new Uint8ClampedArray(buf),
    release: () => webview2.releaseBuffer(buf),
    renderUs: meta.render_us,
    queuedUs: meta.queued_us,
    totalMs: performance.now() - p.t0,
  });
});

async function viaShm(spec: RenderSpec, priority: number): Promise<TileData> {
  const id = nextShmId++;
  const t0 = performance.now();
  const done = new Promise<TileData>((resolve) => pendingShm.set(id, { t0, resolve }));
  try {
    await invoke("render_tile_shm", { spec, priority, id });
  } catch (e) {
    pendingShm.delete(id);
    if (String(e) === "cancelled") throw new Cancelled();
    throw e;
  }
  return done;
}

export type Transport = (spec: RenderSpec, priority: number) => Promise<TileData>;

export const transports: Record<Startup["transport"], Transport> = {
  shm: webview2 ? viaShm : viaProtocol,
  protocol: viaProtocol,
  ipc: viaIpc,
};
