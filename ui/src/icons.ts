// Ribbon and status bar icons: line drawings on a 20 × 20 grid, drawn with
// the text colour. Parts marked `a` use the icon's accent colour (a CSS
// variable chosen per icon), parts marked `f` are filled instead of stroked.

export interface Part {
  d: string;
  a?: boolean;
  f?: boolean;
}

/** Accent colour variable per icon group (app.css). */
export type Hue = "blue" | "green" | "orange" | "red" | "purple" | "teal" | "yellow";

const p = (d: string, a = false, f = false): Part => ({ d, a, f });

export const ICONS: Record<string, { hue?: Hue; parts: Part[] }> = {
  save: {
    hue: "purple",
    parts: [p("M4 3.5h9.5l2.5 2.5v10.5H4z"), p("M7 3.5v4h6v-4"), p("M6.5 16.5v-5h7v5", true)],
  },
  undo: { parts: [p("M7.5 12 3.5 8l4-4"), p("M3.5 8h8a4.5 4.5 0 0 1 0 9H9", true)] },
  redo: { parts: [p("M12.5 12l4-4-4-4"), p("M16.5 8h-8a4.5 4.5 0 0 0 0 9H11", true)] },
  print: {
    hue: "blue",
    parts: [p("M6 7V3h8v4"), p("M6 14H3.5V7.5h13V14H14"), p("M6 11.5h8v5.5H6z", true)],
  },
  settings: {
    parts: [
      p("M10 12.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z", true),
      p(
        "M8.6 2.5h2.8l.4 2.1 1.5.8 2-.8 1.4 2.4-1.6 1.4v1.7l1.6 1.4-1.4 2.4-2-.8-1.5.8-.4 2.1H8.6l-.4-2.1-1.5-.8-2 .8-1.4-2.4 1.6-1.4V8.4L3.3 7l1.4-2.4 2 .8 1.5-.8z",
      ),
    ],
  },
  back: { parts: [p("M12 4 6 10l6 6")] },
  forward: { parts: [p("M8 4l6 6-6 6")] },
  minus: { parts: [p("M4.5 10h11")] },
  plus: { parts: [p("M10 4.5v11M4.5 10h11")] },
  fitWidth: {
    hue: "teal",
    parts: [p("M5 3.5h10v13H5z"), p("M2 10h16M4 8l-2 2 2 2M16 8l2 2-2 2", true)],
  },
  fitPage: {
    hue: "teal",
    parts: [p("M6 2.5h8v15H6z"), p("M10 5v10M8.3 6.7 10 5l1.7 1.7M8.3 13.3 10 15l1.7-1.7", true)],
  },
  panel: {
    hue: "blue",
    parts: [p("M3 4h14v12H3z"), p("M7.5 4v12"), p("M4.5 6.5h1.5M4.5 9h1.5M4.5 11.5h1.5", true)],
  },
  pages: {
    hue: "blue",
    parts: [p("M3 3h6v6H3zM11 3h6v6h-6zM3 11h6v6H3z"), p("M11 11h6v6h-6z", true)],
  },
  continuous: { parts: [p("M6 1.5h8v7H6zM6 11.5h8v7H6z")] },
  single: { parts: [p("M5.5 2.5h9v15h-9z"), p("M8 6h4M8 8.5h4M8 11h2.5", true)] },
  twoPage: { parts: [p("M2.5 4h7v12h-7zM10.5 4h7v12h-7z")] },
  rtl: { hue: "teal", parts: [p("M16 10H4"), p("M7.5 6.5 4 10l3.5 3.5", true)] },
  rotateLeft: { hue: "orange", parts: [p("M4 9a6.5 6.5 0 1 1 2 6"), p("M3.5 4.5V9H8", true)] },
  rotateRight: { hue: "orange", parts: [p("M16 9a6.5 6.5 0 1 0-2 6"), p("M16.5 4.5V9H12", true)] },
  select: { parts: [p("M5 3l10 6.5-4.5 1L8 15z", true, true), p("M5 3l10 6.5-4.5 1L8 15z")] },
  snip: {
    hue: "teal",
    parts: [p("M3 6V3h3M14 3h3v3M17 14v3h-3M6 17H3v-3"), p("M6.5 8h7M6.5 10.5h7M6.5 13h4", true)],
  },
  find: {
    hue: "blue",
    parts: [p("M8.5 14a5.5 5.5 0 1 0 0-11 5.5 5.5 0 0 0 0 11z"), p("M12.5 12.5l4.5 4.5", true)],
  },
  pen: { hue: "blue", parts: [p("M13.5 3.5l3 3L7 16H4v-3z"), p("M11.5 5.5l3 3", true)] },
  line: { hue: "blue", parts: [p("M4 16 16 4", true)] },
  arrow: { hue: "blue", parts: [p("M4 16 15.5 4.5"), p("M9 4h7v7", true)] },
  rect: { hue: "blue", parts: [p("M3.5 5h13v10h-13z", true)] },
  ellipse: {
    hue: "blue",
    parts: [p("M10 15.5c3.6 0 6.5-2.5 6.5-5.5S13.6 4.5 10 4.5 3.5 7 3.5 10s2.9 5.5 6.5 5.5z", true)],
  },
  note: {
    hue: "yellow",
    parts: [p("M3.5 3.5h13v10l-3 3h-10z", true, true), p("M3.5 3.5h13v10l-3 3h-10z"), p("M13.5 16.5v-3h3")],
  },
  textbox: { hue: "orange", parts: [p("M2.5 4h15v12h-15z"), p("M6.5 7.5h7M10 7.5v6", true)] },
  stamp: {
    hue: "red",
    parts: [p("M8 9.5V7a2 2 0 1 1 4 0v2.5"), p("M3.5 9.5h13v4h-13z", true), p("M4.5 16.5h11")],
  },
  picture: {
    hue: "green",
    parts: [
      p("M2.5 4h15v12h-15z"),
      p("M2.5 14l4.5-4.5 3.5 3.5 2-2 5 5", true),
      p("M13.5 8.5a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3z", true),
    ],
  },
  tick: { hue: "green", parts: [p("M3.5 10.5l4 4 9-9", true)] },
  cross: { hue: "red", parts: [p("M5 5l10 10M15 5 5 15", true)] },
  sign: {
    hue: "purple",
    parts: [
      p("M2.5 16.5h15"),
      p("M3.5 12.5c2-4 3.5-7 4.5-6s-2 6.5-.5 6.5 2.5-3.5 3.5-3 0 2.5 1 2.5 2-1.5 3-1.5", true),
    ],
  },
  flatten: {
    hue: "purple",
    parts: [p("M10 3 2.5 7 10 11l7.5-4z", true), p("M2.5 10.5 10 14.5l7.5-4M2.5 13.5 10 17.5l7.5-4")],
  },
  ocr: {
    hue: "teal",
    parts: [p("M3 6V3h3M14 3h3v3M17 14v3h-3M6 17H3v-3"), p("M6 14l4-8 4 8M7.5 11h5", true)],
  },
  itext: { hue: "orange", parts: [p("M4 4.5h12M10 4.5v11", true), p("M7.5 15.5h5")] },
  table: {
    hue: "green",
    parts: [p("M2.5 3.5h15v13h-15z"), p("M2.5 7.5h15", true), p("M2.5 12h15M7.5 3.5v13M12.5 3.5v13")],
  },
  open: { hue: "yellow", parts: [p("M2.5 5V15.5h13l2-7H6l-2 7", true), p("M2.5 15.5V4h5l1.5 1.5h6v3")] },
  newDoc: {
    hue: "blue",
    parts: [p("M5 2.5h6.5L15.5 6.5v11H5z"), p("M11.5 2.5v4h4"), p("M10 9.5v5M7.5 12h5", true)],
  },
  merge: {
    hue: "purple",
    parts: [p("M3 3h6v8H3zM11 3h6v8h-6z"), p("M6 11v2.5h8V11M10 13.5V18M8 16l2 2 2-2", true)],
  },
  pictures: {
    hue: "green",
    parts: [p("M5.5 2.5h9v12h-9z"), p("M5.5 12.5l3-3 2 2 1.5-1.5 2.5 2.5", true), p("M3 5v12.5h9.5")],
  },
  chevron: { parts: [p("M5.5 8l4.5 4.5L14.5 8")] },
  close: { parts: [p("M5.5 5.5l9 9M14.5 5.5l-9 9")] },
  delete: { hue: "red", parts: [p("M3.5 5.5h13M8 5.5V3.5h4v2"), p("M5 5.5l1 11h8l1-11", true)] },
  monitor: { hue: "teal", parts: [p("M2.5 3.5h15v10h-15z"), p("M7 17h6M10 13.5V17", true)] },
  sun: {
    hue: "yellow",
    parts: [
      p("M10 13.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z", true, true),
      p("M10 13.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z", true),
      p("M10 1.5v2M10 16.5v2M1.5 10h2M16.5 10h2M4 4l1.4 1.4M14.6 14.6 16 16M4 16l1.4-1.4M14.6 5.4 16 4"),
    ],
  },
  moon: { hue: "purple", parts: [p("M16.5 12.5A7 7 0 0 1 7.5 3.5a7 7 0 1 0 9 9z", true)] },
  globe: {
    hue: "blue",
    parts: [
      p("M10 17.5a7.5 7.5 0 1 0 0-15 7.5 7.5 0 0 0 0 15z"),
      p("M2.5 10h15M10 2.5c-2.5 3-2.5 12 0 15M10 2.5c2.5 3 2.5 12 0 15", true),
    ],
  },
  person: {
    hue: "green",
    parts: [
      p("M10 9.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z", true),
      p("M3.5 17.5c0-3.5 3-5.5 6.5-5.5s6.5 2 6.5 5.5"),
    ],
  },
  refresh: {
    hue: "teal",
    parts: [
      p("M16 8a6.5 6.5 0 0 0-11.5-2.5M4 12a6.5 6.5 0 0 0 11.5 2.5"),
      p("M4.5 2v3.5H8M15.5 18v-3.5H12", true),
    ],
  },
  comment: { hue: "yellow", parts: [p("M3 4h14v9.5H9l-4 3v-3H3z", true)] },
};

const built = new Map<string, string>();

/** The icon as SVG markup at `size` CSS px (empty for an unknown name). */
export function iconSvg(name: string, size: number): string {
  const key = `${name}/${size}`;
  let svg = built.get(key);
  if (svg === undefined) {
    const icon = ICONS[name];
    const accent = icon?.hue ? `var(--hue-${icon.hue})` : "currentColor";
    const paths = (icon?.parts ?? [])
      .map((part) =>
        part.f
          ? `<path d="${part.d}" fill="${accent}" fill-opacity="0.28" stroke="none"/>`
          : `<path d="${part.d}" fill="none" stroke="${part.a ? accent : "currentColor"}"/>`,
      )
      .join("");
    svg = icon ? `<svg width="${size}" height="${size}" viewBox="0 0 20 20">${paths}</svg>` : "";
    built.set(key, svg);
  }
  return svg;
}
