<script lang="ts">
  // Split a document into several files: every N pages, by page ranges, or
  // at each top-level bookmark. The folder is chosen next.
  import Dialog from "./Dialog.svelte";
  import type { OutlineItem } from "./api";
  import { t } from "./i18n.svelte";
  import { parseRange } from "./print";

  export interface Part {
    pages: number[];
    name: string;
  }

  interface Props {
    count: number;
    outline: OutlineItem[] | null;
    /** The document's name without ".pdf". */
    base: string;
    /** Writes the parts; resolves with the files written, or null if cancelled. */
    onsplit: (parts: Part[]) => Promise<string[] | null>;
    onclose: () => void;
  }
  let { count, outline, base, onsplit, onclose }: Props = $props();

  let mode = $state<"every" | "ranges" | "bookmarks">("every");
  let every = $state(1);
  let ranges = $state("");
  let busy = $state(false);
  let error = $state("");

  const span = (a: number, b: number) => (a === b ? `${a + 1}` : `${a + 1}-${b + 1}`);
  const pagesFrom = (a: number, b: number) => Array.from({ length: b - a + 1 }, (_, k) => a + k);

  /** Top-level bookmarks with a page, in page order, first one per page. */
  const marks = $derived.by(() => {
    const out: { page: number; title: string }[] = [];
    for (const it of outline ?? []) {
      if (it.target?.kind !== "page") continue;
      const page = it.target.page;
      if (page >= 0 && page < count && !out.some((m) => m.page === page)) out.push({ page, title: it.title });
    }
    return out.sort((a, b) => a.page - b.page);
  });

  const parts = $derived.by((): Part[] | null => {
    if (mode === "every") {
      const n = Math.floor(every);
      if (!(n >= 1 && n <= count)) return null;
      const out: Part[] = [];
      for (let a = 0; a < count; a += n) {
        const b = Math.min(count - 1, a + n - 1);
        out.push({ pages: pagesFrom(a, b), name: `${base} - ${span(a, b)}` });
      }
      return out;
    }
    if (mode === "ranges") {
      const groups = ranges
        .split(/[;\n]/)
        .flatMap((g) => g.split(/[,،]/))
        .map((g) => g.trim())
        .filter(Boolean);
      if (!groups.length) return null;
      const out: Part[] = [];
      for (const g of groups) {
        const pages = parseRange(g, count);
        if (!pages || !pages.length) return null;
        out.push({ pages, name: `${base} - ${g.replace(/\s+/g, "")}` });
      }
      return out;
    }
    if (!marks.length) return null;
    return marks.map((m, k) => {
      // Pages before the first bookmark go with it.
      const a = k === 0 ? 0 : m.page;
      const b = k + 1 < marks.length ? marks[k + 1].page - 1 : count - 1;
      return { pages: pagesFrom(a, Math.max(a, b)), name: `${base} - ${m.title}` };
    });
  });

  async function go() {
    if (!parts) return;
    busy = true;
    error = "";
    try {
      const written = await onsplit(parts);
      if (written) onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog
  title={t("split.title")}
  ok={t("split.ok")}
  canOk={!!parts && parts.length > 0}
  {busy}
  {error}
  onok={go}
  oncancel={onclose}
>
  <fieldset>
    <legend>{t("split.how")}</legend>
    <label>
      <input type="radio" bind:group={mode} value="every" />
      {t("split.every")}
      <input type="number" min="1" max={count} bind:value={every} onfocus={() => (mode = "every")} />
      {t("split.everyPages")}
    </label>
    <label class="ranges">
      <input type="radio" bind:group={mode} value="ranges" />
      {t("split.ranges")}
      <input
        type="text"
        dir="ltr"
        bind:value={ranges}
        placeholder="1-3, 4-10, 11"
        onfocus={() => (mode = "ranges")}
        aria-invalid={mode === "ranges" && ranges.trim() !== "" && !parts}
      />
    </label>
    <label>
      <input type="radio" bind:group={mode} value="bookmarks" disabled={!marks.length} />
      {marks.length ? t("split.bookmarks", { n: marks.length }) : t("split.noBookmarks")}
    </label>
  </fieldset>
  <p class="note" aria-live="polite">
    {parts ? t("split.makes", { n: parts.length }) : t("split.bad", { n: count })}
  </p>
</Dialog>

<style>
  .ranges input[type="text"] {
    flex: 1;
  }
</style>
