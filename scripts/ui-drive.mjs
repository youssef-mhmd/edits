// Drives the edits app through WebView2's DevTools protocol.
// usage: node scripts/ui-drive.mjs <pdf or ""> <steps.json> <screenshot dir>
// Needs a release build (npx tauri build --no-bundle). The viewer is on window.__viewer.
// steps: [{eval: "js"}, {key: "PageDown", ctrl?, shift?, code?}, {click: "css selector"},
//         {select: ["css", "value"]}, {wait: ms}, {shot: "name"}, {log: "js expr"}]
import { spawn } from "node:child_process";
import { writeFileSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";

const [pdf, stepsFile, outDir] = process.argv.slice(2);
const steps = JSON.parse(readFileSync(stepsFile, "utf8"));
const port = 9333;
const exe = fileURLToPath(new URL("../target/release/edits.exe", import.meta.url));
const stateDir = process.env.EDITS_STATE_DIR ?? `${tmpdir()}/edits-ui-test-state`;
const app = spawn(exe, pdf ? pdf.split("|") : [], {
  env: {
    ...process.env,
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
    // Never hand off to, or save into, the user's own running app.
    // UI_SINGLE_INSTANCE=1 keeps it on, to test second launches ({"launch": FILE} steps).
    ...(process.env.UI_SINGLE_INSTANCE ? {} : { EDITS_NO_SINGLE_INSTANCE: "1" }),
    EDITS_STATE_DIR: stateDir,
  },
  stdio: "ignore",
});
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let target;
for (let i = 0; i < 100 && !target; i++) {
  await sleep(100);
  try {
    const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    target = list.find((t) => t.type === "page");
  } catch {}
}
if (!target) throw new Error("no debug target");
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0;
const pending = new Map();
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  }
};
const send = (method, params = {}) =>
  new Promise((r) => {
    const n = ++id;
    pending.set(n, r);
    ws.send(JSON.stringify({ id: n, method, params }));
  });
const evaluate = async (expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails));
  return r.result?.result?.value;
};

await sleep(1500);
for (const s of steps) {
  if (s.eval) await evaluate(s.eval);
  if (s.click) await evaluate(`document.querySelector(${JSON.stringify(s.click)}).click()`);
  if (s.select)
    await evaluate(`(() => { const el = document.querySelector(${JSON.stringify(s.select[0])});
      el.value = ${JSON.stringify(s.select[1])}; el.dispatchEvent(new Event('change', {bubbles: true})); })()`);
  if (s.key) {
    const mods = (s.alt ? 1 : 0) | (s.ctrl ? 2 : 0) | (s.shift ? 8 : 0);
    const base = { key: s.key, code: s.code ?? s.key, modifiers: mods, windowsVirtualKeyCode: s.vk ?? 0 };
    await send("Input.dispatchKeyEvent", { type: "keyDown", ...base });
    await send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
  }
  if (s.launch) {
    // A second launch, as a double-click in Explorer would do.
    const second = spawn(exe, [s.launch], {
      env: { ...process.env, EDITS_STATE_DIR: stateDir },
      stdio: "ignore",
    });
    await new Promise((r) => second.on("exit", r));
  }
  if (s.wait) await sleep(s.wait);
  if (s.log) console.log(s.log, "=>", JSON.stringify(await evaluate(s.log)));
  if (s.shot) {
    const r = await send("Page.captureScreenshot", { format: "png" });
    writeFileSync(`${outDir}/${s.shot}.png`, Buffer.from(r.result.data, "base64"));
  }
}
ws.close();
app.kill();
process.exit(0);
