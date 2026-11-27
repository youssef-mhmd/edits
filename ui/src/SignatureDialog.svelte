<script lang="ts">
  // Make a signature: drawn with the mouse or a pen, typed, or from a
  // picture. It becomes a PNG with a transparent background, trimmed to the
  // ink, and is kept on this computer for reuse.
  import { onMount } from "svelte";
  import { open as pickFile } from "@tauri-apps/plugin-dialog";
  import Dialog from "./Dialog.svelte";
  import { pictureDataUrl } from "./api";
  import { t } from "./i18n.svelte";

  interface Props {
    /** Receives the signature (a PNG data URL) and whether to keep it. */
    ondone: (png: string, keep: boolean) => void;
    onclose: () => void;
  }
  let { ondone, onclose }: Props = $props();

  const W = 600;
  const H = 200;
  const FONTS = [
    ["Segoe Script", "Segoe Script"],
    ["Ink Free", "Ink Free"],
    ["Aldhabi", "Aldhabi (عربي)"],
    ["Segoe UI", "Segoe UI"],
  ];
  const INKS = ["#000000", "#1a3a8c"];

  let mode = $state<"draw" | "type" | "picture">("draw");
  let canvas: HTMLCanvasElement | null = $state(null);
  let name = $state("");
  let font = $state(FONTS[0][0]);
  let ink = $state(INKS[0]);
  let keep = $state(true);
  let clearWhite = $state(true);
  let picture: HTMLImageElement | null = $state(null);
  let drawn = $state(false);
  let error = $state("");
  let strokes: number[][][] = [];

  const ctx = () => canvas?.getContext("2d") ?? null;

  function redraw() {
    const c = ctx();
    if (!c || !canvas) return;
    c.clearRect(0, 0, W, H);
    c.strokeStyle = ink;
    c.fillStyle = ink;
    if (mode === "draw") {
      c.lineWidth = 3;
      c.lineCap = "round";
      c.lineJoin = "round";
      for (const s of strokes) {
        c.beginPath();
        s.forEach(([x, y], k) => (k ? c.lineTo(x, y) : c.moveTo(x, y)));
        if (s.length === 1) c.lineTo(s[0][0] + 0.1, s[0][1]);
        c.stroke();
      }
    } else if (mode === "type" && name.trim()) {
      let size = 90;
      c.direction = "inherit";
      c.textBaseline = "middle";
      c.textAlign = "center";
      do {
        c.font = `${size}px "${font}"`;
        size -= 4;
      } while (c.measureText(name).width > W - 40 && size > 12);
      c.fillText(name, W / 2, H / 2);
    } else if (mode === "picture" && picture) {
      const k = Math.min(W / picture.width, H / picture.height);
      const w = picture.width * k;
      const h = picture.height * k;
      c.drawImage(picture, (W - w) / 2, (H - h) / 2, w, h);
      if (clearWhite) {
        // Near-white paper becomes transparent.
        const img = c.getImageData(0, 0, W, H);
        const d = img.data;
        for (let i = 0; i < d.length; i += 4) {
          const light = Math.min(d[i], d[i + 1], d[i + 2]);
          if (light > 200) d[i + 3] = Math.round(d[i + 3] * Math.max(0, (235 - light) / 35));
        }
        c.putImageData(img, 0, 0);
      }
    }
  }

  $effect(() => {
    // Draw again when anything shown changes.
    void [mode, name, font, ink, picture, clearWhite, canvas];
    redraw();
  });

  // ------------------------------------------------------------ drawing
  let pen = false;
  function at(e: PointerEvent): number[] {
    const r = canvas!.getBoundingClientRect();
    return [((e.clientX - r.left) * W) / r.width, ((e.clientY - r.top) * H) / r.height];
  }
  function down(e: PointerEvent) {
    if (mode !== "draw" || !canvas) return;
    pen = true;
    canvas.setPointerCapture(e.pointerId);
    strokes.push([at(e)]);
    drawn = true;
    redraw();
  }
  function move(e: PointerEvent) {
    if (!pen) return;
    strokes[strokes.length - 1].push(at(e));
    redraw();
  }
  function up() {
    pen = false;
  }
  function clear() {
    strokes = [];
    drawn = false;
    redraw();
  }

  async function choosePicture() {
    const path = await pickFile({ filters: [{ name: t("tool.image"), extensions: ["png", "jpg", "jpeg"] }] });
    if (typeof path !== "string") return;
    try {
      const url = await pictureDataUrl(path);
      const img = new Image();
      img.onload = () => (picture = img);
      img.src = url;
    } catch (e) {
      error = String(e);
    }
  }

  const ready = $derived(
    (mode === "draw" && drawn) ||
      (mode === "type" && name.trim() !== "") ||
      (mode === "picture" && !!picture),
  );

  /** The canvas trimmed to what is drawn on it, with a small margin, as PNG. */
  function finish() {
    if (!canvas) return;
    const c = ctx()!;
    const d = c.getImageData(0, 0, W, H).data;
    let [x0, y0, x1, y1] = [W, H, -1, -1];
    for (let y = 0; y < H; y++)
      for (let x = 0; x < W; x++)
        if (d[(y * W + x) * 4 + 3] > 8) {
          x0 = Math.min(x0, x);
          y0 = Math.min(y0, y);
          x1 = Math.max(x1, x);
          y1 = Math.max(y1, y);
        }
    if (x1 < 0) return;
    const m = 6;
    [x0, y0] = [Math.max(0, x0 - m), Math.max(0, y0 - m)];
    [x1, y1] = [Math.min(W - 1, x1 + m), Math.min(H - 1, y1 + m)];
    const out = document.createElement("canvas");
    // Twice the size, for print.
    out.width = (x1 - x0 + 1) * 2;
    out.height = (y1 - y0 + 1) * 2;
    out.getContext("2d")!.drawImage(canvas, x0, y0, x1 - x0 + 1, y1 - y0 + 1, 0, 0, out.width, out.height);
    ondone(out.toDataURL("image/png"), keep);
    onclose();
  }

  onMount(() => redraw());
</script>

<Dialog title={t("sig.title")} ok={t("sig.ok")} canOk={ready} {error} onok={finish} oncancel={onclose} wide>
  <div class="tabs" role="tablist">
    {#each [["draw", t("sig.draw")], ["type", t("sig.type")], ["picture", t("sig.picture")]] as [id, label] (id)}
      <button
        type="button"
        role="tab"
        aria-selected={mode === id}
        class:on={mode === id}
        onclick={() => (mode = id as typeof mode)}
      >
        {label}
      </button>
    {/each}
  </div>
  <canvas
    bind:this={canvas}
    width={W}
    height={H}
    class:drawing={mode === "draw"}
    aria-label={t("sig.area")}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={up}
  ></canvas>
  {#if mode === "draw"}
    <div class="row">
      <span class="note">{t("sig.drawHint")}</span>
      <button type="button" onclick={clear} disabled={!drawn}>{t("sig.clear")}</button>
    </div>
  {:else if mode === "type"}
    <div class="row">
      <input
        type="text"
        dir="auto"
        bind:value={name}
        placeholder={t("sig.name")}
        aria-label={t("sig.name")}
      />
      <select bind:value={font} aria-label={t("sig.font")}>
        {#each FONTS as [f, label] (f)}<option value={f} style:font-family={f}>{label}</option>{/each}
      </select>
    </div>
  {:else}
    <div class="row">
      <button type="button" onclick={choosePicture}>{t("sig.choose")}</button>
      <label><input type="checkbox" bind:checked={clearWhite} /> {t("sig.clearWhite")}</label>
    </div>
  {/if}
  <div class="row">
    <span>{t("sig.ink")}</span>
    {#each INKS as c (c)}
      <button
        type="button"
        class="swatch"
        class:on={ink === c}
        style:background={c}
        aria-label={c}
        aria-pressed={ink === c}
        onclick={() => (ink = c)}
      ></button>
    {/each}
    <span class="grow"></span>
    <label><input type="checkbox" bind:checked={keep} /> {t("sig.keep")}</label>
  </div>
</Dialog>

<style>
  .tabs {
    display: flex;
    gap: 4px;
  }
  .tabs button.on {
    background: var(--selected);
  }
  canvas {
    width: 100%;
    aspect-ratio: 3 / 1;
    background: var(--paper);
    border: 1px solid var(--border);
    border-radius: 4px;
    touch-action: none;
  }
  canvas.drawing {
    cursor: crosshair;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .row input[type="text"] {
    flex: 1;
  }
  .grow {
    flex: 1;
  }
  .swatch {
    width: 22px;
    height: 22px;
    padding: 0;
    border-radius: 50%;
  }
  .swatch.on {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
</style>
