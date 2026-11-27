<script lang="ts">
  // Organise pages (PRD 5.3): every page as a thumbnail. Click, Ctrl-click
  // and Shift-click select; drag to move; the bar above turns, copies,
  // deletes, inserts, crops, extracts and splits. Changes go through the
  // document's editing session (undo, save) like any other edit; after
  // each one the document comes back with a new viewer.
  import { onDestroy, tick, untrack } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import { t } from "./i18n.svelte";
  import type { Viewer } from "./viewer";

  export type InsertKind = "blank" | "images" | "pdf";

  interface Props {
    viewer: Viewer;
    /** Moves `pages` to just before index `before`. */
    onmove: (pages: number[], before: number) => Promise<void>;
    onrotate: (pages: number[], quarters: number) => Promise<void>;
    ondelete: (pages: number[]) => Promise<void>;
    onduplicate: (pages: number[]) => Promise<void>;
    oninsert: (kind: InsertKind, at: number) => Promise<void>;
    onextract: (pages: number[]) => void;
    oncrop: (pages: number[]) => void;
    onsplit: () => void;
    onheader: (pages: number[]) => void;
    /** Leaves the grid, showing page `page`. */
    ondone: (page: number) => void;
    /** The page to start at. */
    start: number;
  }
  let {
    viewer,
    onmove,
    onrotate,
    ondelete,
    onduplicate,
    oninsert,
    onextract,
    oncrop,
    onsplit,
    onheader,
    ondone,
    start,
  }: Props = $props();

  const CELL = 150; // thumbnail box, CSS px
  const selected = new SvelteSet<number>();
  let anchor = 0; // where Shift-click ranges start
  let focus = $state(untrack(() => start));
  let grid: HTMLDivElement | null = $state(null);
  let insertOpen = $state(false);

  // Thumbnails, by page index of the current viewer.
  const urls = new SvelteMap<number, string>();
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const asked = new Set<number>();
  let shownFor: Viewer | null = null;
  /**
   * How the last change moved pages (old index -> new index, or null for a
   * page drawn anew), so thumbnails can be kept when the new viewer comes.
   */
  let remap: ((old: number, pages: number) => number | null) | null = null;

  const count = $derived(viewer.info.page_count);
  const pageList = $derived([...Array(count).keys()]);
  const sizes = $derived(viewer.info.pages);

  // A new viewer: carry thumbnails over where the change says how, else start again.
  $effect(() => {
    const v = viewer;
    untrack(() => {
      if (shownFor === v) return;
      const old = new Map(urls);
      urls.clear();
      asked.clear();
      const r = shownFor ? remap : null;
      for (const [i, u] of old) {
        const to = r ? r(i, v.info.page_count) : null;
        if (to !== null && to >= 0 && to < v.info.page_count) {
          urls.set(to, u);
          asked.add(to);
        } else URL.revokeObjectURL(u);
      }
      remap = null;
      shownFor = v;
      for (const s of [...selected]) if (s >= v.info.page_count) selected.delete(s);
      focus = Math.min(focus, v.info.page_count - 1);
      observeAll();
    });
  });

  let observer: IntersectionObserver | null = null;
  function load(i: number) {
    if (asked.has(i)) return;
    asked.add(i);
    const v = viewer;
    void v
      .previewBlob(i)
      .then((b) => {
        if (v === viewer) urls.set(i, URL.createObjectURL(b));
      })
      .catch(() => asked.delete(i));
  }

  async function observeAll() {
    await tick();
    observer?.disconnect();
    if (!grid) return;
    observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) if (e.isIntersecting) load(Number((e.target as HTMLElement).dataset.page));
      },
      { root: grid, rootMargin: "400px" },
    );
    for (const el of grid.querySelectorAll<HTMLElement>(".cell")) observer.observe(el);
  }

  $effect(() => {
    if (grid) {
      untrack(() => {
        observeAll();
        const el = grid?.querySelector<HTMLElement>(`[data-page="${focus}"]`);
        el?.scrollIntoView({ block: "center" });
        grid?.focus();
      });
    }
  });

  onDestroy(() => {
    observer?.disconnect();
    for (const u of urls.values()) URL.revokeObjectURL(u);
  });

  const box = (i: number) => {
    const p = sizes[i] ?? { w: 1, h: 1 };
    const k = Math.min(CELL / p.w, CELL / p.h);
    return { w: Math.round(p.w * k), h: Math.round(p.h * k) };
  };

  const chosen = $derived([...selected].sort((a, b) => a - b));
  /** The selected pages, or the focused one if none is selected. */
  const targets = () => (chosen.length ? chosen : [focus]);

  function click(e: MouseEvent, i: number) {
    focus = i;
    if (e.shiftKey) {
      const [a, b] = [Math.min(anchor, i), Math.max(anchor, i)];
      if (!e.ctrlKey) selected.clear();
      for (let k = a; k <= b; k++) selected.add(k);
      return;
    }
    anchor = i;
    if (e.ctrlKey) {
      if (selected.has(i)) selected.delete(i);
      else selected.add(i);
    } else {
      selected.clear();
      selected.add(i);
    }
  }

  // ------------------------------------------------------------- changes

  let working = $state(false);
  async function run(fn: () => Promise<void>, how: ((old: number, pages: number) => number | null) | null) {
    if (working) return;
    working = true;
    remap = how;
    try {
      await fn();
    } finally {
      working = false;
    }
  }

  /** Moves `pages` to before `before`, keeping them selected. */
  function move(pages: number[], before: number) {
    const set = new Set(pages);
    const kept = [...Array(count).keys()].filter((i) => !set.has(i));
    const at = kept.filter((i) => i < before).length;
    const order = [...kept.slice(0, at), ...pages, ...kept.slice(at)];
    const where = new Map(order.map((old, i) => [old, i]));
    if (order.every((old, i) => old === i)) return;
    void run(
      () => onmove(pages, before),
      (old) => where.get(old) ?? null,
    ).then(() => {
      selected.clear();
      for (let k = 0; k < pages.length; k++) selected.add(at + k);
      focus = at;
    });
  }

  function rotate(q: number) {
    const set = new Set(targets());
    void run(
      () => onrotate([...set], q),
      (old) => (set.has(old) ? null : old),
    );
  }

  function remove() {
    const pages = targets();
    if (pages.length >= count) return;
    const set = new Set(pages);
    void run(
      () => ondelete(pages),
      (old) => (set.has(old) ? null : old - pages.filter((p) => p < old).length),
    ).then(() => {
      selected.clear();
      focus = Math.min(pages[0], count - 1);
    });
  }

  function duplicate() {
    const pages = targets();
    void run(
      () => onduplicate(pages),
      (old) => old + pages.filter((p) => p < old).length,
    );
  }

  /** New pages go after the last selected page, or at the end. */
  const insertAt = () => (chosen.length ? chosen[chosen.length - 1] + 1 : count);
  function insert(kind: InsertKind) {
    insertOpen = false;
    const at = insertAt();
    const before = count;
    void run(
      () => oninsert(kind, at),
      // Pages at or after `at` move down by however many came in.
      (old, pages) => (old < at ? old : old + pages - before),
    ).then(() => {
      const added = viewer.info.page_count - before;
      if (added > 0) {
        selected.clear();
        for (let k = 0; k < added; k++) selected.add(at + k);
        focus = at;
      }
    });
  }

  // --------------------------------------------------------------- drag

  let dragFrom: number[] | null = null;
  let dropAt: number | null = $state(null); // insertion index under the pointer

  function dragStart(e: DragEvent, i: number) {
    if (!selected.has(i)) {
      selected.clear();
      selected.add(i);
      anchor = i;
    }
    dragFrom = chosen.length ? [...chosen] : [i];
    e.dataTransfer?.setData("text/plain", dragFrom.map((p) => p + 1).join(","));
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }

  function dragOver(e: DragEvent, i: number) {
    if (!dragFrom) return;
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    const r = el.getBoundingClientRect();
    const rtl = getComputedStyle(el).direction === "rtl";
    const firstHalf = rtl ? e.clientX > r.left + r.width / 2 : e.clientX < r.left + r.width / 2;
    dropAt = firstHalf ? i : i + 1;
  }

  function drop(e: DragEvent) {
    e.preventDefault();
    const from = dragFrom;
    const at = dropAt;
    dragFrom = null;
    dropAt = null;
    if (from && at !== null) move(from, at);
  }

  function dragEnd() {
    dragFrom = null;
    dropAt = null;
  }

  // --------------------------------------------------------------- keys

  function columns(): number {
    const cells = grid?.querySelectorAll<HTMLElement>(".cell");
    if (!cells || cells.length < 2) return 1;
    const top = cells[0].offsetTop;
    let n = 0;
    for (const c of cells) {
      if (c.offsetTop !== top) break;
      n++;
    }
    return Math.max(1, n);
  }

  function key(e: KeyboardEvent) {
    const rtl = grid ? getComputedStyle(grid).direction === "rtl" : false;
    const step: Record<string, number> = {
      ArrowRight: rtl ? -1 : 1,
      ArrowLeft: rtl ? 1 : -1,
      ArrowDown: columns(),
      ArrowUp: -columns(),
      Home: -count,
      End: count,
    };
    if (e.key in step) {
      e.preventDefault();
      const to = Math.max(0, Math.min(count - 1, focus + step[e.key]));
      if (e.ctrlKey && !e.shiftKey && chosen.length && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
        // Ctrl+arrow moves the selected pages one place.
        const dir = step[e.key];
        const before = dir < 0 ? Math.max(0, chosen[0] - 1) : Math.min(count, chosen[chosen.length - 1] + 2);
        move(chosen, before);
        return;
      }
      if (e.shiftKey) {
        selected.clear();
        for (let k = Math.min(anchor, to); k <= Math.max(anchor, to); k++) selected.add(k);
      } else {
        selected.clear();
        selected.add(to);
        anchor = to;
      }
      focus = to;
      grid?.querySelector<HTMLElement>(`[data-page="${to}"]`)?.scrollIntoView({ block: "nearest" });
      return;
    }
    if (e.ctrlKey && e.key.toLowerCase() === "a") {
      e.preventDefault();
      for (let k = 0; k < count; k++) selected.add(k);
      return;
    }
    if (e.key === "Delete") {
      e.preventDefault();
      remove();
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      ondone(focus);
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      if (selected.size) selected.clear();
      else ondone(focus);
    }
  }
</script>

<div class="organize">
  <div class="obar" role="toolbar" aria-label={t("org.label")}>
    <button class="primary" onclick={() => ondone(focus)} title={t("org.doneTitle")}>{t("org.done")}</button>
    <span class="sep"></span>
    <button
      onclick={() => rotate(-1)}
      disabled={working}
      title={t("org.rotateLeft")}
      aria-label={t("org.rotateLeft")}
    >
      ⟲
    </button>
    <button
      onclick={() => rotate(1)}
      disabled={working}
      title={t("org.rotateRight")}
      aria-label={t("org.rotateRight")}
    >
      ⟳
    </button>
    <button onclick={duplicate} disabled={working} title={t("org.duplicateTitle")}>
      {t("org.duplicate")}
    </button>
    <button onclick={remove} disabled={working || targets().length >= count} title={t("org.deleteTitle")}>
      {t("org.delete")}
    </button>
    <span class="sep"></span>
    <span class="menu">
      <button
        onclick={() => (insertOpen = !insertOpen)}
        aria-expanded={insertOpen}
        aria-haspopup="menu"
        disabled={working}
        title={t("org.insertTitle")}
      >
        {t("org.insert")} ▾
      </button>
      {#if insertOpen}
        <div class="menu-list" role="menu">
          <button role="menuitem" onclick={() => insert("blank")}>{t("org.insertBlank")}</button>
          <button role="menuitem" onclick={() => insert("images")}>{t("org.insertImages")}</button>
          <button role="menuitem" onclick={() => insert("pdf")}>{t("org.insertPdf")}</button>
        </div>
      {/if}
    </span>
    <button onclick={() => oncrop(targets())} disabled={working} title={t("org.cropTitle")}>
      {t("org.crop")}
    </button>
    <button onclick={() => onheader(targets())} disabled={working} title={t("org.headerTitle")}>
      {t("org.header")}
    </button>
    <span class="sep"></span>
    <button onclick={() => onextract(targets())} disabled={working} title={t("org.extractTitle")}>
      {t("org.extract")}
    </button>
    <button onclick={onsplit} disabled={working} title={t("org.splitTitle")}>{t("org.split")}</button>
    <span class="grow"></span>
    <span class="count" role="status">
      {chosen.length
        ? t("org.selected", { n: chosen.length, total: count })
        : t("org.pages", { total: count })}
    </span>
  </div>
  <div
    class="grid"
    bind:this={grid}
    tabindex="0"
    role="listbox"
    aria-multiselectable="true"
    aria-label={t("org.pagesLabel")}
    onkeydown={key}
    ondrop={drop}
    ondragover={(e) => dragFrom && e.preventDefault()}
  >
    {#each pageList as i (i)}
      {@const b = box(i)}
      <div
        class="cell"
        class:selected={selected.has(i)}
        class:focus={focus === i}
        class:drop-before={dropAt === i}
        class:drop-after={dropAt === i + 1 && i === count - 1}
        data-page={i}
        role="option"
        aria-selected={selected.has(i)}
        tabindex="-1"
        draggable="true"
        onclick={(e) => click(e, i)}
        ondblclick={() => ondone(i)}
        onkeydown={() => {}}
        ondragstart={(e) => dragStart(e, i)}
        ondragover={(e) => dragOver(e, i)}
        ondragend={dragEnd}
      >
        <div class="thumb" style:width="{b.w}px" style:height="{b.h}px">
          {#if urls.get(i)}<img src={urls.get(i)} alt="" draggable="false" />{/if}
        </div>
        <span class="num">{i + 1}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .organize {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    z-index: 5;
  }
  .obar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 8px;
    background: var(--bar);
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .sep {
    width: 1px;
    height: 20px;
    background: var(--border);
    margin: 0 4px;
  }
  .grow {
    flex: 1;
  }
  .count {
    color: var(--muted);
    font-size: 13px;
  }
  .menu {
    position: relative;
  }
  .menu-list {
    position: absolute;
    top: 100%;
    inset-inline-start: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    min-width: 200px;
    background: var(--control);
    border: 1px solid var(--border);
    box-shadow: 0 4px 12px var(--popup-shadow);
    border-radius: 4px;
    padding: 4px;
  }
  .menu-list button {
    text-align: start;
    border: none;
    background: none;
  }
  .menu-list button:hover {
    background: var(--control-hover);
  }
  .grid {
    flex: 1;
    overflow: auto;
    display: flex;
    flex-wrap: wrap;
    align-content: flex-start;
    gap: 8px;
    padding: 16px;
    outline: none;
  }
  .cell {
    width: 170px;
    height: 190px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-end;
    gap: 4px;
    padding: 6px 0;
    border-radius: 6px;
    border: 2px solid transparent;
    position: relative;
    cursor: default;
  }
  .cell.selected {
    background: var(--selected);
  }
  .cell.focus {
    border-color: var(--accent);
  }
  .cell.drop-before::before,
  .cell.drop-after::after {
    content: "";
    position: absolute;
    top: 4px;
    bottom: 4px;
    width: 3px;
    background: var(--accent);
    border-radius: 2px;
  }
  .cell.drop-before::before {
    inset-inline-start: -7px;
  }
  .cell.drop-after::after {
    inset-inline-end: -7px;
  }
  .thumb {
    background: var(--paper);
    box-shadow: 0 1px 3px var(--page-shadow);
    flex: none;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    display: block;
  }
  .num {
    font-size: 12px;
    color: var(--muted);
  }
</style>
