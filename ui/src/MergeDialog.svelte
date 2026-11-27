<script lang="ts">
  // Combine PDF files into a new one, in the order listed.
  import { open as pickFile } from "@tauri-apps/plugin-dialog";
  import Dialog from "./Dialog.svelte";
  import { t } from "./i18n.svelte";

  interface Props {
    /** Files to start with. */
    initial: string[];
    /** Combines the files; resolves false if the reader cancelled. */
    onmerge: (paths: string[]) => Promise<boolean>;
    onclose: () => void;
  }
  let { initial, onmerge, onclose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let files: string[] = $state([...initial]);
  let picked = $state(-1);
  let busy = $state(false);
  let error = $state("");
  const nameOf = (p: string) => p.split(/[\\/]/).pop() ?? p;

  async function add() {
    const r = await pickFile({ multiple: true, filters: [{ name: "PDF", extensions: ["pdf"] }] });
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
      if (await onmerge(files)) onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog
  title={t("merge.title")}
  ok={t("merge.ok")}
  canOk={files.length >= 2}
  {busy}
  {error}
  onok={go}
  oncancel={onclose}
  wide
>
  <p class="note">{t("merge.note")}</p>
  <ol class="files" aria-label={t("merge.files")}>
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
      <li class="empty">{t("merge.empty")}</li>
    {/each}
  </ol>
  <div class="row">
    <button type="button" onclick={add}>{t("merge.add")}</button>
    <button type="button" disabled={picked <= 0} onclick={() => shift(-1)}>{t("merge.up")}</button>
    <button type="button" disabled={picked < 0 || picked >= files.length - 1} onclick={() => shift(1)}>
      {t("merge.down")}
    </button>
    <button type="button" disabled={picked < 0} onclick={drop}>{t("merge.remove")}</button>
  </div>
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
