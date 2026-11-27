<script lang="ts">
  // Crop pages: margins to cut off, in millimetres, on the chosen pages.
  import Dialog from "./Dialog.svelte";
  import { t } from "./i18n.svelte";

  interface Props {
    /** The pages chosen in the grid. */
    chosen: number[];
    count: number;
    /** Crops `pages` by margins in points (left, top, right, bottom), or removes their crop (null). */
    oncrop: (pages: number[], margins: number[] | null) => Promise<void>;
    onclose: () => void;
  }
  let { chosen, count, oncrop, onclose }: Props = $props();

  let which = $state<"chosen" | "all">("chosen");
  let top = $state(10);
  let bottom = $state(10);
  let left = $state(10);
  let right = $state(10);
  let busy = $state(false);
  let error = $state("");

  const pages = $derived(which === "all" ? [...Array(count).keys()] : chosen);
  const valid = $derived([top, bottom, left, right].every((v) => Number.isFinite(v) && v >= 0 && v <= 500));
  const pt = (mm: number) => (mm * 72) / 25.4;

  async function go(margins: number[] | null) {
    busy = true;
    error = "";
    try {
      await oncrop(pages, margins);
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog
  title={t("crop.title")}
  ok={t("crop.ok")}
  canOk={valid && pages.length > 0}
  {busy}
  {error}
  onok={() => go([pt(left), pt(top), pt(right), pt(bottom)])}
  oncancel={onclose}
>
  <fieldset>
    <legend>{t("crop.pages")}</legend>
    <label>
      <input type="radio" bind:group={which} value="chosen" />
      {t("crop.chosen", { n: chosen.length })}
    </label>
    <label><input type="radio" bind:group={which} value="all" /> {t("crop.all", { n: count })}</label>
  </fieldset>
  <fieldset class="margins">
    <legend>{t("crop.margins")}</legend>
    <label>{t("crop.top")} <input type="number" min="0" max="500" step="1" bind:value={top} /> mm</label>
    <label>
      {t("crop.bottom")} <input type="number" min="0" max="500" step="1" bind:value={bottom} /> mm
    </label>
    <label>{t("crop.left")} <input type="number" min="0" max="500" step="1" bind:value={left} /> mm</label>
    <label>{t("crop.right")} <input type="number" min="0" max="500" step="1" bind:value={right} /> mm</label>
  </fieldset>
  <p class="note">{t("crop.note")}</p>
  {#snippet extra()}
    <button type="button" disabled={busy || !pages.length} onclick={() => go(null)}>
      {t("crop.remove")}
    </button>
  {/snippet}
</Dialog>

<style>
  .margins {
    grid-template-columns: 1fr 1fr;
  }
  .margins legend {
    grid-column: 1 / -1;
  }
</style>
