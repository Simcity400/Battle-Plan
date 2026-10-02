import { APPEARANCE_EVENT } from '../shell/appearance/events.ts';
import { categoricalPalette } from './palette.ts';
import type { MotionFeel } from './easing.ts';

export { APPEARANCE_EVENT };

/** The theme values a canvas chart draws with. Charts never hard-code a colour,
 * font size or line width: they read these, so a theme or token change reaches
 * every chart at once. */
export interface ChartTokens {
  surface: string; raised: string; text: string; muted: string; border: string;
  grid: string; axis: string; accent: string; positive: string; warning: string; error: string;
  blueprint: string; blueprintBorder: string; blueprintText: string;
  /** Categorical colours for non-country categories, in fixed order. */
  categories: string[];
  font: string;
  fontSize: { small: number; body: number; title: number };
  lineWidth: { grid: number; series: number; emphasis: number };
  /** The app's chart settings and the system's reduced-motion choice. */
  motion: MotionFeel; reducedMotion: boolean; countryColours: 'raw' | 'fixed';
}
export type ChartPreferences = Pick<ChartTokens, 'motion' | 'reducedMotion' | 'countryColours'>;
const DEFAULT_PREFERENCES: ChartPreferences = { motion: 'smooth', reducedMotion: false, countryColours: 'raw' };

/** Theme variables the chart reads, by token. */
const VARIABLES = {
  surface: '--surface', raised: '--raised', text: '--text', muted: '--muted', border: '--border',
  grid: '--chart-grid', axis: '--chart-axis', accent: '--accent', positive: '--positive', warning: '--warning', error: '--error',
  blueprint: '--blueprint', blueprintBorder: '--blueprint-border', blueprintText: '--blueprint-text',
} as const;

const DEFAULT_SIZES: ChartTokens['fontSize'] = { small: 11, body: 12, title: 13 };

/** Builds tokens from resolved variable values; `get` returns a computed colour,
 * `sizes` are the app's type scale in pixels and `preferences` the chart settings. */
export function tokensFromValues(get: (variable: string) => string, font: string, sizes: ChartTokens['fontSize'] = DEFAULT_SIZES,
  preferences: ChartPreferences = DEFAULT_PREFERENCES): ChartTokens {
  const colors = Object.fromEntries(Object.entries(VARIABLES).map(([token, variable]) => [token, get(variable)])) as Record<keyof typeof VARIABLES, string>;
  return {
    ...colors, categories: categoricalPalette(colors.surface), font,
    fontSize: sizes,
    lineWidth: { grid: 1, series: 2, emphasis: 2.5 }, ...preferences,
  };
}

let cached: ChartTokens | null = null;
// Any appearance change drops the cache, even while no chart is mounted (on the
// Settings page), so charts mounted afterwards never draw with the old theme.
if (typeof window !== 'undefined') window.addEventListener(APPEARANCE_EVENT, () => { cached = null; });

/** Current tokens. Theme variables may be `color-mix()` expressions, which only the
 * browser can resolve, so each is read through a probe element's computed colour. */
export function readTokens(): ChartTokens {
  if (cached) return cached;
  const probe = document.createElement('span');
  probe.style.display = 'none';
  document.body.append(probe);
  const get = (variable: string) => { probe.style.color = `var(${variable})`; return getComputedStyle(probe).color; };
  const root = document.documentElement;
  const style = getComputedStyle(root);
  const font = style.getPropertyValue('--font-ui').trim() || 'sans-serif';
  // Chart text follows the app's type scale rather than sizes of its own.
  const size = (variable: string, fallback: number) => Number.parseFloat(style.getPropertyValue(variable)) || fallback;
  const sizes = { small: size('--type-xs', DEFAULT_SIZES.small), body: size('--type-sm', DEFAULT_SIZES.body), title: size('--type-md', DEFAULT_SIZES.title) };
  const motion = root.dataset.chartMotion;
  cached = tokensFromValues(get, font, sizes, {
    motion: motion === 'snappy' || motion === 'cinematic' ? motion : 'smooth',
    reducedMotion: root.dataset.motion === 'reduced',
    countryColours: root.dataset.countryColours === 'fixed' ? 'fixed' : 'raw',
  });
  probe.remove();
  return cached;
}

/** Calls back after every appearance change, with fresh tokens. */
export function subscribeTokens(callback: (tokens: ChartTokens) => void): () => void {
  // The module-level listener has already dropped the cache, so every chart shares one rebuild.
  const listener = () => callback(readTokens());
  window.addEventListener(APPEARANCE_EVENT, listener);
  return () => window.removeEventListener(APPEARANCE_EVENT, listener);
}
