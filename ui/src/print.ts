// Printing. Pages are rendered by the engine at print resolution, turned
// into PNG images in a print-only container (#print-root, see app.css),
// and the WebView's print dialog prints them, one page image per sheet,
// scaled to fit. The dialog chooses the printer, copies and paper.

import { Cancelled, type RenderSpec, type Transport } from "./api";
import type { Viewer } from "./viewer";

/** The engine's largest render side; bigger pages are printed at a lower resolution. */
const MAX_SIDE = 4096;
/** Below visible tiles, above previews and background work. */
const PRIO_PRINT = 900;
const RETRIES = 3;

/**
 * Parses "1-5, 8, 10-" into 0-based page indices, in order, without
 * repeats. Returns null if any part is not a page between 1 and `count`.
 */
export function parseRange(text: string, count: number): number[] | null {
  const out: number[] = [];
  const seen = new Set<number>();
  const parts = text
    .split(/[,،;]/)
    .map((p) => p.trim())
    .filter(Boolean);
  if (!parts.length) return null;
  for (const part of parts) {
    const m = /^(\d*)\s*[-–]\s*(\d*)$/.exec(part) ?? /^(\d+)$/.exec(part);
    if (!m) return null;
    const a = m[1] === "" ? 1 : Number(m[1]);
    const b = m.length > 2 ? (m[2] === "" ? count : Number(m[2])) : a;
    if (!(a >= 1 && b >= a && b <= count)) return null;
    for (let p = a; p <= b; p++) {
      if (!seen.has(p)) {
        seen.add(p);
        out.push(p - 1);
      }
    }
  }
  return out;
}

function printRoot(): HTMLDivElement {
  let root = document.getElementById("print-root") as HTMLDivElement | null;
  if (!root) {
    root = document.createElement("div");
    root.id = "print-root";
    document.body.append(root);
  }
  return root;
}

/** Removes the prepared pages and frees their images. */
export function clearPrint() {
  const root = document.getElementById("print-root");
  if (!root) return;
  for (const img of root.querySelectorAll("img")) URL.revokeObjectURL(img.src);
  root.replaceChildren();
}

async function renderPage(v: Viewer, transport: Transport, page: number, dpi: number): Promise<Blob> {
  const p = v.info.pages[page];
  let w = Math.max(1, Math.round((p.w / 72) * dpi));
  let h = Math.max(1, Math.round((p.h / 72) * dpi));
  const k = Math.min(1, MAX_SIDE / Math.max(w, h));
  w = Math.max(1, Math.floor(w * k));
  h = Math.max(1, Math.floor(h * k));
  const spec: RenderSpec = { doc: v.info.doc, page, page_w: w, page_h: h, x: 0, y: 0, w, h, rotation: 0 };
  for (let attempt = 0; ; attempt++) {
    try {
      const t = await transport(spec, PRIO_PRINT);
      const c = document.createElement("canvas");
      c.width = t.w;
      c.height = t.h;
      c.getContext("2d", { alpha: false })!.putImageData(new ImageData(t.rgba, t.w, t.h), 0, 0);
      t.release?.();
      const blob = await new Promise<Blob | null>((r) => c.toBlob(r, "image/png"));
      c.width = c.height = 0;
      if (!blob) throw new Error("could not encode the page");
      return blob;
    } catch (e) {
      // The viewer cancels renders it no longer needs; a print render can
      // be caught up in that. Ask again.
      if (!(e instanceof Cancelled) || attempt >= RETRIES) throw e;
    }
  }
}

/**
 * Prepares `pages` and opens the print dialog. `progress` is called after
 * each page; `cancelled` is checked between pages.
 */
export async function printPages(
  v: Viewer,
  transport: Transport,
  pages: number[],
  dpi: number,
  progress: (done: number) => void,
  cancelled: () => boolean,
): Promise<boolean> {
  clearPrint();
  const root = printRoot();
  // Two pages in flight: one renders while the other is encoded.
  let next: Promise<Blob> | null = pages.length ? renderPage(v, transport, pages[0], dpi) : null;
  for (let i = 0; i < pages.length; i++) {
    const blob = await next!;
    next = i + 1 < pages.length ? renderPage(v, transport, pages[i + 1], dpi) : null;
    if (cancelled()) {
      await next?.catch(() => {});
      clearPrint();
      return false;
    }
    const sheet = document.createElement("div");
    const img = document.createElement("img");
    img.alt = "";
    img.src = URL.createObjectURL(blob);
    sheet.append(img);
    root.append(sheet);
    progress(i + 1);
  }
  // Every image must be decoded before the print preview snapshots the page.
  await Promise.all([...root.querySelectorAll("img")].map((img) => img.decode().catch(() => {})));
  window.addEventListener("afterprint", clearPrint, { once: true });
  window.print();
  return true;
}
