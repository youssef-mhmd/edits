<script lang="ts">
  // Settings: theme, language, updates; and the version.
  import { onMount, untrack } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { invoke } from "@tauri-apps/api/core";
  import { languages, t, type LangSetting } from "./i18n.svelte";
  import type { Theme } from "./look";

  interface Props {
    theme: Theme;
    language: LangSetting;
    checkUpdates: boolean;
    author: string;
    /** Used when `author` is empty. */
    defaultAuthor: string;
    onauthor: (a: string) => void;
    onchange: (s: { theme: Theme; language: LangSetting; checkUpdates: boolean }) => void;
    /** Looks for an update now; resolves to the new version, or null if up to date. */
    oncheck: () => Promise<string | null>;
    onclose: () => void;
  }
  let { theme, language, checkUpdates, author, defaultAuthor, onauthor, onchange, oncheck, onclose }: Props =
    $props();

  // Edited here; the app is told of every change.
  let th: Theme = $state(untrack(() => theme));
  let lg: LangSetting = $state(untrack(() => language));
  let up = $state(untrack(() => checkUpdates));
  let version = $state("");
  let checking = $state(false);
  let checkResult = $state("");
  let first: HTMLInputElement | null = $state(null);
  let licenses: string | null = $state(null);

  async function showLicenses() {
    licenses =
      licenses === null ? await invoke<string>("third_party_licenses").catch((e) => String(e)) : null;
  }

  function changed() {
    onchange({ theme: th, language: lg, checkUpdates: up });
  }

  async function checkNow() {
    checking = true;
    checkResult = "";
    const found = await oncheck();
    checking = false;
    // When there is one, the update banner shows it.
    checkResult = found ? "" : t("set.upToDate", { version });
  }

  onMount(() => {
    first?.focus();
    void getVersion()
      .then((v) => (version = v))
      .catch(() => {});
  });
</script>

<div
  class="overlay"
  role="presentation"
  onkeydown={(e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      onclose();
    }
  }}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="settings-title">
    <h1 id="settings-title">{t("set.title")}</h1>
    <fieldset>
      <legend>{t("set.theme")}</legend>
      <label>
        <input type="radio" bind:group={th} value="system" bind:this={first} onchange={changed} />
        {t("set.system")}
      </label>
      <label><input type="radio" bind:group={th} value="light" onchange={changed} /> {t("set.light")}</label>
      <label><input type="radio" bind:group={th} value="dark" onchange={changed} /> {t("set.dark")}</label>
    </fieldset>
    <fieldset>
      <legend>{t("set.language")}</legend>
      <label>
        <input type="radio" bind:group={lg} value="system" onchange={changed} />
        {t("set.system")}
      </label>
      {#each languages as l (l.id)}
        <label lang={l.id}>
          <input type="radio" bind:group={lg} value={l.id} onchange={changed} />
          {l.name}
        </label>
      {/each}
    </fieldset>
    <label class="field">
      {t("set.author")}
      <input
        type="text"
        dir="auto"
        value={author}
        placeholder={defaultAuthor}
        onchange={(e) => onauthor(e.currentTarget.value.trim())}
      />
    </label>
    <fieldset>
      <label><input type="checkbox" bind:checked={up} onchange={changed} /> {t("set.updates")}</label>
      <div class="check">
        <button type="button" onclick={checkNow} disabled={checking}>
          {checking ? t("set.checking") : t("set.checkNow")}
        </button>
        <span aria-live="polite">{checkResult}</span>
      </div>
    </fieldset>
    <p class="about">
      {#if version}<strong>{t("set.version", { version })}</strong><br />{/if}
      {t("set.about")}
      <button type="button" class="link" onclick={showLicenses}>{t("set.licenses")}</button>
    </p>
    {#if licenses !== null}
      <pre class="licenses" dir="ltr">{licenses}</pre>
    {/if}
    <div class="actions">
      <button type="button" class="primary" onclick={onclose}>{t("common.close")}</button>
    </div>
  </div>
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
    gap: 14px;
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
  .field {
    display: grid;
    gap: 4px;
    color: var(--muted);
  }
  .field input {
    background: var(--control);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 4px 8px;
    font: inherit;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--muted);
  }
  .about {
    margin: 0;
    color: var(--muted);
    line-height: 1.5;
  }
  .about strong {
    color: var(--text);
    font-weight: 600;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    text-decoration: underline;
  }
  .link:hover:not(:disabled) {
    background: none;
  }
  .licenses {
    max-height: 200px;
    overflow: auto;
    margin: 0;
    padding: 8px;
    background: var(--control);
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 11px;
    white-space: pre-wrap;
  }
  .actions {
    display: flex;
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
  .primary:hover {
    background: var(--primary-hover);
  }
  button:focus-visible,
  input:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
</style>
