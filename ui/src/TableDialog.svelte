<script lang="ts">
  // A table's cells, rows and columns. Rows grow to fit their text on the page.
  import type { TableData } from "./api";
  import Dialog from "./Dialog.svelte";
  import { t } from "./i18n.svelte";

  interface Props {
    table: TableData;
    ondone: (t: TableData) => void;
    onclose: () => void;
  }
  let { table, ondone, onclose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let cells: string[][] = $state(table.cells.map((r) => [...r]));
  // svelte-ignore state_referenced_locally
  let widths: number[] = $state([...table.widths]);
  // svelte-ignore state_referenced_locally
  let header = $state(table.header !== null);
  // svelte-ignore state_referenced_locally
  let rtl = $state(table.rtl);
  // svelte-ignore state_referenced_locally
  let size = $state(table.style.size);

  const cols = $derived(widths.length);
  // Shown in reading order: right to left tables have their first column at the right.
  const order = $derived(rtl ? [...Array(cols).keys()].reverse() : [...Array(cols).keys()]);

  function addRow() {
    cells = [...cells, Array(cols).fill("")];
  }
  function removeRow() {
    if (cells.length > 1) cells = cells.slice(0, -1);
  }
  function addCol() {
    widths = [...widths, widths[widths.length - 1] ?? 80];
    cells = cells.map((r) => [...r, ""]);
  }
  function removeCol() {
    if (cols <= 1) return;
    widths = widths.slice(0, -1);
    cells = cells.map((r) => r.slice(0, -1));
  }

  function done() {
    ondone({
      ...table,
      cells,
      widths,
      heights: cells.map((_, k) => table.heights[k] ?? 20),
      header: header ? (table.header ?? [0.9, 0.9, 0.9]) : null,
      rtl,
      style: { ...table.style, size },
    });
    onclose();
  }
</script>

<Dialog title={t("table.title")} ok={t("table.ok")} onok={done} oncancel={onclose} wide>
  <div
    class="grid"
    dir={rtl ? "rtl" : "ltr"}
    style:grid-template-columns={`repeat(${cols}, minmax(70px, 1fr))`}
  >
    {#each order as c (c)}
      <label class="w">
        <input
          type="number"
          min="10"
          max="1000"
          step="5"
          value={widths[c]}
          aria-label={t("table.width", { n: c + 1 })}
          onchange={(e) => {
            const v = Number(e.currentTarget.value);
            if (v >= 10 && v <= 1000) widths[c] = v;
          }}
        />
        pt
      </label>
    {/each}
    {#each cells as row, r (r)}
      {#each order as c (c)}
        <textarea
          rows="1"
          dir="auto"
          class:head={header && r === 0}
          aria-label={t("table.cell", { row: r + 1, col: c + 1 })}
          bind:value={row[c]}></textarea>
      {/each}
    {/each}
  </div>
  <div class="row">
    <button type="button" onclick={addRow}>{t("table.addRow")}</button>
    <button type="button" disabled={cells.length <= 1} onclick={removeRow}>{t("table.removeRow")}</button>
    <button type="button" onclick={addCol}>{t("table.addCol")}</button>
    <button type="button" disabled={cols <= 1} onclick={removeCol}>{t("table.removeCol")}</button>
  </div>
  <label><input type="checkbox" bind:checked={header} /> {t("table.header")}</label>
  <label><input type="checkbox" bind:checked={rtl} /> {t("table.rtl")}</label>
  <label>{t("item.size")} <input type="number" min="4" max="72" step="1" bind:value={size} /></label>
</Dialog>

<style>
  .grid {
    display: grid;
    gap: 2px;
    max-height: 340px;
    overflow: auto;
  }
  .w {
    display: flex;
    align-items: center;
    gap: 2px;
    color: var(--muted);
    font-size: 11px;
  }
  .w input {
    width: 100%;
  }
  textarea {
    resize: vertical;
    min-height: 26px;
    font: inherit;
  }
  textarea.head {
    font-weight: 700;
  }
  .row {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
</style>
