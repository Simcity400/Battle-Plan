import { contrast, isDark, mix, oklchToRgb, parseColor, rgbaColor, rgbToOklch, solveLightness, toHex } from '../shell/appearance/color.ts';
import type { Oklch, Rgb } from '../shell/appearance/color';
import { colourDistance, NearestColours } from './paletteDistance.ts';

const CATEGORICAL = {
  dark: ['#3987e5', '#d95926', '#199e70', '#c98500', '#d55181', '#9085e9', '#e66767', '#008300'],
  light: ['#2a78d6', '#eb6834', '#1baf7a', '#eda100', '#e87ba4', '#4a3aa7', '#e34948', '#008300'],
} as const;
export const CATEGORY_LIMIT = CATEGORICAL.dark.length;
export const MARK_CONTRAST = 3;

const MEASURE_COLOURS = {
  politicalPower: '#e3a83b',
  commandPower: '#c76a90',
  stability: '#7fa6d6',
  warSupport: '#c98a55',
  armyExperience: '#6cc46e',
  navyExperience: '#7d95c4',
  airExperience: '#d0767a',
  fuel: '#b5a48c',
  supplyOk: '#3fae63',
  supplyStrained: '#e3a83b',
  supplyNone: '#e5484d',
  bopLeft: '#c4705a',
  bopRight: '#4f8fcf',
  short: '#e5484d',
  temperatureCold: '#4f8fcf',
  temperatureWarm: '#e3843b',
  weatherModifier: '#9085e9',
  opinionNegative: '#d95926',
  opinionPositive: '#199e70',
  intelCoverage: '#3987e5',
  heldTrait: '#5c8df6',
  gainedTrait: '#e5a43b',
} as const;

export type MeasureColour = keyof typeof MEASURE_COLOURS;

export function measureColour(name: MeasureColour, surface: string, minContrast = MARK_CONTRAST): string {
  return readableColour(MEASURE_COLOURS[name], surface, minContrast);
}

const FILL_CONTRAST = 1.5;
const COMMAND_TERM_COLOURS = ['#394d72', '#3b64a8', '#44587e', '#355c96'] as const;

export function commandTermColour(surface: string, index: number): string {
  return readableColour(COMMAND_TERM_COLOURS[index % COMMAND_TERM_COLOURS.length]!, surface, FILL_CONTRAST);
}

const CONVOY_EFFICIENCY_STOPS: readonly (readonly [number, string])[] = [
  [1, '#22364f'],
  [0.86, '#a8790f'],
  [0.62, '#d4661c'],
  [0.36, '#e2464e'],
  [0, '#ff6b74'],
];

export function convoyEfficiencyColour(share: number, surface: string): string {
  const value = Math.max(0, Math.min(1, share));
  for (let i = 1; i < CONVOY_EFFICIENCY_STOPS.length; i++) {
    const [upper, upperColour] = CONVOY_EFFICIENCY_STOPS[i - 1]!;
    const [lower, lowerColour] = CONVOY_EFFICIENCY_STOPS[i]!;
    if (value >= lower) return readableColour(mix(lowerColour, upperColour, upper === lower ? 1 : (value - lower) / (upper - lower)), surface, FILL_CONTRAST);
  }
  return readableColour(CONVOY_EFFICIENCY_STOPS[CONVOY_EFFICIENCY_STOPS.length - 1]![1], surface, FILL_CONTRAST);
}

const BLUEPRINT = { '--blueprint': '#1b3a5c', '--blueprint-border': '#2f5c8a' } as const;
const EVENT_CATEGORY_COLOURS = { '--event-politics': '#b49c6e', '--event-diplomacy': '#49b3d8', '--event-milestones': '#7cb675' } as const;

export function paletteVariables(surface: string): Readonly<Record<string, string>> {
  const blueprint = Object.entries(BLUEPRINT).map(([name, value]) => [name, readableColour(value, surface, FILL_CONTRAST)]);
  const events = Object.entries(EVENT_CATEGORY_COLOURS).map(([name, value]) => [name, readableColour(value, surface)]);
  return Object.fromEntries([...blueprint, ...events]);
}

const NEUTRAL_INK = { dark: '#f5f5f5', light: '#111111' } as const;

function neutralInk(surface: string): string {
  return NEUTRAL_INK[isDark(parseColor(surface) ?? { r: 0, g: 0, b: 0 }) ? 'dark' : 'light'];
}

export function surfaceGrey(surface: string, amount: number): string {
  return mix(surface, neutralInk(surface), amount);
}

const NEUTRAL_STEPS = { other: 0.197, fallback: 0.53, total: 1 } as const;

export function neutralColour(name: keyof typeof NEUTRAL_STEPS, surface: string): string {
  return surfaceGrey(surface, NEUTRAL_STEPS[name]);
}

export function tailColour(surface: string): string {
  return rgbaColor(neutralInk(surface), 0.22);
}

const GOLDEN_ANGLE = 137.50776405003785;
const LIGHTNESS_STEP = 0.7548776662466927;
const CHROMA_STEP = 0.5698402909980532;
const GENERATED_CONTRAST = 4.2;
const MAX_POOLS = 32;
const MAX_ORDINAL_SLOTS = 2048;

export type AuthoredColour = readonly [number, number, number] | Rgb | string;

export interface ColourOptions {
  readonly base?: readonly string[];
  readonly contrastSurfaces?: readonly string[];
  readonly avoid?: readonly string[];
}

interface GeneratedPool {
  readonly background: Rgb;
  readonly extras: readonly Rgb[];
  readonly darkSurface: boolean;
  readonly target: number;
  readonly base: readonly string[];
  readonly baseRgbs: readonly Rgb[];
  readonly colors: string[];
  readonly rgbs: Rgb[];
  readonly used: Set<string>;
  cursor: number;
}

const palettes = new Map<string, string[]>();
const pools = new Map<string, GeneratedPool>();
const readableBySurface = new Map<string, Map<string, string>>();
const MAX_READABLE_SURFACES = 8;
const MAX_READABLE_PER_SURFACE = 512;
let readableSolves = 0;

export const paletteCache = {
  get readableSolves() { return readableSolves; },
  get surfaces() { return readableBySurface.size; },
  clear(): void {
    readableBySurface.clear();
    readableSolves = 0;
  },
};

function touchMap<K, V>(map: Map<K, V>, key: K, value: V, limit: number): void {
  map.delete(key);
  map.set(key, value);
  while (map.size > limit) map.delete(map.keys().next().value!);
}

function roundRgb({ r, g, b }: Rgb): Rgb {
  return {
    r: Math.round(Math.min(255, Math.max(0, r))),
    g: Math.round(Math.min(255, Math.max(0, g))),
    b: Math.round(Math.min(255, Math.max(0, b))),
  };
}

const EXTRA_SURFACE_CONTRAST = MARK_CONTRAST + 0.05;

function meetsContrast(candidate: Rgb, background: Rgb, minContrast: number, extras: readonly Rgb[]): boolean {
  if (contrast(candidate, background) < minContrast) return false;
  for (let i = 0; i < extras.length; i++) {
    if (contrast(candidate, extras[i]!) < EXTRA_SURFACE_CONTRAST) return false;
  }
  return true;
}

function solveQuantised(color: Oklch, background: Rgb, darkSurface: boolean, minContrast: number, extras: readonly Rgb[]): Rgb {
  const initial = roundRgb(oklchToRgb(color));
  if (meetsContrast(initial, background, minContrast, extras)) return initial;
  let lo = color.l;
  let hi = darkSurface ? 1 : 0;
  let best = roundRgb(oklchToRgb({ ...color, l: hi }));
  for (let step = 0; step < 11; step++) {
    const mid = (lo + hi) / 2;
    const candidate = roundRgb(oklchToRgb({ ...color, l: mid }));
    if (meetsContrast(candidate, background, minContrast, extras)) {
      best = candidate;
      hi = mid;
    } else {
      lo = mid;
    }
  }
  return best;
}

function poolFor(surface: string, options?: ColourOptions): GeneratedPool {
  const base = options?.base ?? categoricalPalette(surface);
  const extraKey = options?.contrastSurfaces?.length ? options.contrastSurfaces.join(',') : '';
  const key = `${surface}\0${extraKey}\0${base.join(',')}`;
  const existing = pools.get(key);
  if (existing) {
    touchMap(pools, key, existing, MAX_POOLS);
    return existing;
  }
  const background = parseColor(surface) ?? { r: 0, g: 0, b: 0 };
  const darkSurface = isDark(background);
  const extreme: Rgb = darkSurface ? { r: 255, g: 255, b: 255 } : { r: 0, g: 0, b: 0 };
  const extras = (options?.contrastSurfaces ?? [])
    .map(hex => parseColor(hex))
    .filter((rgb): rgb is Rgb => rgb !== null);
  const maxContrast = contrast(extreme, background);
  const target = maxContrast >= 8 ? GENERATED_CONTRAST : MARK_CONTRAST;
  const used = new Set<string>();
  const resolvedBase: string[] = [];
  const baseRgbs: Rgb[] = [];
  for (const hex of base) {
    const parsed = parseColor(hex);
    if (parsed) {
      const checked = extras.length && !meetsContrast(parsed, background, MARK_CONTRAST, extras)
        ? solveQuantised(rgbToOklch(parsed), background, darkSurface, MARK_CONTRAST, extras)
        : parsed;
      const finalHex = toHex(checked);
      resolvedBase.push(finalHex);
      baseRgbs.push(checked);
      used.add(finalHex);
    } else {
      resolvedBase.push(hex);
      used.add(hex.toLowerCase());
    }
  }
  const pool: GeneratedPool = {
    background,
    extras,
    darkSurface,
    target,
    base: resolvedBase,
    baseRgbs,
    colors: [],
    rgbs: [],
    used,
    cursor: 0,
  };
  touchMap(pools, key, pool, MAX_POOLS);
  return pool;
}

function nextUniqueFallback(pool: GeneratedPool, baseColor: Oklch): { hex: string; rgb: Rgb } {
  for (let shift = 0; shift < 256; shift++) {
    const h = (baseColor.h + shift * 1.40625) % 360;
    const l = Math.min(0.98, Math.max(0.02, baseColor.l + ((shift % 9) - 4) * 0.015));
    const c = Math.min(0.28, Math.max(0.04, baseColor.c + ((shift % 5) - 2) * 0.015));
    const rgb = solveQuantised({ l, c, h }, pool.background, pool.darkSurface, MARK_CONTRAST, pool.extras);
    const hex = toHex(rgb);
    if (!pool.used.has(hex)) return { hex, rgb };
  }
  const rgb = solveQuantised(baseColor, pool.background, pool.darkSurface, MARK_CONTRAST, pool.extras);
  return { hex: toHex(rgb), rgb };
}

function generatedAt(pool: GeneratedPool, index: number): string {
  const limit = Math.max(1, MAX_ORDINAL_SLOTS - pool.base.length);
  const bounded = index >= limit ? index % limit : Math.max(0, index);
  const minBaseGap = pool.target > MARK_CONTRAST ? 3 : 0;
  const minNeighbourGap = pool.target > MARK_CONTRAST ? 12 : 0;
  while (pool.colors.length <= bounded) {
    const prev = pool.rgbs[pool.rgbs.length - 1];
    let accepted: { hex: string; rgb: Rgb } | null = null;
    let lastColor: Oklch = { l: pool.darkSurface ? 0.72 : 0.35, c: 0.14, h: 195 };
    for (let tries = 0; tries < 64; tries++) {
      const step = pool.cursor++;
      const h = (195 + step * GOLDEN_ANGLE) % 360;
      const u = (0.35 + step * LIGHTNESS_STEP) % 1;
      const v = (0.55 + step * CHROMA_STEP) % 1;
      const l = pool.darkSurface ? 0.62 + u * 0.27 : 0.22 + u * 0.30;
      const c = 0.08 + v * 0.14;
      lastColor = { l, c, h };
      const rgb = solveQuantised(lastColor, pool.background, pool.darkSurface, pool.target, pool.extras);
      const hex = toHex(rgb);
      if (pool.used.has(hex)) continue;
      if (minBaseGap > 0 && !pool.baseRgbs.every(other => colourDistance(rgb, other) >= minBaseGap)) continue;
      if (prev && minNeighbourGap > 0 && colourDistance(rgb, prev) < minNeighbourGap) continue;
      accepted = { hex, rgb };
      break;
    }
    if (accepted === null) accepted = nextUniqueFallback(pool, lastColor);
    pool.used.add(accepted.hex);
    pool.colors.push(accepted.hex);
    pool.rgbs.push(accepted.rgb);
  }
  return pool.colors[bounded]!;
}

export function authoredHex(color: AuthoredColour): string {
  if (typeof color === 'string') return color;
  return toHex('r' in color ? color : { r: color[0], g: color[1], b: color[2] });
}

function colourAtSlot(surface: string, slot: number, options?: ColourOptions): string {
  if (options?.contrastSurfaces?.length) {
    const pool = poolFor(surface, options);
    return slot < pool.base.length ? pool.base[slot]! : generatedAt(pool, slot - pool.base.length);
  }
  const base = options?.base ?? categoricalPalette(surface);
  if (slot < base.length) return base[slot]!;
  return generatedAt(poolFor(surface, options), slot - base.length);
}

export function colour(surface: string, ordinal: number, options?: ColourOptions): string {
  const raw = Number.isFinite(ordinal) ? Math.max(0, Math.floor(ordinal)) : 0;
  const slot = raw >= MAX_ORDINAL_SLOTS ? raw % MAX_ORDINAL_SLOTS : raw;
  const candidate = colourAtSlot(surface, slot, options);
  if (!options?.avoid?.length) return candidate;
  const avoid = new Set(options.avoid.map(c => c.toLowerCase()));
  if (!avoid.has(candidate.toLowerCase())) return candidate;
  const pool = poolFor(surface, options);
  const limit = Math.max(1, MAX_ORDINAL_SLOTS - pool.base.length);
  const start = Math.max(0, slot - pool.base.length) % limit;
  for (let tries = 0; tries < limit; tries++) {
    const next = generatedAt(pool, (start + tries) % limit);
    if (!avoid.has(next.toLowerCase())) return next;
  }
  return candidate;
}

const IDENTITY_COLOURS = {
  weather: { light_rain: '#3987e5', heavy_rain: '#9085e9', snow: '#9fc9e6', sandstorm: '#c98500' },
} as const satisfies Readonly<Record<string, Readonly<Record<string, string>>>>;

export function identityColour(surface: string, register: keyof typeof IDENTITY_COLOURS, token: string, ordinal: number): string {
  const known: Readonly<Record<string, string>> = IDENTITY_COLOURS[register];
  const authored = known[token];
  if (authored) return readableColour(authored, surface);
  return colour(surface, ordinal, { avoid: Object.values(known).map(hex => readableColour(hex, surface)) });
}
export function readableColour(color: AuthoredColour, surface: string, minContrast = MARK_CONTRAST): string {
  const colorKey = typeof color === 'string'
    ? color
    : 'r' in color
      ? `${color.r},${color.g},${color.b}`
      : `${color[0]},${color[1]},${color[2]}`;
  const entryKey = minContrast === MARK_CONTRAST ? colorKey : `${colorKey}\0${minContrast}`;
  let byColor = readableBySurface.get(surface);
  const cached = byColor?.get(entryKey);
  if (cached !== undefined) return cached;
  readableSolves++;
  const background = parseColor(surface) ?? { r: 0, g: 0, b: 0 };
  const rgb = typeof color === 'string'
    ? (parseColor(color) ?? { r: 128, g: 128, b: 128 })
    : 'r' in color ? color : { r: color[0], g: color[1], b: color[2] };
  const resolved = contrast(rgb, background) >= minContrast
    ? toHex(rgb)
    : toHex(solveQuantised(rgbToOklch(rgb), background, isDark(background), minContrast, []));
  if (!byColor) {
    byColor = new Map();
    touchMap(readableBySurface, surface, byColor, MAX_READABLE_SURFACES);
  }
  if (byColor.size >= MAX_READABLE_PER_SURFACE) byColor.clear();
  byColor.set(entryKey, resolved);
  return resolved;
}

export function categoricalPalette(surface: string): string[] {
  const existing = palettes.get(surface);
  if (existing) {
    touchMap(palettes, surface, existing, MAX_POOLS);
    return existing;
  }
  const background = parseColor(surface) ?? { r: 0, g: 0, b: 0 };
  const dark = isDark(background);
  const palette = CATEGORICAL[dark ? 'dark' : 'light'].map(hex => toHex(readable(parseColor(hex)!, background, dark)));
  touchMap(palettes, surface, palette, MAX_POOLS);
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

const RESERVED_STEP = 0.3;
const RESERVED_FAR_STEP = 0.14;
export const RESERVED_SEPARATION = 0.12;

export function reservedGrey(surface: string, identities: Iterable<string> = []): string {
  const near = surfaceGrey(surface, RESERVED_STEP);
  const nearLightness = rgbToOklch(parseColor(near)!).l;
  for (const identity of identities) {
    const rgb = parseColor(identity);
    if (!rgb) continue;
    const shown = rgbToOklch(rgb);
    if (shown.c < GREY_CHROMA && Math.abs(shown.l - nearLightness) < RESERVED_SEPARATION) return surfaceGrey(surface, RESERVED_FAR_STEP);
  }
  return near;
}

export function nudgeApart(colors: readonly Rgb[], background: Rgb, minDistance = 12, avoid: readonly Rgb[] = []): Rgb[] {
  const darkSurface = isDark(background);
  const surfaceHex = toHex(background);
  const result: Rgb[] = [];
  const neighbours = new NearestColours(avoid);
  for (const [position, color] of colors.entries()) {
    let base: Oklch = rgbToOklch(color);
    if (base.c < GREY_CHROMA) base = rgbToOklch(parseColor(colour(surfaceHex, position))!);
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

export const SECONDARY_DASH: readonly number[] = [6, 4];
