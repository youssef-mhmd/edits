<script lang="ts">
  // Make a new PDF from pictures, one per page, in the order listed.
  import { open as pickFile } from "@tauri-apps/plugin-dialog";
  import Dialog from "./Dialog.svelte";
  import { t } from "./i18n.svelte";

  interface Props {
    /** Text recognition can be offered. */
    canOcr: boolean;
    /** Makes the PDF; `size` in points or null; resolves false if the reader cancelled. */
    onmake: (paths: string[], size: number[] | null, ocr: boolean) => Promise<boolean>;
    onclose: () => void;
  }
  let { canOcr, onmake, onclose }: Props = $props();

  const SIZES: Record<string, number[] | null> = {
    fit: null,
    a4: [595.28, 841.89],
    letter: [612, 792],
    a5: [419.53, 595.28],
  };
  let files: string[] = $state([]);
  let picked = $state(-1);
  let size = $state("a4");
  // svelte-ignore state_referenced_locally
  let ocr = $state(canOcr);
  let busy = $state(false);
  let error = $state("");
  const nameOf = (p: string) => p.split(/[\\/]/).pop() ?? p;

  async function add() {
    const r = await pickFile({
      multiple: true,
      filters: [{ name: "JPEG, PNG", extensions: ["jpg", "jpeg", "png"] }],
    });
    const paths = Array.isArray(r) ? r : typeof r === "string" ? [r] : [];
    files = [...files, ...paths];
  }

  function shift(by: number) {
    const i = picked;
    const j = i + by;
    if (i < 0 || j < 0 || j >= files.length) return;
    const next = [...files];
    [next[i], next[j]] = [next[j], next[i]];
    files = next;
    picked = j;
  }

  function drop() {
    if (picked < 0) return;
    files = files.filter((_, k) => k !== picked);
    picked = Math.min(picked, files.length - 1);
  }

  async function go() {
    busy = true;
    error = "";
    try {
      if (await onmake(files, SIZES[size], canOcr && ocr)) onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog
  title={t("pics.title")}
  ok={t("pics.ok")}
  canOk={files.length >= 1}
  {busy}
  {error}
  onok={go}
  oncancel={onclose}
  wide
>
  <p class="note">{t("pics.note")}</p>
  <ol class="files" aria-label={t("pics.files")}>
    {#each files as f, k (k)}
      <li>
        <button
          type="button"
          class:picked={picked === k}
          aria-pressed={picked === k}
          onclick={() => (picked = k)}
        >
          <span class="n">{k + 1}.</span>
          <span class="name" dir="auto" title={f}>{nameOf(f)}</span>
        </button>
      </li>
    {:else}
      <li class="empty">{t("pics.empty")}</li>
    {/each}
  </ol>
  <div class="row">
    <button type="button" onclick={add}>{t("pics.add")}</button>
    <button type="button" disabled={picked <= 0} onclick={() => shift(-1)}>{t("merge.up")}</button>
    <button type="button" disabled={picked < 0 || picked >= files.length - 1} onclick={() => shift(1)}>
      {t("merge.down")}
    </button>
    <button type="button" disabled={picked < 0} onclick={drop}>{t("merge.remove")}</button>
  </div>
  <label>
    {t("pics.size")}
    <select bind:value={size}>
      <option value="a4">A4</option>
      <option value="letter">Letter</option>
      <option value="a5">A5</option>
      <option value="fit">{t("pics.sizeFit")}</option>
    </select>
  </label>
  {#if canOcr}
    <label><input type="checkbox" bind:checked={ocr} /> {t("pics.ocr")}</label>
  {/if}
</Dialog>

<style>
  .files {
    list-style: none;
    margin: 0;
    padding: 4px;
    min-height: 120px;
    max-height: 300px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--control);
  }
  .files button {
    display: flex;
    gap: 8px;
    width: 100%;
    border: none;
    background: none;
    text-align: start;
    padding: 4px 6px;
  }
  .files button.picked {
    background: var(--selected);
  }
  .n {
    color: var(--muted);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    color: var(--muted);
    padding: 6px;
  }
  .row {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
</style>
