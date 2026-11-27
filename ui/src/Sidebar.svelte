<script lang="ts">
  // Side panel: page thumbnails and the bookmark tree for the active tab.
  import { onDestroy, tick } from "svelte";
  import { SvelteMap, SvelteSet } from "svelte/reactivity";
  import type { AnnotView, OutlineItem } from "./api";
  import { lang, t } from "./i18n.svelte";
  import type { Viewer } from "./viewer";

  interface Props {
    viewer: Viewer;
    /** Current page, 0-based. */
    page: number;
    outline: OutlineItem[] | null;
    panel: "pages" | "bookmarks" | "comments";
    onpanel: (p: "pages" | "bookmarks" | "comments") => void;
    /** Every annotation of the document; null while loading. */
    comments: AnnotView[] | null;
    selected: number | null;
    oncomment: (a: AnnotView) => void;
  }
  let { viewer, page, outline, panel, onpanel, comments, selected, oncomment }: Props = $props();

  const css = (c: number[]) => `rgb(${c.map((v) => Math.round(v * 255)).join(" ")})`;
  /** "D:20261127101500Z" as a short local date. */
  function when(d: string): string {
    const m = /^D:(\d{4})(\d{2})(\d{2})(\d{2})?(\d{2})?/.exec(d);
    if (!m) return "";
    const date = new Date(Date.UTC(+m[1], +m[2] - 1, +m[3], +(m[4] ?? 0), +(m[5] ?? 0)));
    return date.toLocaleDateString(lang());
  }

  // ------------------------------------------------------------ thumbnails
  const THUMB_W = 132; // CSS px
  const LABEL_H = 26;
  const PAD = 10;
  const MAX_URLS = 400;

  let list: HTMLDivElement | null = $state(null);
  let scrollTop = $state(0);
  let listHeight = $state(0);
  const urls = new SvelteMap<number, string>();
  // Bookkeeping only; nothing renders from it.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  const asked = new Set<number>();

  const heights = $derived(viewer.info.pages.map((p) => Math.round((THUMB_W * p.h) / p.w) + LABEL_H + PAD));
  const offsets = $derived.by(() => {
    const out = [0];
    for (const h of heights) out.push(out[out.length - 1] + h);
    return out;
  });
  const total = $derived(offsets[offsets.length - 1]);

  /** First item whose bottom is below `y`. */
  function indexAt(y: number) {
    let lo = 0;
    let hi = heights.length - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (offsets[mid + 1] <= y) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  const visible = $derived.by(() => {
    if (!heights.length) return [];
    const first = indexAt(Math.max(0, scrollTop - 200));
    const out: number[] = [];
    for (let i = first; i < heights.length && offsets[i] < scrollTop + listHeight + 200; i++) out.push(i);
    return out;
  });

  // Ask for images of the thumbnails in view.
  $effect(() => {
    for (const i of visible) {
      if (urls.has(i) || asked.has(i)) continue;
      asked.add(i);
      void viewer.previewBlob(i).then((b) => {
        if (urls.size >= MAX_URLS) {
          // Drop the oldest, unless it is on screen.
          for (const [k, u] of urls) {
            if (visible.includes(k)) continue;
            URL.revokeObjectURL(u);
            urls.delete(k);
            asked.delete(k);
            break;
          }
        }
        urls.set(i, URL.createObjectURL(b));
      });
    }
  });

  // Keep the current page's thumbnail in view while reading.
  let userScrolledAt = 0;
  $effect(() => {
    const p = page;
    if (panel !== "pages" || !list || performance.now() - userScrolledAt < 1500) return;
    const top = offsets[p];
    const bottom = offsets[p + 1];
    if (top < list.scrollTop || bottom > list.scrollTop + list.clientHeight) {
      list.scrollTop = top - (list.clientHeight - (bottom - top)) / 2;
    }
  });

  onDestroy(() => {
    for (const u of urls.values()) URL.revokeObjectURL(u);
  });

  // ------------------------------------------------------------- bookmarks
  const expanded = new SvelteSet<string>();

  /** Flattened bookmarks with a page, in document order, for "where am I". */
  const flat = $derived.by(() => {
    const out: { id: string; page: number }[] = [];
    const walk = (items: OutlineItem[], prefix: string) =>
      items.forEach((it, k) => {
        const id = `${prefix}${k}`;
        if (it.target?.kind === "page") out.push({ id, page: it.target.page });
        walk(it.children, `${id}.`);
      });
    walk(outline ?? [], "");
    return out;
  });

  /** The last bookmark at or before the current page. */
  const currentBookmark = $derived.by(() => {
    let best: string | null = null;
    let bestPage = -1;
    for (const b of flat) {
      if (b.page <= page && b.page >= bestPage) {
        best = b.id;
        bestPage = b.page;
      }
    }
    return best;
  });

  function toggle(id: string) {
    if (expanded.has(id)) expanded.delete(id);
    else expanded.add(id);
  }

  async function showPanel(p: "pages" | "bookmarks" | "comments") {
    onpanel(p);
    await tick();
  }
</script>

{#snippet tree(items: OutlineItem[], prefix: string, depth: number)}
  <ul role={depth === 0 ? "tree" : "group"}>
    {#each items as item, k (k)}
      {@const id = `${prefix}${k}`}
      {@const open = expanded.has(id)}
      <li
        role="treeitem"
        aria-expanded={item.children.length ? open : undefined}
        aria-selected={id === currentBookmark}
      >
        <div
          class="row"
          class:current={id === currentBookmark}
          style:padding-inline-start="{depth * 14 + 4}px"
        >
          {#if item.children.length}
            <button
              class="twisty"
              aria-label={open ? t("side.collapse") : t("side.expand")}
              onclick={() => toggle(id)}
            >
              {open ? "▾" : "▸"}
            </button>
          {:else}
            <span class="twisty"></span>
          {/if}
          <button
            class="title"
            dir="auto"
            disabled={!item.target}
            title={item.target?.kind === "uri" ? item.target.uri : item.title}
            onclick={() => item.target && viewer.follow(item.target)}
          >
            {item.title || "(untitled)"}
          </button>
        </div>
        {#if open}
          {@render tree(item.children, `${id}.`, depth + 1)}
        {/if}
      </li>
    {/each}
  </ul>
{/snippet}

<aside class="sidebar" aria-label={t("side.label")}>
  <div class="tabs" role="tablist">
    <button role="tab" aria-selected={panel === "pages"} onclick={() => showPanel("pages")}>
      {t("side.pages")}
    </button>
    <button role="tab" aria-selected={panel === "comments"} onclick={() => showPanel("comments")}>
      {t("side.comments")}
    </button>
    <button role="tab" aria-selected={panel === "bookmarks"} onclick={() => showPanel("bookmarks")}>
      {t("side.bookmarks")}
    </button>
  </div>

  {#if panel === "pages"}
    <div
      class="thumbs"
      bind:this={list}
      bind:clientHeight={listHeight}
      onscroll={(e) => (scrollTop = e.currentTarget.scrollTop)}
      onwheel={() => (userScrolledAt = performance.now())}
      role="listbox"
      aria-label={t("side.pages")}
    >
      <div style:height="{total}px" style:position="relative">
        {#each visible as i (i)}
          <button
            class="thumb"
            class:current={i === page}
            role="option"
            aria-selected={i === page}
            style:top="{offsets[i]}px"
            onclick={() => viewer.navigate(i)}
            title={t("side.page", { n: i + 1 })}
          >
            <span class="img" style:width="{THUMB_W}px" style:height="{heights[i] - LABEL_H - PAD}px">
              {#if urls.get(i)}<img src={urls.get(i)} alt="" draggable="false" />{/if}
            </span>
            <span class="label">{i + 1}</span>
          </button>
        {/each}
      </div>
    </div>
  {:else if panel === "comments"}
    <div class="comments">
      {#if comments === null}
        <p class="note">{t("side.loading")}</p>
      {:else if comments.length === 0}
        <p class="note">{t("side.noComments")}</p>
      {:else}
        <ul>
          {#each comments as c (c.id)}
            <li>
              <button class="comment" class:current={c.id === selected} onclick={() => oncomment(c)}>
                <span class="c-head">
                  <span class="dot" style:background={css(c.color)}></span>
                  <span class="c-kind">{t(`kind.${c.kind}` as "kind.other")}</span>
                  <span class="c-page">{t("side.page", { n: c.page + 1 })}</span>
                </span>
                {#if c.contents}<span class="c-text" dir="auto">{c.contents}</span>{/if}
                {#if c.author || c.modified}
                  <span class="c-meta" dir="auto">
                    {c.author}{c.author && c.modified ? " · " : ""}{when(c.modified)}
                  </span>
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else}
    <div class="bookmarks">
      {#if outline === null}
        <p class="note">{t("side.loading")}</p>
      {:else if outline.length === 0}
        <p class="note">{t("side.noBookmarks")}</p>
      {:else}
        {@render tree(outline, "", 0)}
      {/if}
    </div>
  {/if}
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    width: 200px;
    min-width: 200px;
    background: var(--bar);
    border-inline-end: 1px solid var(--edge);
    min-height: 0;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 6px 6px 0;
    border-bottom: 1px solid var(--edge);
  }
  .tabs button {
    flex: 1;
    padding: 5px 2px;
    font-size: 12px;
    background: transparent;
    color: var(--muted);
    border: 0;
    border-bottom: 2px solid transparent;
    font: inherit;
    cursor: pointer;
  }
  .tabs button[aria-selected="true"] {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .thumbs,
  .comments,
  .bookmarks {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  .thumb {
    position: absolute;
    left: 0;
    right: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 6px 0 0;
    background: none;
    border: 0;
    color: var(--muted);
    font: inherit;
    cursor: pointer;
  }
  .thumb .img {
    display: block;
    background: #fff;
    box-shadow: 0 1px 3px rgb(0 0 0 / 0.6);
    outline: 2px solid transparent;
    outline-offset: 2px;
  }
  .thumb img {
    display: block;
    width: 100%;
    height: 100%;
  }
  .thumb:hover .img {
    outline-color: var(--border);
  }
  .thumb.current .img {
    outline-color: var(--accent);
  }
  .thumb.current .label {
    color: var(--text);
    font-weight: 600;
  }
  .thumb:focus-visible .img {
    outline-color: var(--accent);
  }
  .bookmarks ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .bookmarks > ul {
    padding: 6px 0;
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 2px;
    padding-inline-end: 6px;
  }
  .row.current {
    background: var(--selected);
  }
  .twisty {
    flex: none;
    width: 18px;
    padding: 3px 0;
    background: none;
    border: 0;
    color: var(--muted);
    font: inherit;
    cursor: pointer;
  }
  .title {
    flex: 1;
    min-width: 0;
    padding: 3px 2px;
    background: none;
    border: 0;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
    overflow-wrap: anywhere;
  }
  .title:hover:not(:disabled) {
    text-decoration: underline;
  }
  .title:disabled {
    color: var(--muted);
    cursor: default;
  }
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .comments ul {
    list-style: none;
    margin: 0;
    padding: 4px 0;
  }
  .comment {
    display: grid;
    gap: 2px;
    width: 100%;
    padding: 6px 10px;
    background: none;
    border: 0;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .comment:hover {
    background: var(--control-hover);
  }
  .comment.current {
    background: var(--selected);
  }
  .c-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1px solid var(--border);
    flex: none;
  }
  .c-kind {
    font-weight: 600;
    flex: 1;
  }
  .c-page,
  .c-meta {
    color: var(--muted);
    font-size: 12px;
  }
  .c-text {
    overflow-wrap: anywhere;
    white-space: pre-wrap;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .note {
    padding: 12px;
    color: var(--muted);
  }
</style>
