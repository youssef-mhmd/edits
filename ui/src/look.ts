// Theme and language, applied before the first frame. The settings live in
// state.json (recent.ts), which loads asynchronously; a copy in
// localStorage lets start-up apply them at once, without a flash of the
// wrong theme or direction.

import { resolveLang, setLang, type LangSetting } from "./i18n.svelte";

export type Theme = "system" | "light" | "dark";

const KEY = "edits.look";
const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
let theme: Theme = "system";

function paint() {
  const dark = theme === "dark" || (theme === "system" && darkQuery.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

darkQuery.addEventListener("change", () => {
  if (theme === "system") paint();
});

/** Applies a theme and language and remembers them for the next start. */
export function applyLook(t: Theme, l: LangSetting) {
  theme = t;
  paint();
  setLang(resolveLang(l));
  try {
    localStorage.setItem(KEY, JSON.stringify({ theme: t, lang: l }));
  } catch {
    // Storage refused: the settings still apply once state.json loads.
  }
}

/** Applies the theme and language last used, before the app mounts. */
export function bootLook() {
  let saved: { theme?: Theme; lang?: LangSetting };
  try {
    saved = JSON.parse(localStorage.getItem(KEY) ?? "{}") ?? {};
  } catch {
    saved = {};
  }
  theme = saved.theme === "light" || saved.theme === "dark" ? saved.theme : "system";
  paint();
  setLang(resolveLang(saved.lang === "en" || saved.lang === "ar" ? saved.lang : "system"));
}
