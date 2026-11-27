// Updates through the Tauri updater: a signed manifest on the project's
// GitHub releases (tauri.conf.json, plugins.updater). Checking never blocks
// start-up and never shows an error on its own; offline is normal.

import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type { Update };

/** The available update, or null if there is none or the check failed. */
export async function findUpdate(): Promise<Update | null> {
  try {
    return await check({ timeout: 15_000 });
  } catch {
    return null;
  }
}

/** Downloads and installs `u`, reporting progress in percent, then restarts. */
export async function installUpdate(u: Update, progress: (pct: number) => void): Promise<void> {
  let total = 0;
  let got = 0;
  await u.downloadAndInstall((e) => {
    if (e.event === "Started") total = e.data.contentLength ?? 0;
    else if (e.event === "Progress") {
      got += e.data.chunkLength;
      if (total) progress(Math.min(100, Math.round((got / total) * 100)));
    } else if (e.event === "Finished") progress(100);
  });
  // On Windows the installer closes the app itself; elsewhere restart it.
  await relaunch();
}
