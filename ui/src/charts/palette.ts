import { contrast, isDark, oklchToRgb, parseColor, rgbToOklch, solveLightness, toHex } from '../shell/appearance/color.ts';
import type { Oklch, Rgb } from '../shell/appearance/color';
import { colourDistance, NearestColours } from './paletteDistance.ts';


const CATEGORICAL = {
  dark: ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#9085e9', '#e66767', '#008300'],
  light: ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#4a3aa7', '#e34948', '#008300'],
} as const;
export const CATEGORY_LIMIT = CATEGORICAL.dark.length;
export const MARK_CONTRAST = 3;

const GOLDEN_ANGLE = 137.50776405003785;
const LIGHTNESS_STEP = 0.7548776662466927;
const CHROMA_STEP = 0.5698402909980532;
const DEFAULT_SURFACE = '#16181d';

export interface CatalogueColourItem<K> {
  readonly id: K;
  readonly color?: readonly [number, number, number] | Rgb | string | null;
}

function generatedColour(index: number, background: Rgb, darkSurface: boolean): string {
  const h = (195 + index * GOLDEN_ANGLE) % 360;
  const u = (0.35 + index * LIGHTNESS_STEP) % 1;
  const v = (0.55 + index * CHROMA_STEP) % 1;
  const l = darkSurface ? 0.56 + u * 0.33 : 0.24 + u * 0.33;
  const c = 0.08 + v * 0.14;
  return toHex(readable(oklchToRgb({ l, c, h }), background, darkSurface));
}

function suppliedColour(color: readonly [number, number, number] | Rgb | string, background: Rgb, darkSurface: boolean): string {
  if (typeof color === 'string') return color;
  const rgb: Rgb = 'r' in color ? color : { r: color[0], g: color[1], b: color[2] };
  return toHex(readable(rgb, background, darkSurface));
}

export function catalogueColours<K extends string | number>(
  items: readonly (K | CatalogueColourItem<K>)[],
  surface = DEFAULT_SURFACE,
  palette?: readonly string[],
): Map<K, string> {
  const background = parseColor(surface) ?? { r: 0, g: 0, b: 0 };
  const darkSurface = isDark(background);
  const base = palette ?? categoricalPalette(surface);
  const result = new Map<K, string>();
  let generated = 0;
  for (const item of items) {
    const isEntry = typeof item === 'object' && item !== null;
    const id = isEntry ? item.id : item;
    if (result.has(id)) continue;
    const color = isEntry ? item.color : undefined;
    if (color != null) {
      result.set(id, suppliedColour(color, background, darkSurface));
    } else {
      const slot = generated++;
      result.set(id, slot < base.length ? base[slot] : generatedColour(slot - base.length, background, darkSurface));
    }
  }
  return result;
}

const palettes = new Map<string, string[]>();
export function categoricalPalette(surface: string): string[] {
  let palette = palettes.get(surface);
  if (!palette) {
    const background = parseColor(surface) ?? { r: 0, g: 0, b: 0 };
    const dark = isDark(background);
    palette = CATEGORICAL[dark ? 'dark' : 'light'].map(hex => toHex(readable(parseColor(hex)!, background, dark)));
    palettes.set(surface, palette);
  }
  return palette;
}

function readable(color: Rgb, background: Rgb, darkSurface: boolean): Rgb {
  if (contrast(color, background) >= MARK_CONTRAST) return color;
  return oklchToRgb(solveLightness(rgbToOklch(color), background, MARK_CONTRAST, darkSurface ? 'lighter' : 'darker'));
}

export function distance(first: Rgb, second: Rgb): number {
  return colourDistance(first, second);
}

export const GREY_CHROMA = 0.045;

export function nudgeApart(colors: readonly Rgb[], background: Rgb, minDistance = 12, avoid: readonly Rgb[] = []): Rgb[] {
  const darkSurface = isDark(background);
  const fallback = categoricalPalette(toHex(background)).map(hex => parseColor(hex)!);
  const result: Rgb[] = [];
  const neighbours = new NearestColours(avoid);
  for (const [position, color] of colors.entries()) {
    let base: Oklch = rgbToOklch(color);
    if (base.c < GREY_CHROMA) base = rgbToOklch(fallback[position % fallback.length]);
    let chosen = readable(oklchToRgb(base), background, darkSurface);
    const mark = (candidate: Rgb) => contrast(candidate, background) >= MARK_CONTRAST && rgbToOklch(candidate).c >= GREY_CHROMA;
    const gap = (candidate: Rgb) => neighbours.gap(candidate);
    if (!(mark(chosen) && gap(chosen) >= minDistance)) {
      const start = rgbToOklch(chosen);
      let best: { color: Rgb; gap: number } | null = null;
      for (let step = 1; step <= 70; step++) {
        const delta = (step % 2 ? 1 : -1) * Math.ceil(step / 2) * 0.02 * (darkSurface ? 1 : -1);
        const candidate = oklchToRgb({ ...start, c: Math.max(start.c, GREY_CHROMA * 2), l: Math.min(0.9, Math.max(0.2, start.l + delta)) });
        if (!mark(candidate)) continue;
        const apart = gap(candidate);
        if (apart >= minDistance) { best = { color: candidate, gap: apart }; break; }
        if (!best || apart > best.gap) best = { color: candidate, gap: apart };
      }
      if (best) chosen = best.color;
    }
    result.push(chosen);
    neighbours.add(chosen);
  }
  return result;
}

export interface LineStyle { color: string; primary: readonly number[]; secondary: readonly number[] }

export function lineDashes(round: number): { primary: readonly number[]; secondary: readonly number[] } {
  return round === 0 ? { primary: [], secondary: [6, 4] } : { primary: [2 * round, 3], secondary: [10 + 2 * round, 3, 2, 3] };
}

export const SECONDARY_DASH = lineDashes(0).secondary;

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
