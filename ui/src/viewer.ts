// The page view: continuous, single-page and two-page layouts, zoom, view
// rotation, a low-resolution preview per page and sharp tiles on top.
// Plain DOM, no framework: this runs on every scroll frame.
//
// Two cache levels (PRD 8.2):
// - Previews: one small render per page, filled in for the whole document in
//   the background and kept as compressed WebP blobs, so a page is never blank
//   even after a jump. A few are kept decoded as canvases.
// - Tiles: 512 device-pixel canvases at the current zoom, bounded by bytes.
//
// Page edges are snapped to whole device pixels and each page is exactly as
// many device pixels as its tiles, so tiles are never resampled (no blur).

import {
  Cancelled,
  pageLinks,
  pageText,
  setViewport,
  type DocInfo,
  type Link,
  type RenderSpec,
  type Target,
  type TileData,
  type Transport,
} from "./api";
import { t as tr } from "./i18n.svelte";
import { caretAt, cleanCopy, lineAt, overText, rangeRects, wordAt, type LoadedText } from "./textgeom";

const TILE = 512; // device pixels
// A page whose 512 px tiles take longer than this to render gets tiles of
// twice the side: each render of a heavy page repeats work for the whole
// page, so fewer, larger tiles finish sooner (edits-bench core does the same).
const SLOW_TILE_US = 40_000;
const PREVIEW_W = 300; // device pixels, unrotated page width
const GAP = 12; // CSS pixels between pages
const MARGIN = 16; // CSS pixels around the document
const TILE_MARGIN = 256; // device pixels rendered beyond the visible edge
const TILE_BUDGET = 24 * 1024 * 1024; // bytes of tile canvases kept off screen
const PREVIEW_CANVAS_BUDGET = 8 * 1024 * 1024; // bytes of decoded previews kept
const PREVIEW_BLOB_BUDGET = 48 * 1024 * 1024; // bytes of compressed previews kept
const BACKGROUND_INFLIGHT = 2; // background preview renders at once
const MAX_PAGE_PX = 30_000; // engine limit; larger pages show the preview only
const PRIO_PREVIEW_VISIBLE = 3000;
const PRIO_PREVIEW_NEAR = 2000;
const PRIO_TILE = 1000;
const PRIO_BACKGROUND = 1;

export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 8;

export type ViewMode = "continuous" | "single" | "two-page";
export type FitMode = "width" | "page" | "none";

/**
 * Least-recently-used cache of canvases, bounded by pixel bytes.
 * The budget is soft: canvases on screen are never evicted, even when they
 * alone exceed it (large window, high zoom), or the page would fall back to
 * its blurry preview.
 */
class CanvasCache {
  private map = new Map<string, HTMLCanvasElement>();
  private bytes = 0;
  constructor(private budget: number) {}

  get(key: string): HTMLCanvasElement | undefined {
    const c = this.map.get(key);
    if (c) {
      this.map.delete(key);
      this.map.set(key, c);
    }
    return c;
  }

  set(key: string, c: HTMLCanvasElement, inUse: (key: string) => boolean) {
    this.map.set(key, c);
    this.bytes += c.width * c.height * 4;
    this.trim(inUse, key);
  }

  /** Evicts least recently used canvases not in use until within budget. */
  trim(inUse: (key: string) => boolean, keep = "") {
    for (const [k, old] of this.map) {
      if (this.bytes <= this.budget) break;
      if (k === keep || inUse(k)) continue;
      this.map.delete(k);
      this.bytes -= old.width * old.height * 4;
      old.remove();
      old.width = old.height = 0; // free the backing store now
    }
  }
}

/** Compressed previews for the whole document, least recently used first out. */
class BlobStore {
  private map = new Map<number, Blob>();
  private bytes = 0;
  constructor(private budget: number) {}

  has(page: number) {
    return this.map.has(page);
  }

  /** Near the budget: stop filling, or far pages would push out near ones forever. */
  get nearlyFull() {
    return this.bytes > this.budget * 0.9;
  }

  get count() {
    return this.map.size;
  }

  get(page: number): Blob | undefined {
    const b = this.map.get(page);
    if (b) {
      this.map.delete(page);
      this.map.set(page, b);
    }
    return b;
  }

  set(page: number, b: Blob) {
    const old = this.map.get(page);
    if (old) this.bytes -= old.size;
    this.map.delete(page);
    this.map.set(page, b);
    this.bytes += b.size;
    for (const [k, v] of this.map) {
      if (this.bytes <= this.budget) break;
      this.map.delete(k);
      this.bytes -= v.size;
    }
  }
}

interface PageView {
  el: HTMLDivElement;
  preview: HTMLCanvasElement | null;
  tiles: Map<string, HTMLCanvasElement>; // key -> canvas attached to el
  error: HTMLDivElement | null;
  links: HTMLElement[];
  /** Selection and search highlights, drawn over the tiles. */
  marks: HTMLDivElement;
}

/** A caret: page index and position between characters of its text. */
export interface Caret {
  page: number;
  i: number;
}

/** A search hit: characters [start, end) of a page's text. */
export interface Hit {
  page: number;
  start: number;
  end: number;
}

const TEXT_CACHE_PAGES = 60;

/** A reading position: page index and how far down it the view's top edge is (0 to 1). */
export interface Position {
  page: number;
  y: number;
}

/** Everything needed to reopen a document where the reader left it. */
export interface ViewState extends Position {
  zoom: number;
  fit: FitMode;
  mode: ViewMode;
  rotation: number;
  rtl: boolean;
}

/** Maps a rectangle in unrotated page fractions to the page turned by `q` quarter turns. */
function rotateRect([l, t, r, b]: number[], q: number): number[] {
  switch (q) {
    case 1:
      return [1 - b, l, 1 - t, r];
    case 2:
      return [1 - r, 1 - b, 1 - l, 1 - t];
    case 3:
      return [t, 1 - r, b, 1 - l];
    default:
      return [l, t, r, b];
  }
}

const MAX_HISTORY = 100;

/** Where a page sits: CSS px from the content's top-left, snapped to device pixels. */
interface Box {
  x: number;
  y: number;
  pw: number; // device px
  ph: number; // device px
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export class Viewer {
  readonly scroller: HTMLDivElement;
  private content: HTMLDivElement;
  private rows: number[][] = []; // page indices per row, in reading order
  private rowTops: number[] = []; // CSS px, for the rows laid out
  private rowOf: number[] = []; // page -> row index
  private boxes = new Map<number, Box>(); // pages laid out
  private views = new Map<number, PageView>();
  private tileCache = new CanvasCache(TILE_BUDGET);
  private previewCanvases = new CanvasCache(PREVIEW_CANVAS_BUDGET);
  private previewBlobs = new BlobStore(PREVIEW_BLOB_BUDGET);
  private inflight = new Set<string>();
  private backgroundInflight = 0;
  private errors = new Map<number, string>();
  /** Pages whose tiles are slow to render; they get larger tiles. */
  private slowPages = new Set<number>();
  private frame = 0;
  private lastKeep = "";
  private _zoom = 1;
  private _mode: ViewMode = "continuous";
  private _fit: FitMode = "width";
  private _rotation = 0;
  private _rtl = false;
  private single = 0; // page shown in single-page mode
  private lastWidth = 0;
  private lastHeight = 0;
  private wheelCooldown = 0;
  private active = true;
  /** Where a hidden tab was, to return to when it is shown. */
  private hiddenAt: Position | null = null;
  private links = new Map<number, Link[] | "loading">();
  private back: Position[] = [];
  private forward: Position[] = [];
  private blobWaiters = new Map<number, ((b: Blob) => void)[]>();
  private texts = new Map<number, LoadedText>();
  private textLoads = new Map<number, Promise<LoadedText>>();
  private sel: { a: Caret; b: Caret } | null = null;
  private dragging = false;
  private hits = new Map<number, [number, number][]>();
  private currentHit: Hit | null = null;
  dpr = window.devicePixelRatio || 1;
  /** Called when the reader clicks a link to a web address. */
  onExternalLink: (uri: string) => void = () => {};
  /** Called when a page's compressed preview becomes available (thumbnails). */
  onPreviewStored: (page: number) => void = () => {};
  /** Called after every update, for the toolbar and the bench script. */
  onUpdate: () => void = () => {};
  /** Called when a tile arrives. */
  onTile: (t: TileData) => void = () => {};

  constructor(
    host: HTMLElement,
    readonly info: DocInfo,
    private transport: Transport,
  ) {
    this.scroller = document.createElement("div");
    this.scroller.className = "scroller";
    this.scroller.tabIndex = 0; // keyboard scrolling works once clicked
    this.content = document.createElement("div");
    this.content.className = "content";
    this.scroller.append(this.content);
    host.append(this.scroller);
    this.scroller.addEventListener("scroll", () => this.schedule(), { passive: true });
    this.scroller.addEventListener("wheel", (e) => this.onWheel(e), { passive: true });
    this.scroller.addEventListener("mousedown", (e) => this.onMouseDown(e));
    this.scroller.addEventListener("mousemove", (e) => this.onHover(e));
    window.addEventListener("mousemove", this.onDrag);
    window.addEventListener("mouseup", this.onMouseUp);
    new ResizeObserver(() => this.onResize()).observe(this.scroller);
    this.buildRows();
    this.refit(0, 0);
  }

  destroy() {
    cancelAnimationFrame(this.frame);
    this.active = false;
    window.removeEventListener("mousemove", this.onDrag);
    window.removeEventListener("mouseup", this.onMouseUp);
    this.scroller.remove();
  }

  /**
   * Hidden tabs stop rendering: queued renders are cancelled and nothing new
   * is requested until the tab is shown again.
   */
  setActive(active: boolean) {
    if (active === this.active) return;
    this.active = active;
    if (!active) {
      this.hiddenAt = this.position();
      cancelAnimationFrame(this.frame);
      this.frame = 0;
      this.lastKeep = "";
      void setViewport(this.info.doc, [], []);
    } else {
      // The window may have changed size while hidden: fit again, then go
      // back to exactly where the reader was.
      const at = this.hiddenAt;
      this.hiddenAt = null;
      this.lastWidth = this.scroller.clientWidth;
      this.lastHeight = this.scroller.clientHeight;
      this.refit(0, 0);
      if (at) this.goTo(at.page, at.y);
      this.schedule();
    }
  }

  // ------------------------------------------------------------ positions

  /** Where the top edge of the view is. */
  position(): Position {
    if (this.hiddenAt) return this.hiddenAt;
    const page = this.currentPage();
    const b = this.boxes.get(page);
    if (!b) return { page, y: 0 };
    const y = (this.scroller.scrollTop - b.y) / (b.ph / this.dpr);
    return { page, y: Math.min(1, Math.max(0, y)) };
  }

  /** Scrolls so that `y` (0 = top) of page `page` is at the top of the view. */
  goTo(page: number, y = 0) {
    this.goToPage(page);
    const b = this.boxes.get(page);
    if (b && y > 0) this.scroller.scrollTop = b.y + y * (b.ph / this.dpr) - MARGIN;
    this.schedule();
  }

  /** Like `goTo`, and remembered so Back returns here. */
  navigate(page: number, y = 0) {
    this.back.push(this.position());
    if (this.back.length > MAX_HISTORY) this.back.shift();
    this.forward = [];
    this.goTo(page, y);
  }

  /** Follows a bookmark or link target. */
  follow(t: Target) {
    if (t.kind === "uri") {
      this.onExternalLink(t.uri);
      return;
    }
    // A vertical position survives a half turn; quarter turns lose it.
    const y = t.y === null ? 0 : this._rotation === 0 ? t.y : this._rotation === 2 ? 1 - t.y : 0;
    this.navigate(t.page, Math.max(0, y - 0.02));
  }

  get canGoBack() {
    return this.back.length > 0;
  }
  get canGoForward() {
    return this.forward.length > 0;
  }

  goBack() {
    const p = this.back.pop();
    if (!p) return;
    this.forward.push(this.position());
    this.goTo(p.page, p.y);
  }

  goForward() {
    const p = this.forward.pop();
    if (!p) return;
    this.back.push(this.position());
    this.goTo(p.page, p.y);
  }

  viewState(): ViewState {
    return {
      ...this.position(),
      zoom: this._zoom,
      fit: this._fit,
      mode: this._mode,
      rotation: this._rotation,
      rtl: this._rtl,
    };
  }

  /** Restores a saved view; values that do not fit this document are ignored. */
  restore(s: Partial<ViewState>) {
    if (s.mode === "continuous" || s.mode === "single" || s.mode === "two-page") this._mode = s.mode;
    if (typeof s.rotation === "number") this._rotation = ((Math.round(s.rotation) % 4) + 4) % 4;
    if (typeof s.rtl === "boolean") this._rtl = s.rtl;
    if (s.fit === "width" || s.fit === "page" || s.fit === "none") this._fit = s.fit;
    if (this._fit === "none" && typeof s.zoom === "number" && Number.isFinite(s.zoom)) {
      this._zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, s.zoom));
    }
    const page = typeof s.page === "number" ? Math.max(0, Math.min(this.info.page_count - 1, s.page)) : 0;
    this.single = page;
    this.buildRows();
    this.layout();
    const z = this.fitZoom();
    if (z !== null) this._zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
    this.layout();
    this.goTo(page, typeof s.y === "number" ? Math.min(1, Math.max(0, s.y)) : 0);
    if (this._fit !== "none") {
      this.scroller.scrollLeft = (this.scroller.scrollWidth - this.scroller.clientWidth) / 2;
    }
  }

  // ------------------------------------------------------------ thumbnails

  /** The compressed preview of page `i`, rendering it first if needed. */
  previewBlob(i: number): Promise<Blob> {
    const b = this.previewBlobs.get(i);
    if (b && b.size > 0) return Promise.resolve(b);
    return new Promise((resolve) => {
      const list = this.blobWaiters.get(i) ?? [];
      list.push(resolve);
      this.blobWaiters.set(i, list);
      const key = `p${i}`;
      if (!this.inflight.has(key) && !this.errors.has(i)) {
        this.request(key, this.previewSpec(i), PRIO_PREVIEW_NEAR, (c) => {
          const show = this.views.has(i) && !this.views.get(i)!.preview;
          if (show) this.keepPreview(i, c);
          this.compress(i, c, !show);
        });
      }
    });
  }

  // ---------------------------------------------------------------- state

  get zoom() {
    return this._zoom;
  }
  get mode() {
    return this._mode;
  }
  get fit() {
    return this._fit;
  }
  get rotation() {
    return this._rotation;
  }
  get rtl() {
    return this._rtl;
  }
  /** True while the zoom follows the window width (kept for the bench script). */
  get fitsWidth() {
    return this._fit === "width";
  }
  get pageCount() {
    return this.info.page_count;
  }

  setMode(mode: ViewMode) {
    if (mode === this._mode) return;
    const page = this.currentPage();
    this._mode = mode;
    this.single = page;
    this.buildRows();
    this.relayoutKeeping(page);
  }

  /** Two-page view: put the first page of each spread on the right (Arabic books). */
  setRtl(rtl: boolean) {
    this._rtl = rtl;
    this.layout();
    this.schedule();
  }

  /** Turns the view by quarter turns, clockwise for positive `by`. */
  rotate(by: number) {
    const page = this.currentPage();
    this._rotation = (((this._rotation + by) % 4) + 4) % 4;
    this.relayoutKeeping(page);
  }

  fitWidth() {
    this._fit = "width";
    this.refit(0, 0);
    this.scroller.scrollLeft = (this.scroller.scrollWidth - this.scroller.clientWidth) / 2;
  }

  fitPage() {
    this._fit = "page";
    const page = this.currentPage();
    this.refit(0, 0);
    this.goToPage(page);
  }

  /** Sets a manual zoom, keeping the point at (`ax`, `ay`) in the viewport still. */
  setZoom(z: number, ax = this.scroller.clientWidth / 2, ay = this.scroller.clientHeight / 2) {
    this._fit = "none";
    this.applyZoom(z, ax, ay);
  }

  goToPage(i: number) {
    i = Math.max(0, Math.min(i, this.info.page_count - 1));
    if (this._mode === "single") {
      this.single = i;
      this.buildRows();
      this.layout();
      this.scroller.scrollTop = 0;
    } else {
      this.scroller.scrollTop = this.rowTops[this.rowOf[i]] - MARGIN;
    }
    this.schedule();
  }

  nextPage() {
    if (this._mode === "single") return this.goToPage(this.single + 1);
    const row = this.rowOf[this.currentPage()];
    if (row + 1 < this.rows.length) this.goToPage(this.rows[row + 1][0]);
  }

  prevPage() {
    if (this._mode === "single") return this.goToPage(this.single - 1);
    const row = this.rowOf[this.currentPage()];
    const top = this.rowTops[row] - MARGIN;
    // Part-way down a row: go to its top first.
    if (this.scroller.scrollTop > top + 2) this.goToPage(this.rows[row][0]);
    else if (row > 0) this.goToPage(this.rows[row - 1][0]);
  }

  /** Index of the page the reader is on: the row at the top third of the view. */
  currentPage(): number {
    if (this._mode === "single") return this.single;
    if (!this.rowTops.length) return 0;
    return this.rows[this.rowAt(this.scroller.scrollTop + this.scroller.clientHeight / 3)][0];
  }

  // --------------------------------------------------------------- layout

  /** Page size on screen in points, after view rotation. */
  private shown(i: number): { w: number; h: number } {
    const p = this.info.pages[i];
    return this._rotation % 2 ? { w: p.h, h: p.w } : p;
  }

  /** Device-pixel size of page `i` at the current zoom and rotation. */
  pagePx(i: number): [number, number] {
    const p = this.shown(i);
    const s = this._zoom * this.dpr;
    return [Math.max(1, Math.round(p.w * s)), Math.max(1, Math.round(p.h * s))];
  }

  private buildRows() {
    const n = this.info.page_count;
    if (this._mode === "two-page") {
      this.rows = [];
      for (let i = 0; i < n; i += 2) this.rows.push(i + 1 < n ? [i, i + 1] : [i]);
    } else if (this._mode === "single") {
      this.rows = [[Math.max(0, Math.min(this.single, n - 1))]];
    } else {
      this.rows = Array.from({ length: n }, (_, i) => [i]);
    }
    this.rowOf = [];
    if (this._mode === "single") {
      for (let i = 0; i < n; i++) this.rowOf[i] = 0;
    } else {
      this.rows.forEach((r, k) => r.forEach((i) => (this.rowOf[i] = k)));
    }
  }

  /** Width of a row in points at zoom 1 (for fitting). */
  private rowWidthPt(row: number[]): number {
    return row.reduce((s, i) => s + this.shown(i).w, 0);
  }

  /**
   * Positions every row and page at the current zoom. Rows are centred;
   * pages within a two-page row sit side by side, right to left if `rtl`.
   */
  private layout() {
    const d = this.dpr;
    const gap = Math.round(GAP * d);
    let widest = 0;
    const rowPx = this.rows.map((row) => {
      const sizes = row.map((i) => this.pagePx(i));
      const w = sizes.reduce((s, [pw]) => s + pw, 0) + gap * (row.length - 1);
      const h = Math.max(...sizes.map(([, ph]) => ph));
      widest = Math.max(widest, w);
      return { sizes, w, h };
    });
    const contentW = Math.max(widest / d + 2 * MARGIN, this.scroller.clientWidth);
    let y = Math.round(MARGIN * d);
    this.boxes.clear();
    this.rowTops = rowPx.map(({ sizes, w, h }, k) => {
      const top = y / d;
      let x = Math.round(((contentW - w / d) / 2) * d);
      const order = this._rtl ? [...this.rows[k].keys()].reverse() : [...this.rows[k].keys()];
      for (const j of order) {
        const [pw, ph] = sizes[j];
        // Shorter page in a spread: centre it vertically.
        const dy = Math.round((h - ph) / 2);
        this.boxes.set(this.rows[k][j], { x: x / d, y: (y + dy) / d, pw, ph });
        x += pw + gap;
      }
      y += h + gap;
      return top;
    });
    this.content.style.height = `${(y - gap) / d + MARGIN}px`;
    this.content.style.width = `${contentW}px`;
    for (const [i, v] of this.views) {
      if (this.boxes.has(i)) this.place(i, v);
    }
  }

  private place(i: number, v: PageView) {
    this.placeLinks(i, v);
    this.drawMarks(i);
    const b = this.boxes.get(i)!;
    const st = v.el.style;
    st.left = `${b.x}px`;
    st.top = `${b.y}px`;
    st.width = `${b.pw / this.dpr}px`;
    st.height = `${b.ph / this.dpr}px`;
    if (v.preview) this.orientPreview(v.preview, b);
  }

  /** Zoom for the current fit mode, from the current page's row. */
  private fitZoom(): number | null {
    const row = this.rows[this.rowOf[this.currentPage()]] ?? this.rows[0];
    const availW = Math.max(100, this.scroller.clientWidth - 2 * MARGIN - GAP * (row.length - 1));
    const wz = availW / this.rowWidthPt(row);
    if (this._fit === "width") return wz;
    if (this._fit === "page") {
      const availH = Math.max(100, this.scroller.clientHeight - 2 * MARGIN);
      const h = Math.max(...row.map((i) => this.shown(i).h));
      return Math.min(wz, availH / h);
    }
    return null;
  }

  private refit(ax: number, ay: number) {
    const z = this.fitZoom();
    if (z !== null) this.applyZoom(z, ax, ay);
    else {
      this.layout();
      this.schedule();
    }
  }

  /** After a change of mode or rotation: lay out again and stay on `page`. */
  private relayoutKeeping(page: number) {
    if (this._mode === "single") this.single = page;
    this.buildRows();
    // Fit against the page we will be on.
    this.layout();
    const z = this.fitZoom();
    if (z !== null) this._zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
    this.layout();
    this.goToPage(page);
    if (this._fit !== "none") {
      this.scroller.scrollLeft = (this.scroller.scrollWidth - this.scroller.clientWidth) / 2;
    }
  }

  private applyZoom(z: number, ax: number, ay: number) {
    z = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));
    const k = z / this._zoom;
    const sx = (this.scroller.scrollLeft + ax) * k - ax;
    const sy = (this.scroller.scrollTop + ay) * k - ay;
    this._zoom = z;
    this.layout();
    this.scroller.scrollLeft = sx;
    this.scroller.scrollTop = sy;
    this.schedule();
  }

  /** Window resized, or moved to a display with different scaling. */
  private onResize() {
    // A hidden tab reports a zero size; fitting to that would lose the place.
    if (!this.active || this.scroller.clientWidth === 0) return;
    const dpr = window.devicePixelRatio || 1;
    const dprChanged = dpr !== this.dpr;
    this.dpr = dpr;
    const w = this.scroller.clientWidth;
    const h = this.scroller.clientHeight;
    const fitDependsOnHeight = this._fit === "page";
    if (w !== this.lastWidth || dprChanged || (fitDependsOnHeight && h !== this.lastHeight)) {
      this.lastWidth = w;
      this.lastHeight = h;
      this.refit(0, 0);
    }
    this.schedule();
  }

  /** Single-page view: the wheel turns the page at the top and bottom edges. */
  private onWheel(e: WheelEvent) {
    if (this._mode !== "single" || e.ctrlKey) return;
    const s = this.scroller;
    const now = performance.now();
    if (now < this.wheelCooldown) return;
    const atBottom = s.scrollTop + s.clientHeight >= s.scrollHeight - 1;
    const atTop = s.scrollTop <= 0;
    if (e.deltaY > 0 && atBottom && this.single < this.info.page_count - 1) {
      this.goToPage(this.single + 1);
      this.wheelCooldown = now + 250;
    } else if (e.deltaY < 0 && atTop && this.single > 0) {
      this.goToPage(this.single - 1);
      s.scrollTop = s.scrollHeight;
      this.wheelCooldown = now + 250;
    }
  }

  private rowAt(y: number): number {
    let lo = 0;
    let hi = this.rowTops.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.rowTops[mid] <= y) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  /** Pages intersecting the viewport vertically, extended by `extra` CSS px. */
  pagesInView(extra = 0): number[] {
    const top = this.scroller.scrollTop - extra;
    const bottom = this.scroller.scrollTop + this.scroller.clientHeight + extra;
    const out: number[] = [];
    for (let r = this.rowAt(Math.max(0, top)); r < this.rows.length && this.rowTops[r] < bottom; r++) {
      for (const i of this.rows[r]) {
        const b = this.boxes.get(i);
        if (b && b.y + b.ph / this.dpr > top && b.y < bottom) out.push(i);
      }
    }
    return out;
  }

  /** Visible part of page `i`, in its device pixels, grown by `margin`. */
  private visibleRect(i: number, margin: number): Rect | null {
    const b = this.boxes.get(i);
    if (!b) return null;
    const left = (this.scroller.scrollLeft - b.x) * this.dpr - margin;
    const top = (this.scroller.scrollTop - b.y) * this.dpr - margin;
    const x0 = Math.max(0, left);
    const y0 = Math.max(0, top);
    const x1 = Math.min(b.pw, left + this.scroller.clientWidth * this.dpr + 2 * margin);
    const y1 = Math.min(b.ph, top + this.scroller.clientHeight * this.dpr + 2 * margin);
    return x1 > x0 && y1 > y0 ? { x: x0, y: y0, w: x1 - x0, h: y1 - y0 } : null;
  }

  /** Tile side for page `i`, in device pixels. */
  private tileSide(i: number): number {
    return this.slowPages.has(i) ? 2 * TILE : TILE;
  }

  private tileSpecs(i: number, margin: number): RenderSpec[] {
    const [pw, ph] = this.pagePx(i);
    if (pw > MAX_PAGE_PX || ph > MAX_PAGE_PX || this.errors.has(i)) return [];
    const r = this.visibleRect(i, margin);
    if (!r) return [];
    const specs: RenderSpec[] = [];
    const rotation = this._rotation;
    const side = this.tileSide(i);
    for (let y = Math.floor(r.y / side) * side; y < r.y + r.h; y += side) {
      for (let x = Math.floor(r.x / side) * side; x < r.x + r.w; x += side) {
        specs.push({ doc: this.info.doc, page: i, page_w: pw, page_h: ph, x, y, w: side, h: side, rotation });
      }
    }
    return specs;
  }

  private key = (s: RenderSpec) => `${s.page}/${s.page_w}/${s.rotation}/${s.x}/${s.y}/${s.w}`;

  /** Previews are rendered unrotated; the canvas is turned to fit the page. */
  private previewSpec(i: number): RenderSpec {
    const p = this.info.pages[i];
    const w = PREVIEW_W;
    const h = Math.max(1, Math.min(4096, Math.round((w * p.h) / p.w)));
    return { doc: this.info.doc, page: i, page_w: w, page_h: h, x: 0, y: 0, w, h, rotation: 0 };
  }

  /** True when page `i` has a preview, tiles or an error message to show. */
  hasContent(i: number): boolean {
    const v = this.views.get(i);
    return !!v && (v.preview !== null || v.tiles.size > 0 || v.error !== null);
  }

  /** True when every visible tile of page `i` at the current zoom is shown. */
  isSharp(i: number): boolean {
    const v = this.views.get(i);
    if (!v) return false;
    return this.tileSpecs(i, 0).every((s) => (v.tiles.get(this.key(s))?.width ?? 0) > 0);
  }

  // ---------------------------------------------------------------- update

  schedule() {
    if (!this.frame) {
      this.frame = requestAnimationFrame(() => {
        this.frame = 0;
        this.update();
      });
    }
  }

  private update() {
    if (!this.active) return;
    const near = new Set(this.pagesInView(this.scroller.clientHeight));
    const visible = this.pagesInView(0);

    for (const [i, v] of this.views) {
      if (!near.has(i)) {
        v.el.remove();
        this.views.delete(i);
      }
    }
    // Tiles out of view go back to the cache, where they can be evicted:
    // canvases cost GPU memory in the WebView, so only what is in view (and
    // a small margin) stays attached. Previews stay.
    let detached = false;
    for (const [i, v] of this.views) {
      if (!visible.includes(i)) detached = this.detachTiles(v, null) || detached;
    }
    for (const i of near) {
      if (!this.views.has(i)) {
        const el = document.createElement("div");
        el.className = "page";
        const marks = document.createElement("div");
        marks.className = "marks";
        el.append(marks);
        const v: PageView = { el, preview: null, tiles: new Map(), error: null, links: [], marks };
        this.views.set(i, v);
        this.place(i, v);
        this.content.append(el);
        const err = this.errors.get(i);
        if (err) this.showError(v, err);
        this.loadLinks(i);
        if (this.pageHasMarks(i)) void this.loadText(i).then(() => this.drawMarks(i));
      }
    }

    const keep: [number, number][] = [];
    const center = this.currentPage();

    // Previews first: they guarantee a page is never blank.
    for (const i of near) {
      const v = this.views.get(i)!;
      if (v.preview || this.errors.has(i)) continue;
      const prio = visible.includes(i) ? PRIO_PREVIEW_VISIBLE : PRIO_PREVIEW_NEAR - Math.abs(i - center);
      this.loadPreview(i, prio);
    }

    // Sharp tiles for what is in view.
    for (const i of visible) {
      const v = this.views.get(i)!;
      const specs = this.tileSpecs(i, TILE_MARGIN);
      const [pw] = this.pagePx(i);
      keep.push([i, pw]);
      specs.forEach((s, n) => {
        const key = this.key(s);
        if (v.tiles.has(key)) return;
        const cached = this.tileCache.get(key);
        if (cached) {
          this.attachTile(v, key, s, cached);
          return;
        }
        this.request(key, s, PRIO_TILE - Math.abs(i - center) * 50 - n, (c) => {
          this.tileCache.set(key, c, (k) => this.isAttached(k));
          // Attach only if still wanted: the view may have moved on.
          const view = this.views.get(i);
          if (view && this.tileSpecs(i, TILE_MARGIN).some((t) => this.key(t) === key)) {
            this.attachTile(view, key, s, c);
          }
        });
      });
      this.dropStaleTiles(i);
      detached = this.detachTiles(v, new Set(specs.map(this.key))) || detached;
    }
    if (detached) this.tileCache.trim((k) => this.isAttached(k));

    this.fillPreviewsInBackground(center);

    // Tell the engine which renders still matter; it cancels the rest.
    // Previews (unrotated, PREVIEW_W wide) are always kept.
    const keepKey = JSON.stringify(keep);
    if (keepKey !== this.lastKeep) {
      this.lastKeep = keepKey;
      void setViewport(this.info.doc, keep, [PREVIEW_W]);
    }
    this.onUpdate();
  }

  // -------------------------------------------------------------- previews

  /** Shows page `i`'s preview from memory, from its compressed copy, or renders it. */
  private loadPreview(i: number, prio: number) {
    const key = `p${i}`;
    const cached = this.previewCanvases.get(key);
    if (cached) {
      this.attachPreviewTo(i, cached);
      return;
    }
    const blob = this.previewBlobs.get(i);
    if (blob) {
      if (this.inflight.has(key)) return;
      this.inflight.add(key);
      createImageBitmap(blob)
        .then((bmp) => {
          const c = document.createElement("canvas");
          c.width = bmp.width;
          c.height = bmp.height;
          c.getContext("2d", { alpha: false })!.drawImage(bmp, 0, 0);
          bmp.close();
          this.keepPreview(i, c);
        })
        .catch(() => this.previewBlobs.set(i, new Blob())) // undecodable: render again
        .finally(() => {
          this.inflight.delete(key);
          this.schedule();
        });
      return;
    }
    this.request(key, this.previewSpec(i), prio, (c) => {
      this.keepPreview(i, c);
      this.compress(i, c);
    });
  }

  private keepPreview(i: number, c: HTMLCanvasElement) {
    this.previewCanvases.set(`p${i}`, c, (k) => this.views.get(Number(k.slice(1)))?.preview === c);
    this.attachPreviewTo(i, c);
  }

  /**
   * Stores a compressed copy of a preview. With `free`, the canvas is not
   * needed afterwards and its pixel memory (often on the GPU) is released at
   * once rather than whenever garbage collection runs.
   */
  private compress(i: number, c: HTMLCanvasElement, free = false) {
    c.toBlob(
      (b) => {
        if (b && b.size > 0) {
          this.previewBlobs.set(i, b);
          this.blobWaiters.get(i)?.forEach((resolve) => resolve(b));
          this.blobWaiters.delete(i);
          this.onPreviewStored(i);
        }
        if (free) c.width = c.height = 0;
      },
      "image/webp",
      0.8,
    );
  }

  /**
   * Renders previews for pages that have none, nearest to the reader first,
   * a few at a time at the lowest priority. Results are only compressed and
   * stored, so the whole document is covered at little memory cost.
   */
  private fillPreviewsInBackground(center: number) {
    const n = this.info.page_count;
    if (this.previewBlobs.nearlyFull || this.previewBlobs.count + this.errors.size >= n) return;
    for (let d = 0; d < n && this.backgroundInflight < BACKGROUND_INFLIGHT; d++) {
      for (const i of d === 0 ? [center] : [center + d, center - d]) {
        if (i < 0 || i >= n || this.backgroundInflight >= BACKGROUND_INFLIGHT) continue;
        const key = `p${i}`;
        if (this.previewBlobs.has(i) || this.inflight.has(key) || this.errors.has(i)) continue;
        this.backgroundInflight++;
        this.request(
          key,
          this.previewSpec(i),
          PRIO_BACKGROUND,
          (c) => {
            const show = this.views.has(i) && !this.views.get(i)!.preview;
            if (show) this.keepPreview(i, c);
            this.compress(i, c, !show);
          },
          () => this.backgroundInflight--,
        );
      }
    }
  }

  private attachPreviewTo(i: number, c: HTMLCanvasElement) {
    const v = this.views.get(i);
    const b = this.boxes.get(i);
    if (!v || !b || v.preview) return;
    c.className = "preview";
    v.preview = c;
    this.orientPreview(c, b);
    v.el.prepend(c);
  }

  /** Stretches and turns the unrotated preview canvas over the page box. */
  private orientPreview(c: HTMLCanvasElement, b: Box) {
    const w = b.pw / this.dpr;
    const h = b.ph / this.dpr;
    const odd = this._rotation % 2 === 1;
    const st = c.style;
    st.width = `${odd ? h : w}px`;
    st.height = `${odd ? w : h}px`;
    st.left = `${w / 2}px`;
    st.top = `${h / 2}px`;
    st.transform = `translate(-50%, -50%) rotate(${this._rotation * 90}deg)`;
  }

  // ------------------------------------------------------------------ text

  /** Page `i`'s text with character boxes, cached for the pages near the reader. */
  loadText(i: number): Promise<LoadedText> {
    const have = this.texts.get(i);
    if (have) {
      this.texts.delete(i);
      this.texts.set(i, have);
      return Promise.resolve(have);
    }
    let p = this.textLoads.get(i);
    if (!p) {
      p = pageText(this.info.doc, i)
        .then((t) => ({ text: t.text, boxes: Float32Array.from(t.boxes) }))
        .catch(() => ({ text: "", boxes: new Float32Array() }))
        .then((t) => {
          this.texts.set(i, t);
          this.textLoads.delete(i);
          for (const k of this.texts.keys()) {
            if (this.texts.size <= TEXT_CACHE_PAGES) break;
            if (!this.views.has(k)) this.texts.delete(k);
          }
          return t;
        });
      this.textLoads.set(i, p);
    }
    return p;
  }

  /** The page under a point in the window, and the point in unrotated page fractions. */
  private pointToPage(clientX: number, clientY: number): { page: number; fx: number; fy: number } | null {
    const r = this.scroller.getBoundingClientRect();
    const x = clientX - r.left + this.scroller.scrollLeft;
    const y = clientY - r.top + this.scroller.scrollTop;
    let best: { page: number; d: number } | null = null;
    for (const i of this.views.keys()) {
      const b = this.boxes.get(i);
      if (!b) continue;
      const w = b.pw / this.dpr;
      const h = b.ph / this.dpr;
      const dx = x < b.x ? b.x - x : x > b.x + w ? x - b.x - w : 0;
      const dy = y < b.y ? b.y - y : y > b.y + h ? y - b.y - h : 0;
      const d = dx + dy;
      if (!best || d < best.d) best = { page: i, d };
    }
    if (!best) return null;
    const b = this.boxes.get(best.page)!;
    // Displayed fractions, clamped to the page.
    const dxf = Math.min(1, Math.max(0, (x - b.x) / (b.pw / this.dpr)));
    const dyf = Math.min(1, Math.max(0, (y - b.y) / (b.ph / this.dpr)));
    // Undo the view rotation.
    const [fx, fy] =
      this._rotation === 1
        ? [dyf, 1 - dxf]
        : this._rotation === 2
          ? [1 - dxf, 1 - dyf]
          : this._rotation === 3
            ? [1 - dyf, dxf]
            : [dxf, dyf];
    return { page: best.page, fx, fy };
  }

  private caretFor(at: { page: number; fx: number; fy: number }): Caret | null {
    const t = this.texts.get(at.page);
    return t ? { page: at.page, i: caretAt(t, at.fx, at.fy) } : null;
  }

  private onMouseDown(e: MouseEvent) {
    if (e.button !== 0 || (e.target as HTMLElement).closest("a.link")) return;
    const at = this.pointToPage(e.clientX, e.clientY);
    if (!at) return;
    e.preventDefault();
    this.scroller.focus();
    const clicks = e.detail;
    void this.loadText(at.page).then((t) => {
      const c = this.caretFor(at);
      if (!c) return;
      if (clicks >= 2) {
        const [a, z] = clicks === 2 ? wordAt(t, c.i) : lineAt(t, c.i);
        this.setSelection({ page: c.page, i: a }, { page: c.page, i: z });
        return;
      }
      if (e.shiftKey && this.sel) this.setSelection(this.sel.a, c);
      else this.setSelection(c, c);
      this.dragging = true;
    });
  }

  private onDrag = (e: MouseEvent) => {
    if (!this.dragging || !this.sel) return;
    // Scroll while dragging past the edges.
    const r = this.scroller.getBoundingClientRect();
    if (e.clientY < r.top + 24) this.scroller.scrollTop -= 24;
    else if (e.clientY > r.bottom - 24) this.scroller.scrollTop += 24;
    const at = this.pointToPage(e.clientX, e.clientY);
    if (!at) return;
    const c = this.caretFor(at);
    if (c) this.setSelection(this.sel.a, c);
    else void this.loadText(at.page);
  };

  private onMouseUp = () => {
    this.dragging = false;
  };

  /** The text cursor over text, so it is clear what can be selected. */
  private onHover(e: MouseEvent) {
    if (this.dragging) return;
    const at = this.pointToPage(e.clientX, e.clientY);
    const t = at && this.texts.get(at.page);
    if (at && !t) void this.loadText(at.page);
    this.scroller.style.cursor = t && overText(t, at!.fx, at!.fy) ? "text" : "";
  }

  setSelection(a: Caret, b: Caret) {
    const old = this.sel;
    this.sel = { a, b };
    const pages = new Set<number>();
    for (const s of [old, this.sel]) {
      if (!s) continue;
      for (let p = Math.min(s.a.page, s.b.page); p <= Math.max(s.a.page, s.b.page); p++) pages.add(p);
    }
    for (const p of pages) if (this.views.has(p)) this.drawMarks(p);
  }

  clearSelection() {
    const old = this.sel;
    this.sel = null;
    if (old)
      for (let p = Math.min(old.a.page, old.b.page); p <= Math.max(old.a.page, old.b.page); p++)
        this.drawMarks(p);
  }

  /** Selects all text on page `i`. */
  async selectPage(i: number) {
    const t = await this.loadText(i);
    this.setSelection({ page: i, i: 0 }, { page: i, i: t.text.length });
  }

  get hasSelection(): boolean {
    const s = this.sel;
    return !!s && (s.a.page !== s.b.page || s.a.i !== s.b.i);
  }

  /** The selection's start and end in reading order. */
  private ordered(): [Caret, Caret] | null {
    const s = this.sel;
    if (!s) return null;
    const before = s.a.page < s.b.page || (s.a.page === s.b.page && s.a.i <= s.b.i);
    return before ? [s.a, s.b] : [s.b, s.a];
  }

  /** The selected text, ready for the clipboard. */
  async selectedText(): Promise<string> {
    const o = this.ordered();
    if (!o) return "";
    const [a, b] = o;
    const parts: string[] = [];
    for (let p = a.page; p <= b.page; p++) {
      const t = await this.loadText(p);
      parts.push(t.text.slice(p === a.page ? a.i : 0, p === b.page ? b.i : t.text.length));
    }
    return cleanCopy(parts.join("\n"));
  }

  // --------------------------------------------------------------- search

  /** Replaces all search hits (pages to character ranges). */
  setHits(hits: Map<number, [number, number][]>, current: Hit | null = null) {
    const pages = new Set([...this.hits.keys(), ...hits.keys()]);
    this.hits = hits;
    this.currentHit = current;
    for (const p of pages) {
      if (!this.views.has(p)) continue;
      if (hits.has(p)) void this.loadText(p).then(() => this.drawMarks(p));
      else this.drawMarks(p);
    }
  }

  /** Adds hits for one page as they arrive. */
  addHits(page: number, ranges: [number, number][]) {
    this.hits.set(page, ranges);
    if (this.views.has(page)) void this.loadText(page).then(() => this.drawMarks(page));
  }

  /** Makes `h` the current hit and scrolls it into view. */
  async showHit(h: Hit) {
    const old = this.currentHit;
    this.currentHit = h;
    if (old && this.views.has(old.page)) this.drawMarks(old.page);
    const t = await this.loadText(h.page);
    const rects = rangeRects(t, h.start, h.end).map((r) => rotateRect(r, this._rotation));
    if (this._mode === "single" && this.single !== h.page) this.goToPage(h.page);
    const b = this.boxes.get(h.page);
    if (!b || !rects.length) return this.goToPage(h.page);
    const [l, top, r, bot] = rects[0];
    const w = b.pw / this.dpr;
    const ht = b.ph / this.dpr;
    const y0 = b.y + top * ht;
    const y1 = b.y + bot * ht;
    const s = this.scroller;
    if (y0 < s.scrollTop || y1 > s.scrollTop + s.clientHeight) s.scrollTop = y0 - s.clientHeight / 3;
    const x0 = b.x + l * w;
    const x1 = b.x + r * w;
    if (x0 < s.scrollLeft || x1 > s.scrollLeft + s.clientWidth) s.scrollLeft = x0 - s.clientWidth / 3;
    this.schedule();
    if (this.views.has(h.page)) this.drawMarks(h.page);
  }

  private pageHasMarks(i: number): boolean {
    const o = this.ordered();
    return this.hits.has(i) || (!!o && i >= o[0].page && i <= o[1].page);
  }

  /** Redraws page `i`'s selection and search highlights. */
  private drawMarks(i: number) {
    const v = this.views.get(i);
    if (!v) return;
    v.marks.replaceChildren();
    const t = this.texts.get(i);
    if (!t) return;
    const add = (start: number, end: number, cls: string) => {
      for (const r of rangeRects(t, start, end)) {
        const [l, top, rt, bot] = rotateRect(r, this._rotation);
        const d = document.createElement("div");
        d.className = cls;
        d.style.left = `${l * 100}%`;
        d.style.top = `${top * 100}%`;
        d.style.width = `${(rt - l) * 100}%`;
        d.style.height = `${(bot - top) * 100}%`;
        v.marks.append(d);
      }
    };
    for (const [s, e] of this.hits.get(i) ?? []) {
      const cur = this.currentHit && this.currentHit.page === i && this.currentHit.start === s;
      add(s, e, cur ? "hit current" : "hit");
    }
    const o = this.ordered();
    if (o && i >= o[0].page && i <= o[1].page) {
      add(i === o[0].page ? o[0].i : 0, i === o[1].page ? o[1].i : t.text.length, "sel");
    }
  }

  // ----------------------------------------------------------------- links

  /** Loads page `i`'s links once, then shows them on its view. */
  private loadLinks(i: number) {
    if (this.links.has(i)) return;
    this.links.set(i, "loading");
    pageLinks(this.info.doc, i)
      .then((l) => this.links.set(i, l))
      .catch(() => this.links.set(i, []))
      .finally(() => {
        const v = this.views.get(i);
        if (v) this.placeLinks(i, v);
      });
  }

  /** Lays the clickable link areas over page `i`, turned with the view. */
  private placeLinks(i: number, v: PageView) {
    const links = this.links.get(i);
    if (!Array.isArray(links)) return;
    for (const el of v.links) el.remove();
    v.links = links.map((link) => {
      const [l, t, r, b] = rotateRect(link.rect, this._rotation);
      const a = document.createElement("a");
      a.className = "link";
      a.tabIndex = -1;
      a.style.left = `${l * 100}%`;
      a.style.top = `${t * 100}%`;
      a.style.width = `${(r - l) * 100}%`;
      a.style.height = `${(b - t) * 100}%`;
      const target = link.target;
      a.title = target.kind === "uri" ? target.uri : tr("page.goTo", { n: target.page + 1 });
      a.href = target.kind === "uri" ? target.uri : "#";
      a.addEventListener("click", (e) => {
        e.preventDefault();
        this.follow(target);
      });
      v.el.append(a);
      return a;
    });
  }

  // ----------------------------------------------------------------- tiles

  private isAttached(key: string): boolean {
    const page = Number(key.slice(0, key.indexOf("/")));
    return this.views.get(page)?.tiles.has(key) ?? false;
  }

  private request(
    key: string,
    spec: RenderSpec,
    prio: number,
    done: (c: HTMLCanvasElement) => void,
    always?: () => void,
  ) {
    if (this.inflight.has(key)) {
      always?.();
      return;
    }
    this.inflight.add(key);
    this.transport(spec, prio)
      .then((t) => {
        this.onTile(t);
        if (spec.w === TILE && spec.page_w !== PREVIEW_W && t.renderUs > SLOW_TILE_US) {
          this.slowPages.add(spec.page);
        }
        const c = document.createElement("canvas");
        c.width = t.w;
        c.height = t.h;
        // One copy, straight into the canvas; the source buffer is freed at once.
        c.getContext("2d", { alpha: false })!.putImageData(new ImageData(t.rgba, t.w, t.h), 0, 0);
        t.release?.();
        done(c);
      })
      .catch((e) => {
        if (e instanceof Cancelled) return;
        // Remember the failure so the page shows it instead of retrying forever.
        this.errors.set(spec.page, String(e));
        const v = this.views.get(spec.page);
        if (v) this.showError(v, String(e));
      })
      .finally(() => {
        this.inflight.delete(key);
        always?.();
        this.schedule();
      });
  }

  private showError(v: PageView, message: string) {
    if (v.error) return;
    const el = document.createElement("div");
    el.className = "page-error";
    el.setAttribute("role", "alert");
    const title = document.createElement("strong");
    title.textContent = tr("page.error");
    const detail = document.createElement("span");
    detail.textContent = message.charAt(0).toUpperCase() + message.slice(1) + ".";
    el.append(title, detail);
    v.error = el;
    v.el.append(el);
  }

  private attachTile(v: PageView, key: string, s: RenderSpec, c: HTMLCanvasElement) {
    const st = c.style;
    st.left = `${(s.x / s.page_w) * 100}%`;
    st.top = `${(s.y / s.page_h) * 100}%`;
    st.width = `${(c.width / s.page_w) * 100}%`;
    st.height = `${(c.height / s.page_h) * 100}%`;
    c.className = "tile";
    c.dataset.pw = String(s.page_w);
    c.dataset.rot = String(s.rotation);
    c.dataset.side = String(s.w);
    c.dataset.page = String(s.page);
    v.tiles.set(key, c);
    v.el.append(c);
    this.dropStaleTiles(s.page);
  }

  /**
   * Detaches current tiles of a page that are not in `wanted` (all of them
   * when null). They stay in the tile cache. Tiles of other zooms are left
   * to `dropStaleTiles`, which keeps them until the new ones cover the view.
   */
  private detachTiles(v: PageView, wanted: Set<string> | null): boolean {
    let any = false;
    for (const [k, c] of v.tiles) {
      if (wanted?.has(k)) continue;
      if (wanted && this.isStale(c)) continue;
      c.remove();
      v.tiles.delete(k);
      any = true;
    }
    return any;
  }

  /** True for a tile of another zoom, rotation or tile size than page `i` now uses. */
  private isStale(c: HTMLCanvasElement): boolean {
    const page = Number(c.dataset.page);
    const [pw] = this.pagePx(page);
    return (
      Number(c.dataset.pw) !== pw ||
      c.dataset.rot !== String(this._rotation) ||
      c.dataset.side !== String(this.tileSide(page))
    );
  }

  /** Once the current zoom's tiles cover the view, removes other zooms' and rotations' tiles. */
  private dropStaleTiles(i: number) {
    const v = this.views.get(i);
    if (!v) return;
    const rot = String(this._rotation);
    // Tiles of another rotation never line up: drop them at once.
    const sharp = this.isSharp(i);
    for (const [k, c] of v.tiles) {
      if (c.dataset.rot !== rot || (sharp && this.isStale(c))) {
        c.remove();
        v.tiles.delete(k);
      }
    }
  }
}
