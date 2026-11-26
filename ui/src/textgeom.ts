// Text geometry on one page: hit-testing, selection rectangles, words and
// lines. Works on the page text from the engine (characters in reading order
// with a box each, in unrotated page fractions).

export interface LoadedText {
  text: string;
  boxes: Float32Array;
}

/** True for letters of right-to-left scripts. */
const RTL = /[\u0590-\u08ff\ufb1d-\ufdff\ufe70-\ufeff]/;
const WORD = /[\p{L}\p{N}\p{M}_]/u;

const hasBox = (b: Float32Array, i: number) => b[4 * i + 2] > b[4 * i] || b[4 * i + 3] > b[4 * i + 1];

/**
 * Caret position (0..length) nearest to the point (fx, fy), in unrotated page
 * fractions. A point on the trailing half of a character (its left half for
 * right-to-left letters) puts the caret after it.
 */
export function caretAt(t: LoadedText, fx: number, fy: number): number {
  const b = t.boxes;
  let best = -1;
  let bestD = Infinity;
  for (let i = 0; i < t.text.length; i++) {
    if (!hasBox(b, i)) continue;
    const [l, top, r, bot] = [b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]];
    const dx = fx < l ? l - fx : fx > r ? fx - r : 0;
    const dy = fy < top ? top - fy : fy > bot ? fy - bot : 0;
    // Being on the right line matters more than being near horizontally.
    const d = dx + dy * 4;
    if (d < bestD) {
      bestD = d;
      best = i;
      if (d === 0) break;
    }
  }
  if (best < 0) return 0;
  const mid = (b[4 * best] + b[4 * best + 2]) / 2;
  const after = RTL.test(t.text[best]) ? fx < mid : fx > mid;
  return after ? best + 1 : best;
}

/** True when (fx, fy) is on a character, for the text cursor. */
export function overText(t: LoadedText, fx: number, fy: number): boolean {
  const b = t.boxes;
  for (let i = 0; i < t.text.length; i++) {
    if (fx >= b[4 * i] && fx <= b[4 * i + 2] && fy >= b[4 * i + 1] && fy <= b[4 * i + 3] && hasBox(b, i))
      return true;
  }
  return false;
}

/**
 * Rectangles covering characters [start, end): one per run of characters on
 * the same line, each spanning the run's full height.
 */
export function rangeRects(t: LoadedText, start: number, end: number): [number, number, number, number][] {
  const b = t.boxes;
  const out: [number, number, number, number][] = [];
  let cur: [number, number, number, number] | null = null;
  for (let i = Math.max(0, start); i < Math.min(end, t.text.length); i++) {
    if (t.text[i] === "\n") {
      cur = null;
      continue;
    }
    if (!hasBox(b, i)) continue;
    const r: [number, number, number, number] = [b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]];
    if (cur) {
      // Same line if the vertical centres are within half a line height.
      const h = Math.max(cur[3] - cur[1], r[3] - r[1]);
      if (Math.abs((cur[1] + cur[3]) / 2 - (r[1] + r[3]) / 2) < h / 2) {
        cur[0] = Math.min(cur[0], r[0]);
        cur[1] = Math.min(cur[1], r[1]);
        cur[2] = Math.max(cur[2], r[2]);
        cur[3] = Math.max(cur[3], r[3]);
        continue;
      }
    }
    cur = [...r];
    out.push(cur);
  }
  return out;
}

/** The word around caret position `i`. */
export function wordAt(t: LoadedText, i: number): [number, number] {
  const s = t.text;
  let a = Math.min(i, s.length - 1);
  if (a >= 0 && !WORD.test(s[a]) && a > 0 && WORD.test(s[a - 1])) a--;
  if (a < 0 || !WORD.test(s[a])) return [Math.max(0, a), Math.max(0, a + 1)];
  let z = a;
  while (a > 0 && WORD.test(s[a - 1])) a--;
  while (z < s.length && WORD.test(s[z])) z++;
  return [a, z];
}

/** The line around caret position `i`. */
export function lineAt(t: LoadedText, i: number): [number, number] {
  const s = t.text;
  const a = s.lastIndexOf("\n", Math.max(0, i - 1)) + 1;
  const z = s.indexOf("\n", i);
  return [a, z < 0 ? s.length : z];
}

/**
 * Text as it should reach the clipboard: Arabic presentation forms and Latin
 * ligatures become plain letters, so pasted text can be searched and edited.
 */
export function cleanCopy(s: string): string {
  return s.replace(/[\ufb00-\ufb06\ufb50-\ufdff\ufe70-\ufeff]/g, (c) => c.normalize("NFKC"));
}
