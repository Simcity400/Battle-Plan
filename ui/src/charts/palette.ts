import { contrast, isDark, oklchToRgb, parseColor, rgbToOklch, solveLightness, toHex } from '../shell/appearance/color.ts';
import type { Oklch, Rgb } from '../shell/appearance/color';

/** Colours for things that are categories, not countries (slices, stacked parts,
 * advisor types). Countries keep their recorded map colours. The hue order is
 * fixed and never cycled: a ninth category folds into "Other". Each theme gets
 * its own steps, solved for contrast against that theme's surface. */

/** The validated categorical hues, in order, for dark and light surfaces. */
const CATEGORICAL = {
  dark: ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#9085e9', '#e66767', '#008300'],
  light: ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#4a3aa7', '#e34948', '#008300'],
} as const;
export const CATEGORY_LIMIT = CATEGORICAL.dark.length;
/** Graphical objects need 3:1 against their background (WCAG 1.4.11). */
export const MARK_CONTRAST = 3;

/** The categorical palette for a surface colour. */
export function categoricalPalette(surface: string): string[] {
  const background = parseColor(surface) ?? { r: 0, g: 0, b: 0 };
  const dark = isDark(background);
  return CATEGORICAL[dark ? 'dark' : 'light'].map(hex => toHex(readable(parseColor(hex)!, background, dark)));
}

function readable(color: Rgb, background: Rgb, darkSurface: boolean): Rgb {
  if (contrast(color, background) >= MARK_CONTRAST) return color;
  return oklchToRgb(solveLightness(rgbToOklch(color), background, MARK_CONTRAST, darkSurface ? 'lighter' : 'darker'));
}

/** Perceptual distance in OKLab, ×100 (a ΔE of about 2 is barely visible). */
export function distance(first: Rgb, second: Rgb): number {
  const a = rgbToOklch(first);
  const b = rgbToOklch(second);
  const [ax, ay] = [a.c * Math.cos(a.h * Math.PI / 180), a.c * Math.sin(a.h * Math.PI / 180)];
  const [bx, by] = [b.c * Math.cos(b.h * Math.PI / 180), b.c * Math.sin(b.h * Math.PI / 180)];
  return Math.hypot(a.l - b.l, ax - bx, ay - by) * 100;
}

/** Below this chroma a colour reads as grey, which charts reserve for "contested"
 * and "no data". */
export const GREY_CHROMA = 0.045;

/** Makes each colour readable on the background, not grey, and at least
 * `minDistance` from every earlier one, changing lightness first and chroma only
 * for greys. Hue is kept. Stable: the same inputs always give the same result,
 * and earlier colours never change because of later ones. */
export function nudgeApart(colors: readonly Rgb[], background: Rgb, minDistance = 12, avoid: readonly Rgb[] = []): Rgb[] {
  const darkSurface = isDark(background);
  // A grey has no real hue (its angle is rounding noise), so a grey input takes the
  // categorical colour for its position instead of an arbitrary one.
  const fallback = categoricalPalette(toHex(background)).map(hex => parseColor(hex)!);
  const result: Rgb[] = [];
  for (const [position, color] of colors.entries()) {
    let base: Oklch = rgbToOklch(color);
    if (base.c < GREY_CHROMA) base = rgbToOklch(fallback[position % fallback.length]);
    let chosen = readable(oklchToRgb(base), background, darkSurface);
    const mark = (candidate: Rgb) => contrast(candidate, background) >= MARK_CONTRAST && rgbToOklch(candidate).c >= GREY_CHROMA;
    const gap = (candidate: Rgb) => Math.min(...[...avoid, ...result].map(other => distance(candidate, other)));
    if (!(mark(chosen) && gap(chosen) >= minDistance)) {
      const start = rgbToOklch(chosen);
      let best: { color: Rgb; gap: number } | null = null;
      // Try lightness steps alternating away from and toward the surface, far
      // enough to reach either end of the usable range from any start.
      for (let step = 1; step <= 70; step++) {
        const delta = (step % 2 ? 1 : -1) * Math.ceil(step / 2) * 0.02 * (darkSurface ? 1 : -1);
        // Past about 0.9 lightness sRGB cannot hold enough chroma, and colours turn grey.
        const candidate = oklchToRgb({ ...start, c: Math.max(start.c, GREY_CHROMA * 2), l: Math.min(0.9, Math.max(0.2, start.l + delta)) });
        if (!mark(candidate)) continue;
        const apart = gap(candidate);
        if (apart >= minDistance) { best = { color: candidate, gap: apart }; break; }
        // If nothing clears every colour, the most distinct readable one is used.
        if (!best || apart > best.gap) best = { color: candidate, gap: apart };
      }
      if (best) chosen = best.color;
    }
    result.push(chosen);
  }
  return result;
}

/** A line's look: its colour, and the dashes of its primary line (none: solid) and
 * of a secondary line drawn with it (such as demand beside supply). */
export interface LineStyle { color: string; primary: readonly number[]; secondary: readonly number[] }

/** The dash patterns for one round of lines. Round 0 is the ordinary look (a solid
 * primary, a dashed secondary); each later round, used once every palette colour is
 * taken, has dashes of its own length, so no two rounds match however many there
 * are, and a primary pattern (two numbers, or none) never matches a secondary one
 * (four numbers after round 0; 6-4 in it, which no primary dash of 2n-3 equals). */
export function lineDashes(round: number): { primary: readonly number[]; secondary: readonly number[] } {
  return round === 0 ? { primary: [], secondary: [6, 4] } : { primary: [2 * round, 3], secondary: [10 + 2 * round, 3, 2, 3] };
}

/** One distinct palette colour per line: each takes the colour at its `preferred`
 * place (such as its place in the game's order, so it keeps its colour as others
 * come and go) when free, else the first colour no other line has. With no more
 * lines than colours, two lines never share one; past that, colours repeat. */
export function lineColours<K>(lines: readonly { key: K; preferred: number }[], palette: readonly string[]): Map<K, string> {
  const colours = new Map<K, string>();
  const taken = new Set<number>();
  const size = Math.max(1, palette.length);
  for (const line of [...lines].sort((a, b) => a.preferred - b.preferred)) {
    const preferred = Math.max(0, line.preferred) % size;
    const free = !taken.has(preferred) ? preferred : palette.findIndex((_, candidate) => !taken.has(candidate));
    const index = free < 0 ? preferred : free;
    taken.add(index);
    colours.set(line.key, palette[index]);
  }
  return colours;
}

/** Every line's style, distinct at any count: the first round of lines (as many as
 * the palette has colours, by `preferred`) takes `lineColours` with the ordinary
 * dashes, and each further round the colours again with its own `lineDashes`. */
export function lineStyles<K>(lines: readonly { key: K; preferred: number }[], palette: readonly string[]): Map<K, LineStyle> {
  const sorted = [...lines].sort((a, b) => a.preferred - b.preferred);
  const size = Math.max(1, palette.length);
  const styles = new Map<K, LineStyle>();
  for (let start = 0; start < sorted.length; start += size) {
    const dashes = lineDashes(start / size);
    for (const [key, color] of lineColours(sorted.slice(start, start + size), palette)) styles.set(key, { color, ...dashes });
  }
  return styles;
}
