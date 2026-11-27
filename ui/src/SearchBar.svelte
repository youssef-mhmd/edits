<script lang="ts">
  // Find in document (Ctrl F). Results stream in from the engine, starting at
  // the current page; the first one at or after it becomes current.
  import { onDestroy, onMount } from "svelte";
  import { t } from "./i18n.svelte";
  import { cancelSearch, search, type SearchOptions } from "./api";
  import type { Hit, Viewer } from "./viewer";

  interface Props {
    viewer: Viewer;
    options: SearchOptions;
    onoptions: (o: SearchOptions) => void;
    onclose: () => void;
  }
  let { viewer, options, onoptions, onclose }: Props = $props();

  let input: HTMLInputElement;
  let query = $state("");
  let hits: Hit[] = $state.raw([]);
  let current = $state(-1);
  let searching = $state(false);
  let searched = $state(0);
  let error = $state("");
  let searchId = 0;
  let generation = 0;
  let timer = 0;

  export function focus() {
    input?.focus();
    input?.select();
  }

  function stop() {
    if (searchId) void cancelSearch(searchId);
    searchId = 0;
  }

  async function run() {
    stop();
    const gen = ++generation;
    hits = [];
    current = -1;
    searched = 0;
    error = "";
    viewer.setHits(new Map());
    const q = query.trim();
    if (!q) {
      searching = false;
      return;
    }
    searching = true;
    const start = viewer.currentPage();
    // Collected per search; nothing renders from it directly.
    // eslint-disable-next-line svelte/prefer-svelte-reactivity
    const byPage = new Map<number, [number, number][]>();
    searchId = await search(viewer.info.doc, q, options, start, viewer.pageCount, (e) => {
      if (gen !== generation) return;
      if (e.kind === "batch") {
        searched = e.searched;
        if (!e.results.length) return;
        for (const [page, ranges] of e.results) {
          byPage.set(page, ranges);
          viewer.addHits(page, ranges);
        }
        const all: Hit[] = [];
        for (const [page, ranges] of [...byPage].sort((a, b) => a[0] - b[0])) {
          for (const [s, z] of ranges) all.push({ page, start: s, end: z });
        }
        const cur = current >= 0 ? hits[current] : null;
        hits = all;
        if (cur) {
          current = all.findIndex((h) => h.page === cur.page && h.start === cur.start);
        } else {
          // First result: the first hit at or after the page the search started on.
          const k = all.findIndex((h) => h.page >= start);
          current = k >= 0 ? k : 0;
          void viewer.showHit(all[current]);
        }
      } else if (e.kind === "done") {
        searching = false;
        searched = e.searched;
      } else {
        searching = false;
        error = e.message;
      }
    });
  }

  function schedule() {
    clearTimeout(timer);
    timer = window.setTimeout(run, 200);
  }

  export function step(dir: number) {
    if (!hits.length) return;
    current = (current + dir + hits.length) % hits.length;
    void viewer.showHit(hits[current]);
  }

  function toggle(key: keyof SearchOptions) {
    onoptions({ ...options, [key]: !options[key] });
  }

  // Options changed: search again.
  $effect(() => {
    void options;
    if (query.trim()) schedule();
  });

  onMount(() => focus());
  onDestroy(() => {
    clearTimeout(timer);
    stop();
    viewer.setHits(new Map());
  });

  const status = $derived(
    error
      ? error
      : !query.trim()
        ? ""
        : hits.length
          ? t("find.of", { n: current + 1, total: `${hits.length}${searching ? "+" : ""}` })
          : searching
            ? t("find.searching", { done: searched, total: viewer.pageCount })
            : t("find.none"),
  );
</script>

<div class="searchbar" role="search">
  <input
    bind:this={input}
    bind:value={query}
    dir="auto"
    placeholder={t("find.placeholder")}
    aria-label={t("find.placeholder")}
    oninput={schedule}
    onkeydown={(e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        if (searching && !hits.length) return;
        step(e.shiftKey ? -1 : 1);
      } else if (e.key === "Escape") {
        e.preventDefault();
        onclose();
      }
    }}
  />
  <span class="status" aria-live="polite">{status}</span>
  <button
    onclick={() => step(-1)}
    disabled={!hits.length}
    title={t("find.prevTitle")}
    aria-label={t("find.prev")}
  >
    ↑
  </button>
  <button
    onclick={() => step(1)}
    disabled={!hits.length}
    title={t("find.nextTitle")}
    aria-label={t("find.next")}
  >
    ↓
  </button>
  <span class="sep"></span>
  <button
    class:on={options.match_case}
    aria-pressed={options.match_case}
    onclick={() => toggle("match_case")}
    title={t("find.case")}
  >
    Aa
  </button>
  <button
    class:on={options.whole_word}
    aria-pressed={options.whole_word}
    onclick={() => toggle("whole_word")}
    title={t("find.wordTitle")}
  >
    {t("find.word")}
  </button>
  <button
    class:on={options.ignore_diacritics}
    aria-pressed={options.ignore_diacritics}
    onclick={() => toggle("ignore_diacritics")}
    title={t("find.diacritics")}
  >
    حَ = ح
  </button>
  <button
    class:on={options.letter_variants}
    aria-pressed={options.letter_variants}
    onclick={() => toggle("letter_variants")}
    title={t("find.variants")}
  >
    أ = ا
  </button>
  <button onclick={onclose} title={t("find.closeTitle")} aria-label={t("find.close")}>×</button>
</div>

<style>
  .searchbar {
    position: absolute;
    top: 8px;
    inset-inline-end: 20px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px;
    background: var(--bar);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 4px 16px var(--popup-shadow);
  }
  input {
    width: 220px;
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 4px 8px;
    font: inherit;
  }
  input:focus-visible,
  button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .status {
    min-width: 9ch;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  button {
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 3px 7px;
    font: inherit;
    cursor: pointer;
    white-space: nowrap;
  }
  button:disabled {
    opacity: 0.4;
    cursor: default;
  }
  button.on {
    border-color: var(--accent);
    background: var(--selected);
  }
  .sep {
    width: 1px;
    height: 18px;
    background: var(--border);
    margin: 0 2px;
  }
</style>
