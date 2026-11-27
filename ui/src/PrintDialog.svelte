<script lang="ts">
  // Print: which pages and at what resolution. The system print dialog
  // that follows picks the printer, copies and paper.
  import { onMount, untrack } from "svelte";
  import type { Transport } from "./api";
  import { t } from "./i18n.svelte";
  import { parseRange, printPages } from "./print";
  import type { Viewer } from "./viewer";

  interface Props {
    viewer: Viewer;
    transport: Transport;
    dpi: number;
    ondpi: (dpi: number) => void;
    onclose: () => void;
  }
  let { viewer, transport, dpi, ondpi, onclose }: Props = $props();

  // Fixed while the dialog is open.
  const count = untrack(() => viewer.pageCount);
  const current = untrack(() => viewer.currentPage());
  let which: "all" | "current" | "range" = $state("all");
  let range = $state("");
  let quality = $state(untrack(() => (dpi >= 300 ? 300 : 150)));
  let done = $state(-1); // pages prepared; -1 before starting
  let total = $state(0);
  let error = $state("");
  let stop = false;
  let firstField: HTMLInputElement | null = $state(null);

  const pages = $derived(
    which === "all"
      ? Array.from({ length: count }, (_, i) => i)
      : which === "current"
        ? [current]
        : parseRange(range, count),
  );

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!pages || !pages.length || done >= 0) return;
    ondpi(quality);
    error = "";
    total = pages.length;
    done = 0;
    stop = false;
    try {
      const printed = await printPages(
        viewer,
        transport,
        pages,
        quality,
        (n) => (done = n),
        () => stop,
      );
      if (printed || stop) onclose();
    } catch (err) {
      error = t("print.failed", { error: String(err) });
      done = -1;
    }
  }

  function cancel() {
    if (done >= 0) stop = true;
    else onclose();
  }

  onMount(() => firstField?.focus());
</script>

<div
  class="overlay"
  role="presentation"
  onkeydown={(e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      cancel();
    }
  }}
>
  <form class="dialog" onsubmit={submit} aria-labelledby="print-title">
    <h1 id="print-title">{t("print.title")}</h1>
    <fieldset disabled={done >= 0}>
      <legend>{t("print.pages")}</legend>
      <label>
        <input type="radio" bind:group={which} value="all" bind:this={firstField} />
        {t("print.all", { n: count })}
      </label>
      <label>
        <input type="radio" bind:group={which} value="current" />
        {t("print.current", { n: current + 1 })}
      </label>
      <label class="range">
        <input type="radio" bind:group={which} value="range" />
        {t("print.range")}
        <input
          type="text"
          dir="ltr"
          bind:value={range}
          placeholder={t("print.rangeHint")}
          onfocus={() => (which = "range")}
          aria-invalid={which === "range" && range.trim() !== "" && !pages}
        />
      </label>
      {#if which === "range" && range.trim() !== "" && !pages}
        <p class="error" role="alert">{t("print.badRange", { n: count })}</p>
      {/if}
    </fieldset>
    <fieldset disabled={done >= 0}>
      <legend>{t("print.quality")}</legend>
      <label><input type="radio" bind:group={quality} value={150} /> {t("print.standard")}</label>
      <label><input type="radio" bind:group={quality} value={300} /> {t("print.high")}</label>
    </fieldset>
    {#if done >= 0}
      <p class="progress" aria-live="polite">
        {t("print.preparing", { done: Math.min(done + 1, total), total })}
        <progress max={total} value={done}></progress>
      </p>
    {:else}
      <p class="note">{t("print.note")}</p>
    {/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="submit" class="primary" disabled={!pages || !pages.length || done >= 0}>
        {t("print.next")}
      </button>
      <button type="button" onclick={cancel}>{t("common.cancel")}</button>
    </div>
  </form>
</div>

<style>
  .overlay {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    background: var(--overlay);
    z-index: 20;
  }
  .dialog {
    display: grid;
    gap: 12px;
    width: min(420px, 90vw);
    padding: 18px 22px;
    background: var(--bar);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 32px var(--popup-shadow);
  }
  h1 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  fieldset {
    display: grid;
    gap: 6px;
    margin: 0;
    padding: 0;
    border: 0;
  }
  legend {
    padding: 0;
    margin-bottom: 6px;
    color: var(--muted);
  }
  label {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .range input[type="text"] {
    flex: 1;
  }
  input[type="text"] {
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 3px 8px;
    font: inherit;
  }
  input[aria-invalid="true"] {
    border-color: var(--error);
  }
  .note,
  .progress {
    margin: 0;
    color: var(--muted);
  }
  .progress {
    display: grid;
    gap: 6px;
  }
  progress {
    width: 100%;
  }
  .error {
    margin: 0;
    color: var(--error);
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
  button {
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 4px 12px;
    font: inherit;
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    background: var(--control-hover);
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .primary:hover:not(:disabled) {
    background: var(--primary-hover);
  }
  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
