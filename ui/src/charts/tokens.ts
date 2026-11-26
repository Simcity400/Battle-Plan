import { APPEARANCE_EVENT } from '../shell/appearance/events.ts';
import { categoricalPalette } from './palette.ts';
import type { MotionFeel } from './easing.ts';

export const CHART_LAYOUT = { right: 14, countryEndLabels: 150, measureEndLabels: 128, stripMin: 40, stripInset: 50, hatchSpacing: 8, gapOpacity: 0.08, heatLabelWidth: 118 } as const;

export interface ChartTokens {
  surface: string; raised: string; text: string; muted: string; border: string;
  grid: string; axis: string; accent: string; positive: string; warning: string; error: string;
  categories: string[];
  font: string;
  fontSize: { small: number; body: number; title: number };
  lineWidth: { grid: number; series: number; emphasis: number };
  hatchSpacing: number; gapOpacity: number;
  motion: MotionFeel; reducedMotion: boolean; countryColours: 'raw' | 'fixed';
}
export type ChartPreferences = Pick<ChartTokens, 'motion' | 'reducedMotion' | 'countryColours'>;
const DEFAULT_PREFERENCES: ChartPreferences = { motion: 'smooth', reducedMotion: false, countryColours: 'raw' };

const VARIABLES = {
  surface: '--surface', raised: '--raised', text: '--text', muted: '--muted', border: '--border',
  grid: '--chart-grid', axis: '--chart-axis', accent: '--accent', positive: '--positive', warning: '--warning', error: '--error',
} as const;

const DEFAULT_SIZES: ChartTokens['fontSize'] = { small: 11, body: 12, title: 13 };

export function tokensFromValues(get: (variable: string) => string, font: string, sizes: ChartTokens['fontSize'] = DEFAULT_SIZES,
  preferences: ChartPreferences = DEFAULT_PREFERENCES): ChartTokens {
  const colors = Object.fromEntries(Object.entries(VARIABLES).map(([token, variable]) => [token, get(variable)])) as Record<keyof typeof VARIABLES, string>;
  return {
    ...colors, categories: categoricalPalette(colors.surface), font,
    fontSize: sizes,
    lineWidth: { grid: 1, series: 2, emphasis: 2.5 }, hatchSpacing: CHART_LAYOUT.hatchSpacing, gapOpacity: CHART_LAYOUT.gapOpacity, ...preferences,
  };
}

let cached: ChartTokens | null = null;
if (typeof window !== 'undefined') window.addEventListener(APPEARANCE_EVENT, () => { cached = null; });

export function readTokens(): ChartTokens {
  if (cached) return cached;
  const probe = document.createElement('span');
  probe.style.display = 'none';
  document.body.append(probe);
  const get = (variable: string) => { probe.style.color = `var(${variable})`; return getComputedStyle(probe).color; };
  const root = document.documentElement;
  const style = getComputedStyle(root);
  const font = style.getPropertyValue('--font-ui').trim() || 'sans-serif';
  const size = (variable: string, fallback: number) => Number.parseFloat(style.getPropertyValue(variable)) || fallback;
  const sizes = { small: size('--type-xs', DEFAULT_SIZES.small), body: size('--type-sm', DEFAULT_SIZES.body), title: size('--type-md', DEFAULT_SIZES.title) };
  const motion = root.dataset.chartMotion;
  cached = tokensFromValues(get, font, sizes, {
    motion: motion === 'snappy' || motion === 'cinematic' ? motion : 'smooth',
    reducedMotion: root.dataset.motion === 'reduced',
    countryColours: root.dataset.countryColours === 'fixed' ? 'fixed' : 'raw',
  });
  cached.hatchSpacing = size('--chart-hatch-spacing', CHART_LAYOUT.hatchSpacing);
  probe.remove();
  return cached;
}

export function subscribeTokens(callback: (tokens: ChartTokens) => void): () => void {
  const listener = () => callback(readTokens());
  window.addEventListener(APPEARANCE_EVENT, listener);
  return () => window.removeEventListener(APPEARANCE_EVENT, listener);
}
