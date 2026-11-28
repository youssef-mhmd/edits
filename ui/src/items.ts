// Page items (text boxes, pictures, shapes, lines, tables) on the page view:
// choosing, moving and resizing them with guides and snapping, making new
// ones with the item tools, and writing text in place. Items are page
// content drawn by the engine; this layer only shows frames, handles and
// guides over them, in page fractions (/Rotate applied, view rotation
// removed), like the annotation layer.

import {
  itemPicture,
  itemsPage,
  itemsSet,
  type Changed,
  type ItemKind,
  type ItemView,
  type TableData,
  type TextStyle,
} from "./api";

export type ItemTool = "itext" | "irect" | "iellipse" | "iline" | "iarrow" | "iimage" | "itable";
export const ITEM_TOOLS: ItemTool[] = ["itext", "irect", "iellipse", "iline", "iarrow", "iimage", "itable"];
export const isItemTool = (t: string): t is ItemTool => (ITEM_TOOLS as string[]).includes(t);

/** What new items look like. */
export interface ItemDefaults {
  text: TextStyle;
  stroke: number[];
  fill: number[] | null;
  width: number;
  /** The interface reads right to left: new tables start at the right. */
  rtl: boolean;
}

/** What the layer needs from the viewer and the app. */
export interface ItemHost {
  doc(): number;
  /** Page size as shown, in points. */
  pageSize(i: number): { w: number; h: number };
  /** CSS px per point. */
  zoom(): number;
  /** The page's item layer, if the page is on screen. */
  layer(i: number): HTMLDivElement | null;
  /** Takes in a change (undo state, pages drawn again). */
  changed(c: Changed): void;
  error(e: unknown): void;
  /** The chosen items changed. */
  selection(items: ItemView[]): void;
  /** Asks for a picture file. */
  pickPicture(): Promise<string | null>;
  /** Opens the table editor for a table item. */
  editTable(item: ItemView): void;
  defaults(): ItemDefaults;
  /** Margins for guides, in points (left, top, right, bottom), or null. */
  margins(): number[] | null;
}

type Pt = [number, number];
type Gesture =
  | { type: "create"; tool: ItemTool; page: number; a: Pt; b: Pt }
  | { type: "move"; page: number; start: Pt; dx: number; dy: number; moved: boolean; x: number; y: number }
  | { type: "resize"; page: number; id: number; handle: string; rect: number[] };

const HANDLE_PX = 7;
const HIT_PX = 4;
const SNAP_PX = 6;

const pct = (x: number) => `${x * 100}%`;
const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

export class ItemLayer {
  private lists = new Map<number, ItemView[]>();
  private loading = new Set<number>();
  /** The chosen items: their page and ids. */
  private page = -1;
  private ids = new Set<number>();
  private gesture: Gesture | null = null;
  /** Guide lines shown while snapping, in fractions. */
  private guides: { x: number[]; y: number[] } = { x: [], y: [] };
  /** The text being written in place. */
  private editing: { page: number; id: number; area: HTMLTextAreaElement } | null = null;
  /** Changes of a page run one after another. */
  private queue: Promise<void> = Promise.resolve();
  /** Copied items, for pasting. */
  private clipboard: ItemView[] = [];

  constructor(private host: ItemHost) {}

  // ------------------------------------------------------------ data

  /** Reads page `i`'s items once (when its view is made). */
  load(i: number) {
    if (this.lists.has(i) || this.loading.has(i)) {
      this.draw(i);
      return;
    }
    this.loading.add(i);
    void itemsPage(this.host.doc(), i)
      .then((l) => this.lists.set(i, l))
      .catch(() => this.lists.set(i, []))
      .finally(() => {
        this.loading.delete(i);
        this.draw(i);
      });
  }

  /** Reads the items of `pages` again (after undo, redo, recovery). */
  reload(pages: number[]) {
    for (const i of pages) {
      this.lists.delete(i);
      if (this.host.layer(i)) this.load(i);
    }
    if (pages.includes(this.page)) this.select(this.page, []);
  }

  items(i: number): ItemView[] {
    return this.lists.get(i) ?? [];
  }

  /** The chosen items. */
  get selected(): ItemView[] {
    return this.items(this.page).filter((it) => this.ids.has(it.id));
  }

  get busy(): boolean {
    return this.gesture !== null;
  }

  /** Writes page `i`'s new list; the engine draws it. */
  private commit(i: number, list: ItemView[], choose?: (items: ItemView[]) => number[]): Promise<void> {
    this.lists.set(i, list);
    this.draw(i);
    const run = async () => {
      try {
        const c = await itemsSet(this.host.doc(), i, list);
        this.lists.set(i, c.items);
        if (choose) this.select(i, choose(c.items));
        else
          this.select(
            i,
            [...this.ids].filter((id) => c.items.some((it) => it.id === id)),
          );
        this.host.changed(c);
      } catch (e) {
        this.host.error(e);
        this.lists.delete(i);
        this.load(i);
      }
    };
    this.queue = this.queue.then(run);
    return this.queue;
  }

  /** Changes the chosen items with `f` and writes them. */
  update(f: (it: ItemView) => void) {
    if (this.page < 0 || !this.ids.size) return;
    const list = this.items(this.page).map((it) => {
      if (!this.ids.has(it.id)) return it;
      const c = clone(it);
      f(c);
      return c;
    });
    void this.commit(this.page, list);
  }

  /** Changes one item (by id) on page `i`. */
  private updateOne(i: number, id: number, f: (it: ItemView) => void) {
    const list = this.items(i).map((it) => {
      if (it.id !== id) return it;
      const c = clone(it);
      f(c);
      return c;
    });
    void this.commit(i, list);
  }

  setTable(i: number, id: number, t: TableData) {
    this.updateOne(i, id, (it) => {
      if (it.kind.type === "table") it.kind = { type: "table", ...t };
    });
  }

  // ------------------------------------------------------------ choosing

  select(page: number, ids: number[]) {
    const old = this.page;
    // Groups are chosen whole.
    const list = this.items(page);
    const groups = new Set(
      list.filter((it) => ids.includes(it.id) && it.group !== null).map((it) => it.group),
    );
    const all = list.filter((it) => ids.includes(it.id) || (it.group !== null && groups.has(it.group)));
    this.page = all.length ? page : -1;
    this.ids = new Set(all.map((it) => it.id));
    if (old >= 0 && old !== page) this.draw(old);
    if (page >= 0) this.draw(page);
    this.host.selection(this.selected);
  }

  clear() {
    if (this.page >= 0 || this.ids.size) this.select(-1, []);
  }

  /** The topmost item at a point of page `page`. */
  hit(page: number, fx: number, fy: number): ItemView | null {
    const list = this.items(page);
    const p = this.host.pageSize(page);
    const tx = HIT_PX / (p.w * this.host.zoom());
    const ty = HIT_PX / (p.h * this.host.zoom());
    for (let k = list.length - 1; k >= 0; k--) {
      const [l, t, r, b] = list[k].rect;
      if (fx >= l - tx && fx <= r + tx && fy >= t - ty && fy <= b + ty) return list[k];
    }
    return null;
  }

  private resizable(): ItemView | null {
    const s = this.selected;
    if (s.length !== 1 || s[0].kind.type === "table") return null;
    return s[0];
  }

  private handles(r: number[]): [string, number, number][] {
    const [l, t, rt, b] = r;
    const cx = (l + rt) / 2,
      cy = (t + b) / 2;
    return [
      ["nw", l, t],
      ["n", cx, t],
      ["ne", rt, t],
      ["e", rt, cy],
      ["se", rt, b],
      ["s", cx, b],
      ["sw", l, b],
      ["w", l, cy],
    ];
  }

  private handleAt(page: number, fx: number, fy: number): string | null {
    const it = this.resizable();
    if (!it || page !== this.page) return null;
    const p = this.host.pageSize(page);
    const hx = (HANDLE_PX + 2) / (p.w * this.host.zoom());
    const hy = (HANDLE_PX + 2) / (p.h * this.host.zoom());
    for (const [name, x, y] of this.handles(it.rect)) {
      if (Math.abs(fx - x) <= hx && Math.abs(fy - y) <= hy) return name;
    }
    return null;
  }

  /** The cursor over a point, or null if the layer has nothing there. */
  cursorAt(page: number, fx: number, fy: number): string | null {
    const h = this.handleAt(page, fx, fy);
    if (h)
      return h === "n" || h === "s"
        ? "ns-resize"
        : h === "e" || h === "w"
          ? "ew-resize"
          : h === "nw" || h === "se"
            ? "nwse-resize"
            : "nesw-resize";
    return this.hit(page, fx, fy) ? "move" : null;
  }

  // ------------------------------------------------------------ gestures

  /**
   * A press on a page with the select tool or an item tool. Returns true if
   * the layer took it.
   */
  down(tool: string, page: number, fx: number, fy: number, e: MouseEvent): boolean {
    this.finishEditing();
    if (isItemTool(tool)) {
      this.clear();
      this.gesture = { type: "create", tool, page, a: [fx, fy], b: [fx, fy] };
      this.draw(page);
      return true;
    }
    const handle = this.handleAt(page, fx, fy);
    if (handle) {
      const it = this.resizable()!;
      this.gesture = { type: "resize", page, id: it.id, handle, rect: [...it.rect] };
      return true;
    }
    const hit = this.hit(page, fx, fy);
    if (!hit) {
      this.clear();
      return false;
    }
    if (e.detail === 2) {
      this.open(page, hit);
      return true;
    }
    if (e.shiftKey && page === this.page) {
      const ids = new Set(this.ids);
      if (ids.has(hit.id)) ids.delete(hit.id);
      else ids.add(hit.id);
      this.select(page, [...ids]);
    } else if (!this.ids.has(hit.id) || page !== this.page) {
      this.select(page, [hit.id]);
    }
    this.gesture = {
      type: "move",
      page,
      start: [fx, fy],
      dx: 0,
      dy: 0,
      moved: false,
      x: e.clientX,
      y: e.clientY,
    };
    return true;
  }

  drag(page: number, fx: number, fy: number, e: MouseEvent) {
    const g = this.gesture;
    if (!g || page !== g.page) return;
    if (g.type === "create") {
      g.b = [clamp01(fx), clamp01(fy)];
    } else if (g.type === "move") {
      if (!g.moved && Math.hypot(e.clientX - g.x, e.clientY - g.y) < 3) return;
      g.moved = true;
      const box = this.bounds(this.selected);
      // Keep the items on the page.
      let dx = Math.max(-box[0], Math.min(1 - box[2], fx - g.start[0]));
      let dy = Math.max(-box[1], Math.min(1 - box[3], fy - g.start[1]));
      if (!e.altKey) [dx, dy] = this.snapMove(page, box, dx, dy);
      g.dx = dx;
      g.dy = dy;
    } else {
      const it = this.items(page).find((x) => x.id === g.id);
      if (!it) return;
      const r = [...it.rect];
      const h = g.handle;
      const p = this.host.pageSize(page);
      const minX = 4 / p.w,
        minY = it.kind.type === "line" ? 0 : 4 / p.h;
      let [x, y] = [clamp01(fx), clamp01(fy)];
      if (!e.altKey) [x, y] = this.snapPoint(page, x, y, h);
      if (h.includes("w")) r[0] = Math.min(x, r[2] - minX);
      if (h.includes("e")) r[2] = Math.max(x, r[0] + minX);
      if (h.includes("n")) r[1] = Math.min(y, r[3] - minY);
      if (h.includes("s")) r[3] = Math.max(y, r[1] + minY);
      // Pictures keep their shape from the corners (Shift lets them stretch).
      if (it.kind.type === "image" && h.length === 2 && !e.shiftKey) {
        const ar = (it.rect[2] - it.rect[0]) / Math.max(1e-6, it.rect[3] - it.rect[1]);
        const w = r[2] - r[0];
        const hgt = w / ar;
        if (h.includes("n")) r[1] = r[3] - hgt;
        else r[3] = r[1] + hgt;
      }
      g.rect = r;
    }
    this.draw(page);
  }

  up() {
    const g = this.gesture;
    this.gesture = null;
    this.guides = { x: [], y: [] };
    if (!g) return;
    if (g.type === "create") void this.create(g);
    else if (g.type === "move") {
      if (g.moved && (g.dx || g.dy)) this.update((it) => this.shift(it, g.dx, g.dy));
      else this.draw(g.page);
    } else {
      const it = this.items(g.page).find((x) => x.id === g.id);
      if (it && g.rect.join() !== it.rect.join()) this.updateOne(g.page, g.id, (c) => (c.rect = g.rect));
      else this.draw(g.page);
    }
  }

  private shift(it: ItemView, dx: number, dy: number) {
    it.rect = [it.rect[0] + dx, it.rect[1] + dy, it.rect[2] + dx, it.rect[3] + dy];
  }

  // ------------------------------------------------------------ snapping

  /** Lines to snap to: page edges and middle, margins, other items. */
  private targets(page: number): { x: number[]; y: number[] } {
    const p = this.host.pageSize(page);
    const x = [0, 0.5, 1];
    const y = [0, 0.5, 1];
    const m = this.host.margins();
    if (m) {
      x.push(m[0] / p.w, 1 - m[2] / p.w);
      y.push(m[1] / p.h, 1 - m[3] / p.h);
    }
    for (const it of this.items(page)) {
      if (page === this.page && this.ids.has(it.id)) continue;
      const [l, t, r, b] = it.rect;
      x.push(l, (l + r) / 2, r);
      y.push(t, (t + b) / 2, b);
    }
    return { x, y };
  }

  private nearest(vals: number[], cands: number[], tol: number): { d: number; at: number } | null {
    let best: { d: number; at: number } | null = null;
    for (const v of vals)
      for (const c of cands) {
        const d = c - v;
        if (Math.abs(d) <= tol && (!best || Math.abs(d) < Math.abs(best.d))) best = { d, at: c };
      }
    return best;
  }

  private snapMove(page: number, box: number[], dx: number, dy: number): [number, number] {
    const p = this.host.pageSize(page);
    const z = this.host.zoom();
    const t = this.targets(page);
    const [l, top, r, b] = [box[0] + dx, box[1] + dy, box[2] + dx, box[3] + dy];
    const sx = this.nearest([l, (l + r) / 2, r], t.x, SNAP_PX / (p.w * z));
    const sy = this.nearest([top, (top + b) / 2, b], t.y, SNAP_PX / (p.h * z));
    this.guides = { x: sx ? [sx.at] : [], y: sy ? [sy.at] : [] };
    return [dx + (sx?.d ?? 0), dy + (sy?.d ?? 0)];
  }

  private snapPoint(page: number, x: number, y: number, handle: string): [number, number] {
    const p = this.host.pageSize(page);
    const z = this.host.zoom();
    const t = this.targets(page);
    const sx =
      handle.includes("e") || handle.includes("w") ? this.nearest([x], t.x, SNAP_PX / (p.w * z)) : null;
    const sy =
      handle.includes("n") || handle.includes("s") ? this.nearest([y], t.y, SNAP_PX / (p.h * z)) : null;
    this.guides = { x: sx ? [sx.at] : [], y: sy ? [sy.at] : [] };
    return [x + (sx?.d ?? 0), y + (sy?.d ?? 0)];
  }

  private bounds(items: ItemView[]): number[] {
    if (!items.length) return [0, 0, 0, 0];
    return [
      Math.min(...items.map((i) => i.rect[0])),
      Math.min(...items.map((i) => i.rect[1])),
      Math.max(...items.map((i) => i.rect[2])),
      Math.max(...items.map((i) => i.rect[3])),
    ];
  }

  // ------------------------------------------------------------ making items

  private async create(g: Extract<Gesture, { type: "create" }>) {
    const { page, tool } = g;
    const p = this.host.pageSize(page);
    const z = this.host.zoom();
    const d = this.host.defaults();
    const [a, b] = [g.a, g.b];
    const tiny = Math.hypot((b[0] - a[0]) * p.w, (b[1] - a[1]) * p.h) * z < 4;
    let rect = [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[0], b[0]), Math.max(a[1], b[1])];
    const size = (wPt: number, hPt: number) => {
      rect = [a[0], a[1], Math.min(1, a[0] + wPt / p.w), Math.min(1, a[1] + hPt / p.h)];
    };
    let kind: ItemKind;
    switch (tool) {
      case "itext":
        if (tiny) size(200, d.text.size * 2);
        kind = {
          type: "text",
          text: "",
          style: { ...d.text },
          fill: null,
          border: null,
          border_width: 1,
          padding: 4,
        };
        // Written first, then placed.
        this.draw(page);
        this.write(page, { id: 0, rect, group: null, kind });
        return;
      case "irect":
      case "iellipse":
        if (tiny) size(100, 60);
        kind = {
          type: "shape",
          ellipse: tool === "iellipse",
          stroke: [...d.stroke],
          width: d.width,
          fill: d.fill && [...d.fill],
        };
        break;
      case "iline":
      case "iarrow": {
        if (tiny) {
          this.draw(page);
          return;
        }
        const down = (b[0] - a[0]) * (b[1] - a[1]) >= 0;
        kind = {
          type: "line",
          flip: !down,
          reverse: b[0] < a[0] || (b[0] === a[0] && b[1] < a[1]),
          color: [...d.stroke],
          width: d.width,
          arrow: tool === "iarrow",
        };
        break;
      }
      case "iimage": {
        const path = await this.host.pickPicture();
        if (!path) {
          this.draw(page);
          return;
        }
        try {
          const pic = await itemPicture(this.host.doc(), path);
          const ar = pic.pixels[0] / Math.max(1, pic.pixels[1]);
          if (tiny) {
            const w = Math.min(p.w / 3, pic.pixels[0] * 0.75);
            size(w, w / ar);
          } else {
            // The box's width, the picture's shape.
            const w = (rect[2] - rect[0]) * p.w;
            rect[3] = Math.min(1, rect[1] + w / ar / p.h);
          }
          kind = { type: "image", image: pic.image };
        } catch (e) {
          this.host.error(e);
          this.draw(page);
          return;
        }
        break;
      }
      case "itable":
        rect = [a[0], a[1], a[0], a[1]];
        kind = {
          type: "table",
          widths: [100, 100, 100],
          heights: [20, 20, 20],
          cells: [
            ["", "", ""],
            ["", "", ""],
            ["", "", ""],
          ],
          style: { ...d.text, size: Math.min(d.text.size, 11) },
          rtl: d.text.rtl ?? d.rtl,
          border: [0, 0, 0],
          border_width: 0.75,
          header: [0.9, 0.9, 0.9],
          padding: 3,
        };
        break;
    }
    const item: ItemView = { id: 0, rect, group: null, kind };
    const before = new Set(this.items(page).map((it) => it.id));
    await this.commit(page, [...this.items(page), item], (items) =>
      items.filter((it) => !before.has(it.id)).map((it) => it.id),
    );
    const made = this.selected[0];
    if (made?.kind.type === "table") this.host.editTable(made);
  }

  // ------------------------------------------------------------ writing text

  /** Opens an item to change its content: text in place, tables in their editor. */
  open(page: number, it: ItemView) {
    if (it.kind.type === "text") this.write(page, it);
    else if (it.kind.type === "table") this.host.editTable(it);
  }

  /** Writes text in place over a text item (new when its id is 0). */
  private write(page: number, it: ItemView) {
    const layer = this.host.layer(page);
    if (!layer || it.kind.type !== "text") return;
    this.finishEditing();
    const k = it.kind;
    const z = this.host.zoom();
    const area = document.createElement("textarea");
    area.className = "iedit";
    area.value = k.text;
    area.dir = k.style.rtl === null ? "auto" : k.style.rtl ? "rtl" : "ltr";
    const st = area.style;
    st.left = pct(it.rect[0]);
    st.top = pct(it.rect[1]);
    st.width = pct(it.rect[2] - it.rect[0]);
    st.minHeight = pct(it.rect[3] - it.rect[1]);
    st.fontSize = `${k.style.size * z}px`;
    st.lineHeight = String(k.style.line_height);
    st.padding = `${k.padding * z}px`;
    st.fontWeight = k.style.bold ? "700" : "400";
    st.fontStyle = k.style.italic ? "italic" : "normal";
    st.fontFamily = k.style.font
      ? `"${k.style.font}", "IBM Plex Sans Arabic", sans-serif`
      : `"IBM Plex Sans Arabic", sans-serif`;
    st.color = `rgb(${k.style.color.map((c) => Math.round(c * 255)).join(" ")})`;
    st.textAlign = k.style.align === "start" ? "start" : k.style.align;
    const grow = () => {
      area.style.height = "auto";
      area.style.height = `${area.scrollHeight}px`;
    };
    area.addEventListener("input", grow);
    area.addEventListener("keydown", (e) => {
      e.stopPropagation();
      if (e.key === "Escape") {
        area.value = k.text;
        this.finishEditing();
      } else if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
        this.finishEditing();
      }
    });
    area.addEventListener("mousedown", (e) => e.stopPropagation());
    area.addEventListener("blur", () => this.finishEditing());
    layer.append(area);
    this.editing = { page, id: it.id, area };
    if (it.id) this.select(page, [it.id]);
    grow();
    area.focus();
    // Hide the drawn text under the editor while writing.
    layer.classList.add("writing");
    this.pending = { page, item: it };
  }

  private pending: { page: number; item: ItemView } | null = null;

  /** Ends writing in place; keeps the text if it changed. */
  finishEditing() {
    const ed = this.editing;
    const p = this.pending;
    if (!ed || !p) return;
    this.editing = null;
    this.pending = null;
    const text = ed.area.value.replace(/\s+$/, "");
    ed.area.parentElement?.classList.remove("writing");
    ed.area.remove();
    const it = p.item;
    if (it.kind.type !== "text" || text === it.kind.text.replace(/\s+$/, "")) {
      this.draw(ed.page);
      return;
    }
    if (it.id === 0) {
      if (!text.trim()) {
        this.draw(ed.page);
        return;
      }
      const before = new Set(this.items(ed.page).map((x) => x.id));
      const made = clone(it);
      if (made.kind.type === "text") made.kind.text = text;
      void this.commit(ed.page, [...this.items(ed.page), made], (items) =>
        items.filter((x) => !before.has(x.id)).map((x) => x.id),
      );
    } else if (!text.trim()) {
      void this.commit(
        ed.page,
        this.items(ed.page).filter((x) => x.id !== it.id),
        () => [],
      );
    } else {
      this.updateOne(ed.page, it.id, (c) => {
        if (c.kind.type === "text") c.kind.text = text;
      });
    }
  }

  // ------------------------------------------------------------ arranging

  remove() {
    if (this.page < 0 || !this.ids.size) return;
    const page = this.page;
    void this.commit(
      page,
      this.items(page).filter((it) => !this.ids.has(it.id)),
      () => [],
    );
  }

  duplicate() {
    if (this.page < 0 || !this.ids.size) return;
    const page = this.page;
    const p = this.host.pageSize(page);
    const copies = this.selected.map((it) => {
      const c = clone(it);
      c.id = 0;
      this.shift(c, 10 / p.w, 10 / p.h);
      return c;
    });
    const before = new Set(this.items(page).map((it) => it.id));
    void this.commit(page, [...this.items(page), ...copies], (items) =>
      items.filter((it) => !before.has(it.id)).map((it) => it.id),
    );
  }

  copy() {
    this.clipboard = this.selected.map(clone);
  }

  /** Pastes copied items onto page `page` (in place, or moved a little on the same page). */
  paste(page: number) {
    if (!this.clipboard.length) return;
    const p = this.host.pageSize(page);
    const items = this.clipboard.map((it) => {
      const c = clone(it);
      c.id = 0;
      if (page === this.page) this.shift(c, 10 / p.w, 10 / p.h);
      return c;
    });
    const before = new Set(this.items(page).map((it) => it.id));
    void this.commit(page, [...this.items(page), ...items], (list) =>
      list.filter((it) => !before.has(it.id)).map((it) => it.id),
    );
  }

  /** Moves the chosen items in the drawing order. */
  order(op: "front" | "back" | "forward" | "backward") {
    if (this.page < 0 || !this.ids.size) return;
    const list = [...this.items(this.page)];
    const mine = list.filter((it) => this.ids.has(it.id));
    const rest = list.filter((it) => !this.ids.has(it.id));
    let out: ItemView[];
    if (op === "front") out = [...rest, ...mine];
    else if (op === "back") out = [...mine, ...rest];
    else {
      // One step past the nearest other item.
      out = [...list];
      const step = op === "forward" ? 1 : -1;
      const idx = out.map((it, k) => (this.ids.has(it.id) ? k : -1)).filter((k) => k >= 0);
      for (const k of step > 0 ? idx.reverse() : idx) {
        const j = k + step;
        if (j < 0 || j >= out.length || this.ids.has(out[j].id)) continue;
        [out[k], out[j]] = [out[j], out[k]];
      }
    }
    void this.commit(this.page, out);
  }

  group() {
    if (this.ids.size < 2) return;
    const g = Math.max(0, ...this.items(this.page).map((it) => it.group ?? 0)) + 1;
    this.update((it) => (it.group = g));
  }

  ungroup() {
    this.update((it) => (it.group = null));
  }

  /** Lines the chosen items up with each other (or with the page, for one item). */
  align(op: "left" | "center" | "right" | "top" | "middle" | "bottom") {
    const sel = this.selected;
    if (!sel.length) return;
    const box = sel.length > 1 ? this.bounds(sel) : [0, 0, 1, 1];
    this.update((it) => {
      const [l, t, r, b] = it.rect;
      const dx =
        op === "left"
          ? box[0] - l
          : op === "right"
            ? box[2] - r
            : op === "center"
              ? (box[0] + box[2]) / 2 - (l + r) / 2
              : 0;
      const dy =
        op === "top"
          ? box[1] - t
          : op === "bottom"
            ? box[3] - b
            : op === "middle"
              ? (box[1] + box[3]) / 2 - (t + b) / 2
              : 0;
      this.shift(it, dx, dy);
    });
  }

  /** Moves the chosen items by points. */
  nudge(dxPt: number, dyPt: number) {
    if (this.page < 0) return;
    const p = this.host.pageSize(this.page);
    this.update((it) => this.shift(it, dxPt / p.w, dyPt / p.h));
  }

  /** Keys for the chosen items; true if handled. */
  key(e: KeyboardEvent, page: number): boolean {
    const ctrl = e.ctrlKey || e.metaKey;
    if (ctrl && e.key.toLowerCase() === "v" && this.clipboard.length && !e.shiftKey) {
      this.paste(this.page >= 0 ? this.page : page);
      return true;
    }
    if (this.page < 0 || !this.ids.size) return false;
    const step = e.shiftKey ? 10 : 1;
    if (e.key === "Delete" || e.key === "Backspace") this.remove();
    else if (e.key === "Escape") this.clear();
    else if (e.key === "ArrowLeft") this.nudge(-step, 0);
    else if (e.key === "ArrowRight") this.nudge(step, 0);
    else if (e.key === "ArrowUp") this.nudge(0, -step);
    else if (e.key === "ArrowDown") this.nudge(0, step);
    else if (e.key === "Enter" && this.selected.length === 1) this.open(this.page, this.selected[0]);
    else if (ctrl && e.key.toLowerCase() === "d") this.duplicate();
    else if (ctrl && e.key.toLowerCase() === "c") this.copy();
    else if (ctrl && e.key.toLowerCase() === "x") {
      this.copy();
      this.remove();
    } else if (ctrl && e.key.toLowerCase() === "g") {
      if (e.shiftKey) this.ungroup();
      else this.group();
    } else if (ctrl && e.key === "]") this.order(e.shiftKey ? "front" : "forward");
    else if (ctrl && e.key === "[") this.order(e.shiftKey ? "back" : "backward");
    else return false;
    return true;
  }

  // ------------------------------------------------------------ drawing

  /** Draws page `i`'s frames, handles, guides and the item being made. */
  draw(i: number) {
    const layer = this.host.layer(i);
    if (!layer) return;
    for (const el of [...layer.children]) if (!el.classList.contains("iedit")) el.remove();
    const g = this.gesture;
    const box = (r: number[], cls: string) => {
      const el = document.createElement("div");
      el.className = cls;
      el.style.left = pct(r[0]);
      el.style.top = pct(r[1]);
      el.style.width = pct(r[2] - r[0]);
      el.style.height = pct(r[3] - r[1]);
      layer.append(el);
      return el;
    };
    if (i === this.page) {
      const solo = this.resizable();
      for (const it of this.selected) {
        let r = it.rect;
        if (g?.type === "move" && g.page === i) r = [r[0] + g.dx, r[1] + g.dy, r[2] + g.dx, r[3] + g.dy];
        if (g?.type === "resize" && g.id === it.id) r = g.rect;
        box(r, g?.type === "move" && g.moved ? "isel moving" : "isel");
        if (solo && solo.id === it.id) {
          for (const [name, x, y] of this.handles(r)) {
            const hd = document.createElement("div");
            hd.className = `ihandle ${name}`;
            hd.style.left = pct(x);
            hd.style.top = pct(y);
            layer.append(hd);
          }
        }
      }
    }
    if (g?.type === "create" && g.page === i) {
      const [a, b] = [g.a, g.b];
      if (g.tool === "iline" || g.tool === "iarrow") {
        const ns = "http://www.w3.org/2000/svg";
        const svg = document.createElementNS(ns, "svg");
        svg.setAttribute("viewBox", "0 0 1 1");
        svg.setAttribute("preserveAspectRatio", "none");
        svg.classList.add("ipreview");
        const ln = document.createElementNS(ns, "line");
        for (const [k, v] of Object.entries({ x1: a[0], y1: a[1], x2: b[0], y2: b[1] }))
          ln.setAttribute(k, String(v));
        ln.setAttribute("vector-effect", "non-scaling-stroke");
        svg.append(ln);
        layer.append(svg);
      } else {
        box(
          [Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.max(a[0], b[0]), Math.max(a[1], b[1])],
          "icreate",
        );
      }
    }
    if (g && g.page === i) {
      for (const x of this.guides.x) box([x, 0, x, 1], "iguide v");
      for (const y of this.guides.y) box([0, y, 1, y], "iguide h");
    }
  }
}
