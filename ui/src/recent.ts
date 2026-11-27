// Recent files and where the reader left each one, saved in the app's data
// folder through the Rust side (state.json).

import { loadState, saveState, type SearchOptions } from "./api";
import type { LangSetting } from "./i18n.svelte";
import type { Theme } from "./look";
import type { ViewState } from "./viewer";

export interface RecentEntry extends Partial<ViewState> {
  path: string;
  name: string;
  pages: number;
  /** Unix ms of the last time it was open. */
  opened: number;
}

export interface Settings {
  sidebarOpen: boolean;
  sidebarPanel: "pages" | "bookmarks";
  searchOptions: SearchOptions;
  theme: Theme;
  language: LangSetting;
  checkUpdates: boolean;
  printDpi: number;
  /** Name written on annotations; empty uses the Windows user name. */
  author: string;
  /** Markup colour, red, green, blue from 0 to 1. */
  markupColor: number[];
}

interface Saved {
  version: 1;
  recent: RecentEntry[];
  settings: Settings;
}

const MAX_RECENT = 30;
const SAVE_DELAY = 800;

let saved: Saved = {
  version: 1,
  recent: [],
  settings: {
    sidebarOpen: false,
    sidebarPanel: "pages",
    searchOptions: { match_case: false, whole_word: false, ignore_diacritics: true, letter_variants: true },
    theme: "system",
    language: "system",
    checkUpdates: true,
    printDpi: 150,
    author: "",
    markupColor: [1, 0.92, 0.23],
  },
};
let timer = 0;

/** Windows paths compare without case. */
const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

export async function loadRecent(): Promise<void> {
  try {
    const s = (await loadState()) as Partial<Saved> | null;
    if (s && s.version === 1 && Array.isArray(s.recent)) {
      saved = {
        version: 1,
        recent: s.recent.filter((r) => r && typeof r.path === "string").slice(0, MAX_RECENT),
        settings: { ...saved.settings, ...(s.settings ?? {}) },
      };
    }
  } catch {
    // No saved state, or unreadable: start fresh.
  }
}

function scheduleSave() {
  clearTimeout(timer);
  timer = window.setTimeout(() => void saveState(saved).catch(() => {}), SAVE_DELAY);
}

/** Saves now, for example before the window closes. */
export function flushRecent(): Promise<void> {
  clearTimeout(timer);
  return saveState(saved).catch(() => {});
}

export const recentFiles = (): RecentEntry[] => saved.recent;

export const lastView = (path: string): RecentEntry | undefined =>
  saved.recent.find((r) => same(r.path, path));

/** Records `path` as the most recent file, with its current view. */
export function remember(path: string, name: string, pages: number, view: ViewState) {
  const rest = saved.recent.filter((r) => !same(r.path, path));
  saved.recent = [{ path, name, pages, opened: Date.now(), ...view }, ...rest].slice(0, MAX_RECENT);
  scheduleSave();
}

export function forget(path: string) {
  saved.recent = saved.recent.filter((r) => !same(r.path, path));
  scheduleSave();
}

export const settings = (): Settings => saved.settings;

export function updateSettings(s: Partial<Settings>) {
  saved.settings = { ...saved.settings, ...s };
  scheduleSave();
}
