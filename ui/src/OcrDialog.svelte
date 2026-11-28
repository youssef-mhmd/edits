<script lang="ts">
  // Recognise text: which pages, and how digits and diacritics come out.
  import type { OcrOptions } from "./api";
  import Dialog from "./Dialog.svelte";
  import { t } from "./i18n.svelte";

  interface Props {
    /** The page shown now (0-based). */
    current: number;
    count: number;
    options: OcrOptions;
    skipText: boolean;
    onstart: (pages: number[], options: OcrOptions, skipText: boolean) => Promise<void>;
    onclose: () => void;
  }
  let { current, count, options, skipText, onstart, onclose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let which = $state<"all" | "current" | "range">(count > 1 ? "all" : "current");
  // svelte-ignore state_referenced_locally
  let from = $state(current + 1);
  // svelte-ignore state_referenced_locally
  let to = $state(count);
  // svelte-ignore state_referenced_locally
  let diacritics = $state(options.diacritics);
  // svelte-ignore state_referenced_locally
  let digits = $state(options.digits);
  // svelte-ignore state_referenced_locally
  let skip = $state(skipText);
  let busy = $state(false);
  let error = $state("");

  const pages = $derived.by(() => {
    if (which === "all") return [...Array(count).keys()];
    if (which === "current") return [current];
    const a = Math.max(1, Math.min(from, to));
    const b = Math.min(count, Math.max(from, to));
    if (!Number.isFinite(a) || !Number.isFinite(b) || a > b) return [];
    return Array.from({ length: b - a + 1 }, (_, k) => a - 1 + k);
  });

  async function go() {
    busy = true;
    error = "";
    try {
      await onstart(pages, { diacritics, digits }, skip);
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog
  title={t("ocr.title")}
  ok={t("ocr.ok")}
  canOk={pages.length > 0}
  {busy}
  {error}
  onok={go}
  oncancel={onclose}
>
  <fieldset>
    <legend>{t("ocr.pages")}</legend>
    {#if count > 1}
      <label><input type="radio" bind:group={which} value="all" /> {t("ocr.all", { n: count })}</label>
    {/if}
    <label>
      <input type="radio" bind:group={which} value="current" />
      {t("ocr.current", { n: current + 1 })}
    </label>
    {#if count > 1}
      <label class="range">
        <input type="radio" bind:group={which} value="range" />
        {t("ocr.range")}
        <input
          type="number"
          min="1"
          max={count}
          bind:value={from}
          onfocus={() => (which = "range")}
          aria-label={t("ocr.range")}
        />
        {t("ocr.to")}
        <input
          type="number"
          min="1"
          max={count}
          bind:value={to}
          onfocus={() => (which = "range")}
          aria-label={t("ocr.to")}
        />
      </label>
    {/if}
    <label><input type="checkbox" bind:checked={skip} /> {t("ocr.skipText")}</label>
  </fieldset>
  <fieldset>
    <label><input type="checkbox" bind:checked={diacritics} /> {t("ocr.diacritics")}</label>
    <label>
      {t("ocr.digits")}
      <select bind:value={digits}>
        <option value="keep">{t("ocr.digitsKeep")}</option>
        <option value="western">{t("ocr.digitsWestern")}</option>
        <option value="arabic">{t("ocr.digitsArabic")}</option>
      </select>
    </label>
  </fieldset>
  <p class="note">{t("ocr.note")}</p>
</Dialog>

<style>
  .range input[type="number"] {
    width: 5em;
  }
</style>
