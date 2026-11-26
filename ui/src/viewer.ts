// Continuous-scroll page view with a low-resolution preview per page and
// sharp tiles on top. Plain DOM, no framework: this runs on every scroll frame.
//
// Each page is a positioned <div>. Its preview canvas is stretched to fill it,
// and tile canvases are placed in percent of the page size, so tiles from the
// previous zoom stay correctly placed (just blurry) until the new ones arrive.

import { Cancelled, setViewport, type DocInfo, type RenderSpec, type TileData, type Transport } from "./api";

const TILE = 512; // device pixels
const PREVIEW_W = 300; // device pixels
const GAP = 12; // CSS pixels between pages
const MARGIN = 16; // CSS pixels around the document
const TILE_MARGIN = 256; // device pixels rendered beyond the visible edge
const TILE_BUDGET = 24 * 1024 * 1024; // bytes of tile canvases kept
const PREVIEW_BUDGET = 8 * 1024 * 1024; // bytes of preview canvases kept
const PRIO_PREVIEW_VISIBLE = 3000;
const PRIO_PREVIEW_NEAR = 2000;
const PRIO_TILE = 1000;

export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 8;

/** Least-recently-used cache of canvases, bounded by pixel bytes. */
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
    for (const [k, old] of this.map) {
      if (this.bytes <= this.budget) break;
      if (inUse(k)) continue;
      this.map.delete(k);
      this.bytes -= old.width * old.height * 4;
      old.remove();
      old.width = old.height = 0; // free the backing store now
    }
  }
}

interface PageView {
  el: HTMLDivElement;
  preview: HTMLCanvasElement | null;
  tiles: Map<string, HTMLCanvasElement>; // key -> canvas attached to el
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
  private tops: number[] = []; // CSS px, page top at zoom 1
  private widest = 0; // points
  private views = new Map<number, PageView>();
  private tileCache = new CanvasCache(TILE_BUDGET);
  private previewCache = new CanvasCache(PREVIEW_BUDGET);
  private inflight = new Set<string>();
  private frame = 0;
  private lastKeep = "";
  private _zoom = 1;
  dpr = window.devicePixelRatio || 1;
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
    this.content = document.createElement("div");
    this.content.className = "content";
    this.scroller.append(this.content);
    host.append(this.scroller);
    this.widest = Math.max(...info.pages.map((p) => p.w));
    this.scroller.addEventListener("scroll", () => this.schedule(), { passive: true });
    new ResizeObserver(() => this.schedule()).observe(this.scroller);
    this.fitWidth();
  }

  destroy() {
    cancelAnimationFrame(this.frame);
    this.scroller.remove();
  }

  get zoom() {
    return this._zoom;
  }

  /** Layout at the current zoom: page tops in CSS px. */
  private layout() {
    const z = this._zoom;
    let y = MARGIN;
    this.tops = this.info.pages.map((p) => {
      const top = y;
      y += p.h * z + GAP;
      return top;
    });
    this.content.style.height = `${y - GAP + MARGIN}px`;
    this.content.style.width = `${this.widest * z + 2 * MARGIN}px`;
    for (const [i, v] of this.views) this.place(i, v.el);
  }

  private place(i: number, el: HTMLDivElement) {
    const p = this.info.pages[i];
    const z = this._zoom;
    const contentW = Math.max(this.widest * z + 2 * MARGIN, this.scroller.clientWidth);
    el.style.left = `${Math.round((contentW - p.w * z) / 2)}px`;
    el.style.top = `${this.tops[i]}px`;
    el.style.width = `${p.w * z}px`;
    el.style.height = `${p.h * z}px`;
  }

  fitWidth() {
    const avail = Math.max(100, this.scroller.clientWidth - 2 * MARGIN - 16);
    // Anchor at the top edge: the line at the top of the view stays there.
    this.setZoom(avail / this.widest, 0, 0);
    this.scroller.scrollLeft = 0;
  }

  /** Changes zoom keeping the point at (`ax`, `ay`) in the viewport still. */
  setZoom(z: number, ax = this.scroller.clientWidth / 2, ay = this.scroller.clientHeight / 2) {
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

  goToPage(i: number) {
    this.scroller.scrollTop = this.tops[Math.max(0, Math.min(i, this.tops.length - 1))] - MARGIN;
  }

  /** Index of the page at the top of the viewport. */
  currentPage(): number {
    return Math.max(0, this.pageAt(this.scroller.scrollTop + this.scroller.clientHeight / 3));
  }

  private pageAt(y: number): number {
    let lo = 0;
    let hi = this.tops.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.tops[mid] <= y) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  /** Pages intersecting the viewport, extended by `extra` CSS px. */
  pagesInView(extra = 0): number[] {
    const top = this.scroller.scrollTop - extra;
    const bottom = this.scroller.scrollTop + this.scroller.clientHeight + extra;
    const out: number[] = [];
    for (let i = this.pageAt(Math.max(0, top)); i < this.tops.length && this.tops[i] < bottom; i++) {
      if (this.tops[i] + this.info.pages[i].h * this._zoom > top) out.push(i);
    }
    return out;
  }

  /** Device-pixel size of page `i` at the current zoom. */
  pagePx(i: number): [number, number] {
    const p = this.info.pages[i];
    const s = this._zoom * this.dpr;
    return [Math.max(1, Math.round(p.w * s)), Math.max(1, Math.round(p.h * s))];
  }

  /** Visible part of page `i`, in its device pixels, grown by `margin`. */
  private visibleRect(i: number, margin: number): Rect | null {
    const v = this.views.get(i);
    if (!v) return null;
    const [pw, ph] = this.pagePx(i);
    const left = (this.scroller.scrollLeft - v.el.offsetLeft) * this.dpr - margin;
    const top = (this.scroller.scrollTop - this.tops[i]) * this.dpr - margin;
    const x0 = Math.max(0, left);
    const y0 = Math.max(0, top);
    const x1 = Math.min(pw, left + this.scroller.clientWidth * this.dpr + 2 * margin);
    const y1 = Math.min(ph, top + this.scroller.clientHeight * this.dpr + 2 * margin);
    return x1 > x0 && y1 > y0 ? { x: x0, y: y0, w: x1 - x0, h: y1 - y0 } : null;
  }

  private tileSpecs(i: number, margin: number): RenderSpec[] {
    const r = this.visibleRect(i, margin);
    if (!r) return [];
    const [pw, ph] = this.pagePx(i);
    const specs: RenderSpec[] = [];
    for (let y = Math.floor(r.y / TILE) * TILE; y < r.y + r.h; y += TILE) {
      for (let x = Math.floor(r.x / TILE) * TILE; x < r.x + r.w; x += TILE) {
        specs.push({ doc: this.info.doc, page: i, page_w: pw, page_h: ph, x, y, w: TILE, h: TILE });
      }
    }
    return specs;
  }

  private key = (s: RenderSpec) => `${s.page}/${s.page_w}/${s.x}/${s.y}`;

  private previewSpec(i: number): RenderSpec {
    const p = this.info.pages[i];
    const w = PREVIEW_W;
    const h = Math.max(1, Math.min(4096, Math.round((w * p.h) / p.w)));
    return { doc: this.info.doc, page: i, page_w: w, page_h: h, x: 0, y: 0, w, h };
  }

  /** True when page `i` has a preview or tiles covering what is in view. */
  hasContent(i: number): boolean {
    const v = this.views.get(i);
    return !!v && (v.preview !== null || v.tiles.size > 0);
  }

  /** True when every visible tile of page `i` at the current zoom is shown. */
  isSharp(i: number): boolean {
    const v = this.views.get(i);
    if (!v) return false;
    return this.tileSpecs(i, 0).every((s) => v.tiles.has(this.key(s)));
  }

  schedule() {
    if (!this.frame) {
      this.frame = requestAnimationFrame(() => {
        this.frame = 0;
        this.update();
      });
    }
  }

  private update() {
    const near = new Set(this.pagesInView(this.scroller.clientHeight));
    const visible = this.pagesInView(0);

    for (const [i, v] of this.views) {
      if (!near.has(i)) {
        v.el.remove();
        this.views.delete(i);
      }
    }
    for (const i of near) {
      if (!this.views.has(i)) {
        const el = document.createElement("div");
        el.className = "page";
        this.place(i, el);
        this.content.append(el);
        this.views.set(i, { el, preview: null, tiles: new Map() });
      }
    }

    const keep: [number, number][] = [];
    const center = this.currentPage();

    // Previews first: they guarantee a page is never blank.
    for (const i of near) {
      const v = this.views.get(i)!;
      const spec = this.previewSpec(i);
      keep.push([i, spec.page_w]);
      if (v.preview) continue;
      const key = `p${i}`;
      const cached = this.previewCache.get(key);
      if (cached) {
        this.attachPreview(v, cached);
        continue;
      }
      const prio = visible.includes(i) ? PRIO_PREVIEW_VISIBLE : PRIO_PREVIEW_NEAR - Math.abs(i - center);
      this.request(key, spec, prio, (c) => {
        this.previewCache.set(key, c, (k) => this.views.has(Number(k.slice(1))));
        const view = this.views.get(i);
        if (view && !view.preview) this.attachPreview(view, c);
      });
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
          const view = this.views.get(i);
          if (view) this.attachTile(view, key, s, c);
        });
      });
      this.dropStaleTiles(i);
    }

    // Tell the engine which renders still matter; it cancels the rest.
    const keepKey = JSON.stringify(keep);
    if (keepKey !== this.lastKeep) {
      this.lastKeep = keepKey;
      void setViewport(this.info.doc, keep);
    }
    this.onUpdate();
  }

  private isAttached(key: string): boolean {
    const page = Number(key.slice(0, key.indexOf("/")));
    return this.views.get(page)?.tiles.has(key) ?? false;
  }

  private request(key: string, spec: RenderSpec, prio: number, done: (c: HTMLCanvasElement) => void) {
    if (this.inflight.has(key)) return;
    this.inflight.add(key);
    this.transport(spec, prio)
      .then((t) => {
        this.onTile(t);
        const c = document.createElement("canvas");
        c.width = t.w;
        c.height = t.h;
        // One copy, straight into the canvas; the source buffer is freed at once.
        c.getContext("2d", { alpha: false })!.putImageData(new ImageData(t.rgba, t.w, t.h), 0, 0);
        t.release?.();
        done(c);
      })
      .catch((e) => {
        if (!(e instanceof Cancelled)) console.warn("tile failed", key, e);
      })
      .finally(() => {
        this.inflight.delete(key);
        this.schedule();
      });
  }

  private attachPreview(v: PageView, c: HTMLCanvasElement) {
    c.className = "preview";
    v.preview = c;
    v.el.prepend(c);
  }

  private attachTile(v: PageView, key: string, s: RenderSpec, c: HTMLCanvasElement) {
    const st = c.style;
    st.left = `${(s.x / s.page_w) * 100}%`;
    st.top = `${(s.y / s.page_h) * 100}%`;
    st.width = `${(c.width / s.page_w) * 100}%`;
    st.height = `${(c.height / s.page_h) * 100}%`;
    c.className = "tile";
    c.dataset.pw = String(s.page_w);
    v.tiles.set(key, c);
    v.el.append(c);
    this.dropStaleTiles(s.page);
  }

  /** Once the current zoom's tiles cover the view, removes other zooms' tiles. */
  private dropStaleTiles(i: number) {
    const v = this.views.get(i);
    if (!v || !this.isSharp(i)) return;
    const [pw] = this.pagePx(i);
    for (const [k, c] of v.tiles) {
      if (Number(c.dataset.pw) !== pw) {
        c.remove();
        v.tiles.delete(k);
      }
    }
  }
}
