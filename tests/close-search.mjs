// Starts a search in the running app over the DevTools protocol, for
// tests/close.ps1 ("search running" case). usage: node tests/close-search.mjs PORT
const port = process.argv[2];
const t = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find((x) => x.type === "page");
const ws = new WebSocket(t.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0;
const evaluate = (expression) =>
  new Promise((resolve) => {
    const n = ++id;
    ws.addEventListener("message", function on(m) {
      const msg = JSON.parse(m.data);
      if (msg.id === n) {
        ws.removeEventListener("message", on);
        resolve(msg.result?.result?.value);
      }
    });
    ws.send(
      JSON.stringify({ id: n, method: "Runtime.evaluate", params: { expression, returnByValue: true } }),
    );
  });
await evaluate(
  "window.dispatchEvent(new KeyboardEvent('keydown', { key: 'f', ctrlKey: true, bubbles: true }))",
);
await new Promise((r) => setTimeout(r, 300));
await evaluate(
  "(() => { const i = document.querySelector('.searchbar input'); i.value = 'the'; i.dispatchEvent(new Event('input', { bubbles: true })); })()",
);
await new Promise((r) => setTimeout(r, 400));
console.log(
  "  search status at close: " +
    (await evaluate("document.querySelector('.searchbar .status')?.textContent ?? 'no search bar'")),
);
process.exit(0);
