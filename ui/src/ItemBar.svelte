<script lang="ts">
  // The chosen page items' look: text font, size, style, colour, alignment
  // and direction; shape and line colours and widths; drawing order,
  // grouping and alignment. Changes apply to every chosen item they fit.
  import type { ItemView, TextStyle } from "./api";
  import type { ItemLayer } from "./items";
  import { t } from "./i18n.svelte";

  interface Props {
    items: ItemView[];
    layer: ItemLayer;
    /** Installed font families: [name, has Arabic]. */
    fonts: [string, boolean][];
    /** A text style was chosen: new text boxes use it too. */
    onstyle: (s: Partial<TextStyle>) => void;
    onedittable: (it: ItemView) => void;
  }
  let { items, layer, fonts, onstyle, onedittable }: Props = $props();

  const COLORS: number[][] = [
    [0, 0, 0],
    [0.35, 0.35, 0.38],
    [1, 1, 1],
    [0.85, 0.15, 0.12],
    [0.95, 0.55, 0.1],
    [0.15, 0.55, 0.25],
    [0.15, 0.4, 0.8],
    [0.45, 0.2, 0.65],
  ];
  const css = (c: number[]) => `rgb(${c.map((v) => Math.round(v * 255)).join(" ")})`;
  const same = (a: number[] | null | undefined, b: number[] | null | undefined) =>
    !!a && !!b && a.every((v, k) => Math.abs(v - b[k]) < 0.01);

  const texts = $derived(items.filter((i) => i.kind.type === "text" || i.kind.type === "table"));
  const first = $derived(items[0]);
  const style: TextStyle | null = $derived.by(() => {
    const k = texts[0]?.kind;
    return k && (k.type === "text" || k.type === "table") ? k.style : null;
  });
  const lines = $derived(items.filter((i) => i.kind.type === "line"));
  const framed = $derived(items.filter((i) => i.kind.type === "text" || i.kind.type === "shape"));
  const stroke = $derived.by(() => {
    const k = first?.kind;
    if (!k) return null;
    if (k.type === "shape") return k.stroke;
    if (k.type === "line") return k.color;
    if (k.type === "text") return k.border;
    return null;
  });
  const fill = $derived.by(() => {
    const k = first?.kind;
    return k && (k.type === "shape" || k.type === "text") ? k.fill : null;
  });
  const width = $derived.by(() => {
    const k = first?.kind;
    if (!k) return 1;
    if (k.type === "shape" || k.type === "line") return k.width;
    if (k.type === "text") return k.border_width;
    return 1;
  });
  const grouped = $derived(items.some((i) => i.group !== null));

  function setStyle(s: Partial<TextStyle>) {
    layer.update((it) => {
      if (it.kind.type === "text" || it.kind.type === "table") it.kind.style = { ...it.kind.style, ...s };
    });
    onstyle(s);
  }

  function setStroke(c: number[] | null) {
    layer.update((it) => {
      const k = it.kind;
      if (k.type === "shape") k.stroke = c;
      else if (k.type === "line" && c) k.color = c;
      else if (k.type === "text") k.border = c;
    });
  }

  function setFill(c: number[] | null) {
    layer.update((it) => {
      const k = it.kind;
      if (k.type === "shape" || k.type === "text") k.fill = c;
    });
  }

  function setWidth(w: number) {
    layer.update((it) => {
      const k = it.kind;
      if (k.type === "shape" || k.type === "line") k.width = w;
      else if (k.type === "text") k.border_width = w;
    });
  }
</script>

<span class="itembar" role="group" aria-label={t("item.bar")}>
  {#if style}
    <select
      class="font"
      value={style.font}
      title={t("item.font")}
      aria-label={t("item.font")}
      onchange={(e) => setStyle({ font: e.currentTarget.value })}
    >
      <option value="">IBM Plex Sans Arabic</option>
      {#each fonts as [name, arabic] (name)}
        <option value={name}>{name}{arabic ? " ع" : ""}</option>
      {/each}
    </select>
    <input
      class="size"
      type="number"
      min="4"
      max="400"
      step="1"
      value={style.size}
      title={t("item.size")}
      aria-label={t("item.size")}
      onchange={(e) => {
        const v = Number(e.currentTarget.value);
        if (v >= 4 && v <= 400) setStyle({ size: v });
      }}
    />
    <button
      class:active={style.bold}
      aria-pressed={style.bold}
      title={t("item.bold")}
      onclick={() => setStyle({ bold: !style.bold })}
    >
      <b>B</b>
    </button>
    <button
      class:active={style.italic}
      aria-pressed={style.italic}
      title={t("item.italic")}
      onclick={() => setStyle({ italic: !style.italic })}
    >
      <i>I</i>
    </button>
    <span class="swatches" role="group" aria-label={t("item.textColor")}>
      {#each COLORS as c (c.join())}
        <button
          class="swatch"
          class:active={same(style.color, c)}
          style:background={css(c)}
          title={t("item.textColor")}
          aria-label={css(c)}
          onclick={() => setStyle({ color: c })}
        ></button>
      {/each}
    </span>
    <select
      value={style.align}
      title={t("item.align")}
      aria-label={t("item.align")}
      onchange={(e) => setStyle({ align: e.currentTarget.value as TextStyle["align"] })}
    >
      <option value="start">{t("item.alignStart")}</option>
      <option value="right">{t("item.alignRight")}</option>
      <option value="center">{t("item.alignCenter")}</option>
      <option value="left">{t("item.alignLeft")}</option>
    </select>
    <select
      value={style.rtl === null ? "auto" : style.rtl ? "rtl" : "ltr"}
      title={t("item.dir")}
      aria-label={t("item.dir")}
      onchange={(e) => {
        const v = e.currentTarget.value;
        setStyle({ rtl: v === "auto" ? null : v === "rtl" });
      }}
    >
      <option value="auto">{t("item.dirAuto")}</option>
      <option value="rtl">{t("item.dirRtl")}</option>
      <option value="ltr">{t("item.dirLtr")}</option>
    </select>
  {/if}
  {#if framed.length || lines.length}
    <span class="sep"></span>
    <span class="swatches" role="group" aria-label={t("item.stroke")}>
      {#if !lines.length}
        <button
          class="swatch none"
          class:active={!stroke}
          title={t("item.noStroke")}
          aria-label={t("item.noStroke")}
          onclick={() => setStroke(null)}
        >
          ⃠
        </button>
      {/if}
      {#each COLORS as c (c.join())}
        <button
          class="swatch ring"
          class:active={same(stroke, c)}
          style:border-color={css(c)}
          title={t("item.stroke")}
          aria-label={css(c)}
          onclick={() => setStroke(c)}
        ></button>
      {/each}
    </span>
    <label class="width">
      {t("ctx.width")}
      <select value={String(width)} onchange={(e) => setWidth(Number(e.currentTarget.value))}>
        {#each [0.5, 1, 2, 3, 5, 8] as w (w)}<option value={String(w)}>{w} pt</option>{/each}
      </select>
    </label>
  {/if}
  {#if framed.length}
    <span class="swatches" role="group" aria-label={t("item.fill")}>
      <button
        class="swatch none"
        class:active={!fill}
        title={t("item.noFill")}
        aria-label={t("item.noFill")}
        onclick={() => setFill(null)}
      >
        ⃠
      </button>
      {#each COLORS.concat([[1, 0.95, 0.6], [0.85, 0.92, 1], [0.9, 0.97, 0.88]]) as c (c.join())}
        <button
          class="swatch"
          class:active={same(fill, c)}
          style:background={css(c)}
          title={t("item.fill")}
          aria-label={css(c)}
          onclick={() => setFill(c)}
        ></button>
      {/each}
    </span>
  {/if}
  {#if lines.length}
    {@const arrow = lines.every((l) => l.kind.type === "line" && l.kind.arrow)}
    <button
      class:active={arrow}
      aria-pressed={arrow}
      title={t("item.arrow")}
      onclick={() =>
        layer.update((it) => {
          if (it.kind.type === "line") it.kind.arrow = !arrow;
        })}
    >
      →
    </button>
  {/if}
  {#if items.length === 1 && first?.kind.type === "table"}
    {@const tb = first}
    <button onclick={() => onedittable(tb)}>{t("item.editTable")}</button>
  {/if}
  <span class="sep"></span>
  <button title={t("item.front")} aria-label={t("item.front")} onclick={() => layer.order("front")}>⤒</button>
  <button title={t("item.back")} aria-label={t("item.back")} onclick={() => layer.order("back")}>⤓</button>
  {#if items.length > 1}
    <button title={t("item.group")} onclick={() => layer.group()}>{t("item.groupShort")}</button>
  {/if}
  {#if grouped}
    <button title={t("item.ungroup")} onclick={() => layer.ungroup()}>{t("item.ungroupShort")}</button>
  {/if}
  <select
    value=""
    title={t("item.alignItems")}
    aria-label={t("item.alignItems")}
    onchange={(e) => {
      const v = e.currentTarget.value;
      e.currentTarget.value = "";
      if (v) layer.align(v as "left");
    }}
  >
    <option value="">{items.length > 1 ? t("item.alignItems") : t("item.alignPage")}</option>
    <option value="left">{t("item.toLeft")}</option>
    <option value="center">{t("item.toCenter")}</option>
    <option value="right">{t("item.toRight")}</option>
    <option value="top">{t("item.toTop")}</option>
    <option value="middle">{t("item.toMiddle")}</option>
    <option value="bottom">{t("item.toBottom")}</option>
  </select>
  <button title={t("item.duplicate")} aria-label={t("item.duplicate")} onclick={() => layer.duplicate()}>
    ⧉
  </button>
  <button title={t("ctx.deleteTitle")} onclick={() => layer.remove()}>{t("ctx.delete")}</button>
</span>

<style>
  .itembar {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: nowrap;
  }
  .font {
    max-width: 170px;
  }
  .size {
    width: 4.5em;
  }
  .swatches {
    display: flex;
    gap: 2px;
  }
  .swatch {
    width: 18px;
    height: 18px;
    min-width: 18px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 50%;
    font-size: 11px;
    line-height: 1;
  }
  .swatch.ring {
    background: transparent;
    border-width: 3px;
  }
  .swatch.none {
    background: var(--control);
    color: var(--muted);
  }
  .swatch.active {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .width {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--muted);
  }
  button.active {
    background: var(--selected);
  }
</style>
