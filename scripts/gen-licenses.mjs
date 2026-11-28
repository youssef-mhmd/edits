// Writes licenses/third-party.txt: every Rust crate and npm package that
// ships in the app, with its version and licence (PRD 10.1). The file is
// bundled with the installer and shown in Settings. Run after changing
// dependencies:  node scripts/gen-licenses.mjs

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// Rust: the app crate's normal (not build or dev) dependencies, for Windows.
const meta = JSON.parse(
  execFileSync(
    "cargo",
    ["metadata", "--format-version", "1", "--filter-platform", "x86_64-pc-windows-msvc", "--locked"],
    { cwd: root, maxBuffer: 1 << 28 },
  ).toString(),
);
const pkgs = new Map(meta.packages.map((p) => [p.id, p]));
const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
const app = meta.packages.find((p) => p.name === "edits");
const seen = new Set();
const stack = [app.id];
while (stack.length) {
  const id = stack.pop();
  if (seen.has(id)) continue;
  seen.add(id);
  for (const d of nodes.get(id).deps) {
    if (d.dep_kinds.some((k) => k.kind === null)) stack.push(d.pkg);
  }
}
const ours = new Set(["edits", "edits-core"]);
const crates = [...seen]
  .map((id) => pkgs.get(id))
  .filter((p) => !ours.has(p.name))
  .map((p) => `${p.name} ${p.version}  ${p.license ?? "see " + (p.repository ?? "crate")}`)
  .sort();

// npm: packages bundled into the front end (dependencies, not devDependencies).
const lock = JSON.parse(readFileSync(join(root, "package-lock.json"), "utf8"));
const npm = Object.entries(lock.packages)
  .filter(([path, p]) => path.startsWith("node_modules/") && !p.dev)
  .map(([path, p]) => `${path.slice("node_modules/".length)} ${p.version}  ${p.license ?? "unknown"}`)
  .sort();

const text = [
  "edits: third-party software",
  "",
  "PDFium (https://pdfium.googlesource.com/pdfium/): BSD-3-Clause and Apache-2.0.",
  "Binary build by pdfium-binaries (https://github.com/bblanchon/pdfium-binaries): MIT.",
  "Their licence texts are in the licenses/pdfium folder next to edits.exe.",
  "",
  "IBM Plex Sans Arabic (https://github.com/IBM/plex): SIL Open Font License 1.1,",
  "copyright 2017 IBM Corp. Licence text: licenses/OFL-IBM-Plex.txt.",
  "",
  "Text recognition models (detector-v2.onnx, recognizer-v6b-int8.onnx) from arabic-ocr",
  "(https://github.com/youssef-mhmd/arabic-ocr), Creative Commons Attribution-ShareAlike 4.0",
  "(https://creativecommons.org/licenses/by-sa/4.0/). Trained on text from Arabic, Persian and",
  "Urdu Wikipedia and Wikisource by their contributors (CC BY-SA 4.0), scans of public-domain",
  "books from Wikimedia Commons, and the Tanzil Quran text (Tanzil Project, CC BY 3.0,",
  "https://tanzil.net). Details: licenses/ocr-models.txt.",
  "",
  "ONNX Runtime 1.31.1 (https://onnxruntime.ai): MIT, copyright Microsoft Corporation.",
  "Licence and notices: licenses/onnxruntime.txt, licenses/onnxruntime-ThirdPartyNotices.txt.",
  "",
  `Rust crates (${crates.length}):`,
  ...crates.map((c) => "  " + c),
  "",
  `npm packages (${npm.length}):`,
  ...npm.map((c) => "  " + c),
  "",
  "Svelte (MIT) is compiled into the interface. The licence texts of these packages",
  "are in their source distributions on crates.io and npmjs.com.",
  "",
].join("\n");

mkdirSync(join(root, "licenses"), { recursive: true });
writeFileSync(join(root, "licenses", "third-party.txt"), text);
console.log(`licenses/third-party.txt: ${crates.length} crates, ${npm.length} npm packages`);
