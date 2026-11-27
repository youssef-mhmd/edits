<script lang="ts">
  // Page numbers, headers, footers and a watermark for the chosen pages.
  import Dialog from "./Dialog.svelte";
  import type { Decor } from "./api";
  import { t } from "./i18n.svelte";

  interface Props {
    chosen: number[];
    count: number;
    ondecorate: (pages: number[], d: Decor) => Promise<void>;
    /** Takes away what was added to `pages` since the last save. */
    onremove: (pages: number[]) => Promise<void>;
    onclose: () => void;
  }
  let { chosen, count, ondecorate, onremove, onclose }: Props = $props();

  const COLORS: [string, number[]][] = [
    ["#000000", [0, 0, 0]],
    ["#555555", [0.33, 0.33, 0.33]],
    ["#1a3380", [0.1, 0.2, 0.5]],
    ["#cc1a1a", [0.8, 0.1, 0.1]],
  ];

  let which = $state<"chosen" | "all">("all");
  let header = $state(["", "", ""]);
  let footer = $state(["", "{n}", ""]);
  let size = $state(10);
  let color = $state("#000000");
  let marginMm = $state(10);
  let first = $state(1);
  let arabicDigits = $state(false);
  let watermark = $state("");
  let wmSize = $state(60);
  let wmOpacity = $state(25);
  let busy = $state(false);
  let error = $state("");

  const pages = $derived(which === "all" ? [...Array(count).keys()] : chosen);
  const empty = $derived([...header, ...footer, watermark].every((s) => !s.trim()));
  const rgb = (css: string) => COLORS.find((c) => c[0] === css)?.[1] ?? [0, 0, 0];

  function preset(text: string) {
    footer = ["", text, ""];
  }

  async function run(fn: () => Promise<void>) {
    busy = true;
    error = "";
    try {
      await fn();
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function go() {
    // {n} counts from the first page chosen.
    const firstPage = pages.length ? Math.min(...pages) : 0;
    const d: Decor = {
      header: [...header] as [string, string, string],
      footer: [...footer] as [string, string, string],
      size,
      color: rgb(color),
      margin: (marginMm * 72) / 25.4,
      offset: Math.round(first) - 1 - firstPage,
      arabic_digits: arabicDigits,
      watermark,
      watermark_size: wmSize,
      watermark_color: [0.8, 0.1, 0.1],
      watermark_opacity: wmOpacity / 100,
    };
    void run(() => ondecorate(pages, d));
  }
</script>

<Dialog
  title={t("hf.title")}
  ok={t("hf.ok")}
  canOk={!empty && pages.length > 0}
  {busy}
  {error}
  onok={go}
  oncancel={onclose}
  wide
>
  <fieldset>
    <legend>{t("crop.pages")}</legend>
    <label><input type="radio" bind:group={which} value="all" /> {t("crop.all", { n: count })}</label>
    <label>
      <input type="radio" bind:group={which} value="chosen" />
      {t("crop.chosen", { n: chosen.length })}
    </label>
  </fieldset>
  <fieldset>
    <legend>{t("hf.numbers")}</legend>
    <div class="row">
      <button type="button" onclick={() => preset("{n}")}>1</button>
      <button type="button" onclick={() => preset("Page {n} of {total}")}>Page 1 of 9</button>
      <button type="button" onclick={() => preset("صفحة {n} من {total}")}>صفحة 1 من 9</button>
      <button type="button" onclick={() => preset("{n} / {total}")}>1 / 9</button>
    </div>
    <p class="note">{t("hf.codes")}</p>
  </fieldset>
  <div class="slots">
    <span></span>
    <span class="head">{t("hf.left")}</span>
    <span class="head">{t("hf.centre")}</span>
    <span class="head">{t("hf.right")}</span>
    <span class="head">{t("hf.header")}</span>
    {#each [0, 1, 2] as k (k)}
      <input type="text" dir="auto" bind:value={header[k]} aria-label={`${t("hf.header")} ${k + 1}`} />
    {/each}
    <span class="head">{t("hf.footer")}</span>
    {#each [0, 1, 2] as k (k)}
      <input type="text" dir="auto" bind:value={footer[k]} aria-label={`${t("hf.footer")} ${k + 1}`} />
    {/each}
  </div>
  <div class="row">
    <label>{t("hf.size")} <input type="number" min="4" max="72" bind:value={size} /> pt</label>
    <label>
      {t("hf.color")}
      <select bind:value={color}>
        {#each COLORS as [css] (css)}<option value={css} style:color={css}>■ {css}</option>{/each}
      </select>
    </label>
    <label>{t("hf.margin")} <input type="number" min="0" max="100" bind:value={marginMm} /> mm</label>
  </div>
  <div class="row">
    <label>{t("hf.first")} <input type="number" min="-9999" max="99999" bind:value={first} /></label>
    <label><input type="checkbox" bind:checked={arabicDigits} /> {t("hf.arabicDigits")}</label>
  </div>
  <fieldset>
    <legend>{t("hf.watermark")}</legend>
    <input type="text" dir="auto" bind:value={watermark} placeholder={t("hf.watermarkHint")} />
    <div class="row">
      <label>{t("hf.size")} <input type="number" min="8" max="400" bind:value={wmSize} /> pt</label>
      <label>{t("hf.opacity")} <input type="number" min="2" max="100" bind:value={wmOpacity} /> %</label>
    </div>
  </fieldset>
  <p class="note">{t("hf.note")}</p>
  {#snippet extra()}
    <button type="button" disabled={busy || !pages.length} onclick={() => void run(() => onremove(pages))}>
      {t("hf.remove")}
    </button>
  {/snippet}
</Dialog>

<style>
  .row {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    align-items: center;
  }
  .slots {
    display: grid;
    grid-template-columns: auto 1fr 1fr 1fr;
    gap: 6px;
    align-items: center;
  }
  .head {
    color: var(--muted);
    font-size: 12px;
  }
</style>
