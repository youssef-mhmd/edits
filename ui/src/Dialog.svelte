<script lang="ts">
  // A modal form: title, fields, and OK / Cancel. Escape cancels.
  import { onMount, type Snippet } from "svelte";
  import { t } from "./i18n.svelte";

  interface Props {
    title: string;
    /** Label of the main button. */
    ok: string;
    canOk?: boolean;
    busy?: boolean;
    error?: string;
    onok: () => void;
    oncancel: () => void;
    children: Snippet;
    /** More buttons, before Cancel. */
    extra?: Snippet;
    wide?: boolean;
  }
  let {
    title,
    ok,
    canOk = true,
    busy = false,
    error = "",
    onok,
    oncancel,
    children,
    extra,
    wide = false,
  }: Props = $props();
  let form: HTMLFormElement | null = $state(null);
  const id = `dlg-${Math.random().toString(36).slice(2)}`;

  onMount(() => form?.querySelector<HTMLElement>("input, select, button")?.focus());
</script>

<div
  class="overlay"
  role="presentation"
  onkeydown={(e) => {
    e.stopPropagation();
    if (e.key === "Escape") oncancel();
  }}
>
  <form
    class="dlg"
    class:wide
    bind:this={form}
    aria-labelledby={id}
    onsubmit={(e) => {
      e.preventDefault();
      if (canOk && !busy) onok();
    }}
  >
    <h1 {id}>{title}</h1>
    <fieldset class="body" disabled={busy}>
      {@render children()}
    </fieldset>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <div class="actions">
      <button type="submit" class="primary" disabled={!canOk || busy}>{ok}</button>
      {@render extra?.()}
      <button type="button" onclick={oncancel}>{t("common.cancel")}</button>
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
  .dlg {
    display: grid;
    gap: 12px;
    width: min(440px, 92vw);
    max-height: 90vh;
    overflow: auto;
    padding: 18px 22px;
    background: var(--bar);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 32px var(--popup-shadow);
  }
  .dlg.wide {
    width: min(620px, 94vw);
  }
  h1 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
  }
  .body {
    display: grid;
    gap: 10px;
    margin: 0;
    padding: 0;
    border: 0;
    min-width: 0;
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
  .dlg :global(fieldset) {
    display: grid;
    gap: 6px;
    margin: 0;
    padding: 0;
    border: 0;
    min-width: 0;
  }
  .dlg :global(legend) {
    padding: 0;
    margin-bottom: 6px;
    color: var(--muted);
  }
  .dlg :global(label) {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dlg :global(.note) {
    margin: 0;
    color: var(--muted);
  }
  .dlg :global(input[type="text"]),
  .dlg :global(input[type="number"]),
  .dlg :global(select) {
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 3px 8px;
    font: inherit;
    min-width: 0;
  }
  .dlg :global(input[type="number"]) {
    width: 5em;
  }
  .dlg :global(input[aria-invalid="true"]) {
    border-color: var(--error);
  }
  .dlg :global(button) {
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 4px 12px;
    font: inherit;
    cursor: pointer;
  }
  .dlg :global(button:hover:not(:disabled)) {
    background: var(--control-hover);
  }
  .dlg :global(button:disabled) {
    opacity: 0.5;
    cursor: default;
  }
  .dlg :global(.primary) {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent);
  }
  .dlg :global(.primary:hover:not(:disabled)) {
    background: var(--primary-hover);
  }
  .dlg :global(button:focus-visible),
  .dlg :global(input:focus-visible),
  .dlg :global(select:focus-visible) {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
