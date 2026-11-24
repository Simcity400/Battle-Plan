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
const GENERATED_CONTRAST = 3.85;
const MAX_POOLS = 64;

export interface CatalogueColourItem<K> {
  readonly id: K;
  readonly color?: readonly [number, number, number] | Rgb | string | null;
}

interface GeneratedPool {
  readonly background: Rgb;
  readonly darkSurface: boolean;
  readonly target: number;
  readonly baseRgbs: readonly Rgb[];
  readonly colors: string[];
  readonly rgbs: Rgb[];
  readonly used: Set<string>;
  cursor: number;
}

const pools = new Map<string, GeneratedPool>();
const catalogueCache = new WeakMap<readonly unknown[], Map<string, Map<unknown, string>>>();

function roundRgb({ r, g, b }: Rgb): Rgb {
  return {
    r: Math.round(Math.min(255, Math.max(0, r))),
    g: Math.round(Math.min(255, Math.max(0, g))),
    b: Math.round(Math.min(255, Math.max(0, b))),
  };
}

function solveQuantised(color: Oklch, background: Rgb, darkSurface: boolean, minContrast: number): Rgb {
  const initial = roundRgb(oklchToRgb(color));
  if (contrast(initial, background) >= minContrast) return initial;
  let lo = color.l;
  let hi = darkSurface ? 1 : 0;
  let best = roundRgb(oklchToRgb({ ...color, l: hi }));
  for (let step = 0; step < 11; step++) {
    const mid = (lo + hi) / 2;
    const candidate = roundRgb(oklchToRgb({ ...color, l: mid }));
    if (contrast(candidate, background) >= minContrast) {
      best = candidate;
      hi = mid;
    } else {
      lo = mid;
    }
  }
  return best;
}

function poolFor(surface: string, base: readonly string[]): GeneratedPool {
  const key = `${surface}\0${base.join(',')}`;
  let pool = pools.get(key);
  if (pool) return pool;
  if (pools.size >= MAX_POOLS) pools.clear();
  const background = parseColor(surface) ?? { r: 0, g: 0, b: 0 };
  const darkSurface = isDark(background);
  const maxContrast = contrast(darkSurface ? { r: 255, g: 255, b: 255 } : { r: 0, g: 0, b: 0 }, background);
  const target = maxContrast >= 8 ? GENERATED_CONTRAST : MARK_CONTRAST;
  const used = new Set<string>();
  const baseRgbs: Rgb[] = [];
  for (const hex of base) {
    const parsed = parseColor(hex);
    if (parsed) baseRgbs.push(parsed);
    used.add(parsed ? toHex(parsed) : hex.toLowerCase());
  }
  pool = { background, darkSurface, target, baseRgbs, colors: [], rgbs: [], used, cursor: 0 };
  pools.set(key, pool);
  return pool;
}

function generatedAt(pool: GeneratedPool, index: number): string {
  const minBaseGap = pool.target > MARK_CONTRAST ? 3 : 0;
  const minNeighbourGap = pool.target > MARK_CONTRAST ? 12 : 0;
  while (pool.colors.length <= index) {
    const prev = pool.rgbs[pool.rgbs.length - 1];
    let accepted: { hex: string; rgb: Rgb } | null = null;
    for (let tries = 0; tries < 64; tries++) {
      const step = pool.cursor++;
      const h = (195 + step * GOLDEN_ANGLE) % 360;
      const u = (0.35 + step * LIGHTNESS_STEP) % 1;
      const v = (0.55 + step * CHROMA_STEP) % 1;
      const l = pool.darkSurface ? 0.62 + u * 0.27 : 0.22 + u * 0.30;
      const c = 0.08 + v * 0.14;
      const rgb = solveQuantised({ l, c, h }, pool.background, pool.darkSurface, pool.target);
      const hex = toHex(rgb);
      if (pool.used.has(hex)) continue;
      if (minBaseGap > 0 && !pool.baseRgbs.every(other => colourDistance(rgb, other) >= minBaseGap)) continue;
      if (prev && minNeighbourGap > 0 && colourDistance(rgb, prev) < minNeighbourGap) continue;
      accepted = { hex, rgb };
      break;
    }
    if (accepted === null) {
      const step = pool.cursor++;
      const h = (195 + step * GOLDEN_ANGLE) % 360;
      const u = (0.35 + step * LIGHTNESS_STEP) % 1;
      const v = (0.55 + step * CHROMA_STEP) % 1;
      const l = pool.darkSurface ? 0.62 + u * 0.27 : 0.22 + u * 0.30;
      const rgb = solveQuantised({ l, c: 0.08 + v * 0.14, h }, pool.background, pool.darkSurface, MARK_CONTRAST);
      accepted = { hex: toHex(rgb), rgb };
    }
    pool.used.add(accepted.hex);
    pool.colors.push(accepted.hex);
    pool.rgbs.push(accepted.rgb);
  }
  return pool.colors[index];
}

function suppliedColour(color: readonly [number, number, number] | Rgb | string): string {
  if (typeof color === 'string') return color;
  return toHex('r' in color ? color : { r: color[0], g: color[1], b: color[2] });
}

export function catalogueColours<K extends string | number>(
  items: readonly (K | CatalogueColourItem<K>)[],
  surface = DEFAULT_SURFACE,
  palette?: readonly string[],
): Map<K, string> {
  const base = palette ?? categoricalPalette(surface);
  const cacheKey = `${surface}\0${base.join(',')}`;
  let bySurface = catalogueCache.get(items);
  const cached = bySurface?.get(cacheKey);
  if (cached) return cached as Map<K, string>;
  const pool = poolFor(surface, base);
  const result = new Map<K, string>();
  let generated = 0;
  for (const item of items) {
    const isEntry = typeof item === 'object' && item !== null;
    const id = isEntry ? item.id : item;
    if (result.has(id)) continue;
    const color = isEntry ? item.color : undefined;
    if (color != null) {
      result.set(id, suppliedColour(color));
    } else {
      const slot = generated++;
      result.set(id, slot < base.length ? base[slot] : generatedAt(pool, slot - base.length));
    }
  }
  if (!bySurface) { bySurface = new Map(); catalogueCache.set(items, bySurface); }
  if (bySurface.size >= MAX_POOLS) bySurface.clear();
  bySurface.set(cacheKey, result as Map<unknown, string>);
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
