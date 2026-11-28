// Calls into the Rust side.

import { Channel, invoke } from "@tauri-apps/api/core";

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

// ------------------------------------------------------------------ text

export interface PageText {
  /** The page's characters in reading order, one UTF-16 unit each. */
  text: string;
  /** Four numbers per character: left, top, right, bottom as page fractions; zeros for inserted spaces and line breaks. */
  boxes: number[];
}

export const pageText = (doc: number, page: number) => invoke<PageText>("page_text", { doc, page });

export interface SearchOptions {
  match_case: boolean;
  whole_word: boolean;
  /** Ignore Arabic diacritics and tatweel. */
  ignore_diacritics: boolean;
  /** أ إ آ = ا, ى = ي, ة = ه, Persian ی ک, and Arabic-Indic digits = 0-9. */
  letter_variants: boolean;
}

export type SearchEvent =
  | { kind: "batch"; results: [number, [number, number][]][]; searched: number }
  | { kind: "done"; searched: number }
  | { kind: "error"; message: string };

/** Starts a search; events arrive on `onEvent`. Returns its id, for `cancelSearch`. */
export function search(
  doc: number,
  query: string,
  options: SearchOptions,
  startPage: number,
  pageCount: number,
  onEvent: (e: SearchEvent) => void,
): Promise<number> {
  const channel = new Channel<SearchEvent>();
  channel.onmessage = onEvent;
  return invoke<number>("search", {
    request: { doc, query, options, start_page: startPage, page_count: pageCount },
    onEvent: channel,
  });
}

export const cancelSearch = (id: number) => invoke<void>("cancel_search", { id });

// ---------------------------------------------------------------- editing

export type MarkupKind = "highlight" | "underline" | "strikeout" | "squiggly";

export interface EditStatus {
  dirty: boolean;
  can_undo: boolean;
  can_redo: boolean;
}

/** A change's result: the new undo state and the pages to draw again. */
export interface Changed {
  status: EditStatus;
  pages: number[];
  id: number | null;
  /** The new page list after pages were added, removed, moved, turned or cropped. */
  info: DocInfo | null;
}

export interface Saved {
  status: EditStatus;
  path: string;
  ms: number;
}

export const annotRemove = (doc: number, id: number) => invoke<Changed>("annot_remove", { doc, id });
export const editUndo = (doc: number) => invoke<Changed>("edit_undo", { doc });
export const editRedo = (doc: number) => invoke<Changed>("edit_redo", { doc });
export const saveDoc = (doc: number, path: string | null = null) => invoke<Saved>("save_doc", { doc, path });
export const userName = () => invoke<string>("user_name");
export const recoveryOffer = (doc: number) => invoke<number | null>("recovery_offer", { doc });
export const recoveryRestore = (doc: number) => invoke<Changed>("recovery_restore", { doc });
export const recoveryDiscard = (doc: number) => invoke<void>("recovery_discard", { doc });

// ------------------------------------------------------------------ forms

export type FieldValue =
  | { type: "text"; value: string }
  | { type: "state"; value: string | null }
  | { type: "choice"; value: string[] };

/** A form field; `boxes` are its widgets (page fractions: left, top, right, bottom). */
export interface FormField {
  id: [number, number];
  name: string;
  tooltip: string;
  kind: "text" | "check" | "radio" | "combo" | "list" | "other";
  read_only: boolean;
  required: boolean;
  multiline: boolean;
  password: boolean;
  comb: boolean;
  editable: boolean;
  multi_select: boolean;
  no_toggle_off: boolean;
  max_len: number | null;
  options: [string, string][];
  value: FieldValue;
  font_size: number;
  color: number[];
  align: number;
  boxes: { page: number; rect: number[]; on: string | null }[];
}

export const flattenDoc = (doc: number) => invoke<Changed>("flatten_doc", { doc });
/** Saved signatures: PNG data URLs. */
export const loadSignatures = () => invoke<string[]>("load_signatures");
export const saveSignatures = (list: string[]) => invoke<void>("save_signatures", { list });
export const pictureDataUrl = (path: string) => invoke<string>("picture_data_url", { path });
export const formFields = (doc: number) => invoke<FormField[]>("form_fields", { doc });
export const fieldSet = (doc: number, id: [number, number], value: FieldValue) =>
  invoke<Changed>("field_set", { doc, id, value });

// ------------------------------------------------------------------ pages

export const pagesMove = (doc: number, pages: number[], before: number) =>
  invoke<Changed>("pages_move", { doc, pages, before });
export const pagesRotate = (doc: number, pages: number[], quarters: number) =>
  invoke<Changed>("pages_rotate", { doc, pages, quarters });
export const pagesDelete = (doc: number, pages: number[]) => invoke<Changed>("pages_delete", { doc, pages });
export const pagesDuplicate = (doc: number, pages: number[]) =>
  invoke<Changed>("pages_duplicate", { doc, pages });
export const pagesInsertBlank = (doc: number, at: number, w: number, h: number, count: number) =>
  invoke<Changed>("pages_insert_blank", { doc, at, w, h, count });
export const pagesInsertImages = (doc: number, at: number, paths: string[]) =>
  invoke<Changed>("pages_insert_images", { doc, at, paths });
export const pagesInsertPdf = (doc: number, at: number, path: string, pages: number[] | null = null) =>
  invoke<Changed>("pages_insert_pdf", { doc, at, path, pages });
/** Margins as fractions of the shown page (left, top, right, bottom); null removes the crop. */
export const pagesCrop = (doc: number, crops: { page: number; margins: number[] | null }[]) =>
  invoke<Changed>("pages_crop", { doc, crops });
/** Page numbers, headers, footers and a watermark; `{n}` and `{total}` are filled in per page. */
export interface Decor {
  header: [string, string, string];
  footer: [string, string, string];
  size: number;
  color: number[];
  /** From the page edge, in points. */
  margin: number;
  /** Added to a page's position (1 for the first page) to give {n}. */
  offset: number;
  arabic_digits: boolean;
  watermark: string;
  watermark_size: number;
  watermark_color: number[];
  watermark_opacity: number;
}
export const pagesDecorate = (doc: number, pages: number[], decor: Decor) =>
  invoke<Changed>("pages_decorate", { doc, pages, decor });
export const pagesUndecorate = (doc: number, pages: number[]) =>
  invoke<Changed>("pages_undecorate", { doc, pages });
export const pagesExport = (doc: number, pages: number[], path: string) =>
  invoke<string>("pages_export", { doc, pages, path });
export const pagesSplit = (doc: number, parts: { pages: number[]; name: string }[], folder: string) =>
  invoke<string[]>("pages_split", { doc, parts, folder });
export const mergeFiles = (paths: string[], target: string) =>
  invoke<string>("merge_files", { paths, target });
/** A new PDF of pictures, one per page; `size` in points, or null for pages the shape of each picture. */
export const imagesToPdf = (paths: string[], target: string, size: number[] | null) =>
  invoke<string>("images_to_pdf", { paths, target, size });

export type AnnotKind =
  | "highlight"
  | "underline"
  | "strikeout"
  | "squiggly"
  | "ink"
  | "line"
  | "square"
  | "circle"
  | "note"
  | "freetext"
  | "stamp"
  | "other";

/** An annotation, geometry in page fractions (top-left origin, /Rotate applied). */
export interface AnnotView {
  id: number;
  page: number;
  kind: AnnotKind;
  subtype: string;
  /** Left, top, right, bottom. */
  rect: number[];
  line: number[] | null;
  color: number[];
  fill: number[] | null;
  opacity: number;
  width: number;
  arrow: boolean;
  contents: string;
  author: string;
  modified: string;
  font_size: number;
  label: string;
  picture: boolean;
}

export interface NewAnnot {
  page: number;
  kind: AnnotKind;
  rects?: number[][];
  ink?: number[][][];
  line?: number[];
  rect?: number[];
  color: number[];
  fill?: number[] | null;
  opacity?: number;
  width?: number;
  arrow?: boolean;
  author?: string;
  contents?: string;
  font_size?: number;
  label?: string;
  image_path?: string;
  /** Or the picture itself as a data URL (signatures). */
  image_data?: string;
}

export interface AnnotStyle {
  color?: number[];
  fill?: number[] | null;
  width?: number;
  opacity?: number;
  contents?: string;
  font_size?: number;
}

export const annotsAll = (doc: number) => invoke<AnnotView[]>("annots_all", { doc });
export const annotsPage = (doc: number, page: number) => invoke<AnnotView[]>("annots_page", { doc, page });
export const annotAdd = (doc: number, annot: NewAnnot) => invoke<Changed>("annot_add", { doc, annot });
export const annotMove = (doc: number, id: number, dx: number, dy: number) =>
  invoke<Changed>("annot_move", { doc, id, dx, dy });
export const annotResize = (doc: number, id: number, rect: number[]) =>
  invoke<Changed>("annot_resize", { doc, id, rect });
export const annotStyle = (doc: number, id: number, style: AnnotStyle) =>
  invoke<Changed>("annot_style", { doc, id, style });

export interface OcrOptions {
  /** Keep diacritics (tashkeel). */
  diacritics: boolean;
  /** "keep" as printed, "western" 0-9, "arabic" Arabic-Indic. */
  digits: "keep" | "western" | "arabic";
}

/** Progress of a text recognition run (`ocr-progress` events). */
export interface OcrProgress {
  doc: number;
  run: number;
  done: number;
  /** Pages left alone because they already had text. */
  skipped: number;
  total: number;
  /** Pages whose text was just added. */
  pages: number[];
  status: EditStatus | null;
  finished: boolean;
  error: string | null;
}

export const ocrAvailable = () => invoke<boolean>("ocr_available");
export const ocrStart = (
  doc: number,
  pages: number[],
  first: number[],
  options: OcrOptions,
  skipText: boolean,
) => invoke<number>("ocr_start", { doc, pages, first, options, skipText });
export const ocrFirst = (doc: number, pages: number[]) => invoke<void>("ocr_first", { doc, pages });
export const ocrCancel = (doc: number) => invoke<void>("ocr_cancel", { doc });
export const ocrRegion = (doc: number, page: number, rect: number[], options: OcrOptions) =>
  invoke<string>("ocr_region", { doc, page, rect, options });
