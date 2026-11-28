<script lang="ts">
  // A new document: page size, orientation, margins and how many pages.
  import Dialog from "./Dialog.svelte";
  import { t } from "./i18n.svelte";

  interface Props {
    /** Makes the document (sizes in points); resolves false if the reader cancelled. */
    onmake: (size: number[], pages: number, margins: number[]) => Promise<boolean>;
    onclose: () => void;
  }
  let { onmake, onclose }: Props = $props();

  const MM = 72 / 25.4;
  const SIZES: Record<string, [number, number]> = {
    a4: [210, 297],
    a5: [148, 210],
    a3: [297, 420],
    letter: [215.9, 279.4],
    legal: [215.9, 355.6],
  };
  let size = $state("a4");
  let landscape = $state(false);
  let customW = $state(210);
  let customH = $state(297);
  let margin = $state(20);
  let pages = $state(1);
  let busy = $state(false);
  let error = $state("");

  const mm = $derived.by((): [number, number] => {
    const [a, b] = size === "custom" ? [customW, customH] : SIZES[size];
    const [w, h] = [Math.min(a, b), Math.max(a, b)];
    return landscape ? [h, w] : [w, h];
  });
  const valid = $derived(
    mm.every((v) => Number.isFinite(v) && v >= 25 && v <= 5000) &&
      Number.isFinite(margin) &&
      margin >= 0 &&
      margin * 3 <= Math.min(...mm) &&
      Number.isInteger(pages) &&
      pages >= 1 &&
      pages <= 500,
  );

  async function go() {
    busy = true;
    error = "";
    try {
      const m = margin * MM;
      if (await onmake([mm[0] * MM, mm[1] * MM], pages, [m, m, m, m])) onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={t("new.title")} ok={t("new.ok")} canOk={valid} {busy} {error} onok={go} oncancel={onclose}>
  <label>
    {t("new.size")}
    <select bind:value={size}>
      <option value="a4">A4 (210 × 297 mm)</option>
      <option value="a5">A5 (148 × 210 mm)</option>
      <option value="a3">A3 (297 × 420 mm)</option>
      <option value="letter">Letter (8.5 × 11 in)</option>
      <option value="legal">Legal (8.5 × 14 in)</option>
      <option value="custom">{t("new.custom")}</option>
    </select>
  </label>
  {#if size === "custom"}
    <label>
      {t("new.width")} <input type="number" min="25" max="5000" step="1" bind:value={customW} /> mm
    </label>
    <label>
      {t("new.height")} <input type="number" min="25" max="5000" step="1" bind:value={customH} /> mm
    </label>
  {/if}
  <fieldset>
    <legend>{t("new.orientation")}</legend>
    <label><input type="radio" bind:group={landscape} value={false} /> {t("new.portrait")}</label>
    <label><input type="radio" bind:group={landscape} value={true} /> {t("new.landscape")}</label>
  </fieldset>
  <label>{t("new.margins")} <input type="number" min="0" max="200" step="1" bind:value={margin} /> mm</label>
  <label>{t("new.pages")} <input type="number" min="1" max="500" step="1" bind:value={pages} /></label>
  <p class="note">{t("new.note")}</p>
</Dialog>
