import { contrast, isDark, mixOklab, normalizeColor, oklchHex, oklchToRgb, parseColor, readableText, rgbToOklch, solveLightness, toHex, toHexAlpha } from './color.ts';

export type ThemeMode = 'dark' | 'light';

/** Every colour a theme can set. CSS reads them as `--kebab-case` variables. */
export const THEME_ROLES = {
  canvas: { label: 'Canvas', group: 'Foundation', help: 'The backdrop behind every panel.' },
  chrome: { label: 'Sidebar and top bar', group: 'Foundation', help: 'Navigation and playback chrome.' },
  surface: { label: 'Panels', group: 'Foundation', help: 'Widgets, cards and settings sections.' },
  raised: { label: 'Controls', group: 'Foundation', help: 'Buttons, fields and hover states.' },
  overlay: { label: 'Menus', group: 'Foundation', help: 'Pop-downs, menus and tooltips.' },
  border: { label: 'Borders', group: 'Foundation', help: 'Outlines and dividers.' },
  text: { label: 'Text', group: 'Text', help: 'Primary text and icons.' },
  muted: { label: 'Secondary text', group: 'Text', help: 'Labels, hints and metadata.' },
  accent: { label: 'Accent', group: 'Accent', help: 'Primary actions, focus and progress.' },
  accentText: { label: 'Text on accent', group: 'Accent', help: 'Labels drawn on the accent colour.' },
  selection: { label: 'Selection', group: 'Accent', help: 'Selected navigation and list items.' },
  glow: { label: 'Ambient glow', group: 'Accent', help: 'The soft light in the ambient backdrop.' },
  positive: { label: 'Positive', group: 'Status', help: 'Loaded, faster and success states.' },
  warning: { label: 'Warning', group: 'Status', help: 'Notices and slower rates.' },
  error: { label: 'Error', group: 'Status', help: 'Failures and recording gaps.' },
} as const;

export type ThemeRole = keyof typeof THEME_ROLES;
export type ThemeColors = Record<ThemeRole, string>;
export const themeRoles = Object.keys(THEME_ROLES) as ThemeRole[];
export const cssVariable = (role: ThemeRole) => `--${role.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`)}`;

export interface ThemeDefinition {
  id: string;
  name: string;
  builtIn?: boolean;
  /** At least one variant; a missing variant falls back to the default theme for that mode. */
  variants: Partial<Record<ThemeMode, ThemeColors>>;
}

export interface ThemeSeeds { canvas: string; accent: string; glow?: string }

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/** Builds a complete, readable palette from a canvas and an accent (and optionally a glow). */
export function deriveColors(seeds: ThemeSeeds): ThemeColors {
  const canvasRgb = parseColor(seeds.canvas) ?? { r: 11, g: 16, b: 23 };
  const accentRgb = parseColor(seeds.accent) ?? { r: 95, g: 179, b: 255 };
  // The ramp follows the canvas actually chosen, not the slot it is saved in.
  const dark = isDark(canvasRgb);
  const canvas = rgbToOklch(canvasRgb), accent = rgbToOklch(accentRgb);
  const hue = canvas.c > 0.012 ? canvas.h : accent.h;
  const tint = clamp(Math.max(canvas.c, accent.c * 0.16), 0.004, 0.035);
  const shade = (lightness: number, chroma = tint) => oklchHex({ l: clamp(lightness, 0.03, 0.992), c: chroma, h: hue });
  const surfaceL = dark ? canvas.l + 0.04 : canvas.l < 0.95 ? canvas.l + (0.995 - canvas.l) * 0.78 : canvas.l - 0.015;
  const surface = shade(surfaceL, dark ? tint : tint * 0.45);
  const surfaceRgb = parseColor(surface)!;
  const textStart = { l: dark ? 0.96 : 0.2, c: Math.min(0.02, accent.c * 0.12), h: accent.h };
  const text = oklchToRgb(solveLightness(textStart, canvasRgb, 8, dark ? 'lighter' : 'darker'));
  const muted = oklchToRgb(solveLightness({ ...textStart, l: dark ? 0.7 : 0.5 }, surfaceRgb, 4.6, dark ? 'lighter' : 'darker'));
  const glow = normalizeColor(seeds.glow) ?? oklchHex({ l: accent.l, c: Math.max(accent.c, 0.05), h: (accent.h + 35) % 360 });
  const accentHex = normalizeColor(seeds.accent) ?? '#5fb3ff';
  return {
    canvas: normalizeColor(seeds.canvas) ?? '#0b1017',
    chrome: shade(dark ? canvas.l + 0.022 : canvas.l + (0.995 - canvas.l) * 0.5, tint * (dark ? 1.1 : 0.6)),
    surface,
    raised: shade(dark ? canvas.l + 0.09 : surfaceL - 0.035, tint * 1.1),
    overlay: shade(dark ? canvas.l + 0.065 : 0.992, tint),
    border: shade(dark ? canvas.l + 0.15 : canvas.l - 0.11, tint * 1.4),
    text: toHex(text),
    muted: toHex(muted),
    accent: accentHex,
    accentText: readableText(accentRgb, dark ? oklchHex({ l: 0.16, c: tint, h: hue }) : '#0b0f14'),
    selection: accentHex,
    glow,
    positive: oklchHex({ l: dark ? 0.76 : 0.52, c: 0.13, h: 158 }),
    warning: oklchHex({ l: dark ? 0.82 : 0.6, c: 0.13, h: 78 }),
    error: oklchHex({ l: dark ? 0.7 : 0.54, c: 0.17, h: 22 }),
  };
}

/** Applies user-supplied roles over a derived palette; invalid values are ignored. */
export function completeColors(partial: Partial<Record<string, unknown>>, fallback: ThemeSeeds): ThemeColors {
  const seeds = {
    canvas: normalizeColor(partial.canvas) ?? fallback.canvas,
    accent: normalizeColor(partial.accent) ?? fallback.accent,
    glow: normalizeColor(partial.glow) ?? undefined,
  };
  const base = deriveColors(seeds);
  for (const role of themeRoles) {
    const value = normalizeColor(partial[role], base.canvas);
    if (value) base[role] = value;
  }
  return base;
}

export function colorsMode(colors: Pick<ThemeColors, 'canvas'>): ThemeMode {
  return isDark(parseColor(colors.canvas) ?? { r: 0, g: 0, b: 0 }) ? 'dark' : 'light';
}

/** A short list of contrast problems the editor can surface. */
export function contrastWarnings(colors: ThemeColors): string[] {
  const rgb = (role: ThemeRole) => parseColor(colors[role])!;
  const checks: [ThemeRole, ThemeRole, number, string][] = [
    ['text', 'surface', 4.5, 'Text is hard to read on panels'],
    ['muted', 'surface', 3, 'Secondary text is hard to read on panels'],
    ['accentText', 'accent', 3, 'Text on accent buttons is hard to read'],
    ['text', 'canvas', 4.5, 'Text is hard to read on the canvas'],
  ];
  return checks.filter(([a, b, ratio]) => contrast(rgb(a), rgb(b)) < ratio).map(([, , , message]) => message);
}

const theme = (id: string, name: string, dark: ThemeSeeds, light: ThemeSeeds, overrides: Partial<Record<ThemeMode, Partial<ThemeColors>>> = {}): ThemeDefinition => ({
  id, name, builtIn: true,
  variants: { dark: { ...deriveColors(dark), ...overrides.dark }, light: { ...deriveColors(light), ...overrides.light } },
});

export const DEFAULT_THEME_ID = 'carbon';

/** Neutral greys with one blue accent, after T3 Code's default palette. */
const carbon: ThemeDefinition = {
  id: 'carbon', name: 'Carbon', builtIn: true,
  variants: {
    dark: {
      canvas: '#0a0a0a', chrome: '#000000', surface: '#111111', raised: '#1e1e1e', overlay: '#111111', border: '#1f1f1f',
      text: '#f5f5f5', muted: '#8a8a8a', accent: '#346bf1', accentText: '#ffffff', selection: '#f5f5f5', glow: '#346bf1',
      positive: '#22c55e', warning: '#fe9a00', error: '#fb414a',
    },
    light: {
      canvas: '#fcfcfc', chrome: '#fafafa', surface: '#ffffff', raised: '#f4f4f5', overlay: '#ffffff', border: '#e4e4e7',
      text: '#27272a', muted: '#71717b', accent: '#1b4ed8', accentText: '#ffffff', selection: '#27272a', glow: '#1b4ed8',
      positive: '#15803d', warning: '#b45309', error: '#dc2626',
    },
  },
};

export const BUILT_IN_THEMES: readonly ThemeDefinition[] = [
  carbon,
  theme('signal', 'Signal', { canvas: '#0c0d10', accent: '#5cb2ff', glow: '#2fc6c0' }, { canvas: '#f2f3f5', accent: '#1f6fd1', glow: '#3fb6d9' }),
  theme('brass', 'Brass', { canvas: '#100e0b', accent: '#e0aa4f', glow: '#c2582d' }, { canvas: '#f1ebdf', accent: '#95650f', glow: '#d98a3d' }),
  theme('iris', 'Iris', { canvas: '#0d0d14', accent: '#8f82ff', glow: '#d06bf5' }, { canvas: '#efeef7', accent: '#5a47e0', glow: '#b46bf0' }),
  theme('field', 'Field', { canvas: '#0e120e', accent: '#aac77e', glow: '#5f8e55' }, { canvas: '#ecede3', accent: '#4e6a28', glow: '#8fb361' }),
  theme('ember', 'Ember', { canvas: '#120c0b', accent: '#ff7452', glow: '#ffae42' }, { canvas: '#f6eeea', accent: '#c2410c', glow: '#f59e0b' }),
  theme('graphite', 'Graphite', { canvas: '#0e0f11', accent: '#e8eaee', glow: '#6b7280' }, { canvas: '#f2f2f3', accent: '#1b1c20', glow: '#9ca3af' }),
];

export function findTheme(id: string, custom: readonly ThemeDefinition[]): ThemeDefinition | undefined {
  return BUILT_IN_THEMES.find(item => item.id === id) ?? custom.find(item => item.id === id);
}

/** The colours shown for `mode`, falling back to the default theme's variant. */
export function themeColors(theme: ThemeDefinition | undefined, mode: ThemeMode): ThemeColors {
  return theme?.variants[mode] ?? BUILT_IN_THEMES[0].variants[mode]!;
}

/** Text and secondary text fade toward the canvas and borders toward transparent below 100%.
 * At 100% the theme is unchanged. Surfaces keep their colours; derived roles follow. */
export function contrastColors(colors: ThemeColors, percent: number): ThemeColors {
  if (percent >= 100) return colors;
  const amount = Math.max(0, percent) / 100;
  const canvas = parseColor(colors.canvas)!;
  const ink = (role: 'text' | 'muted') => toHex(mixOklab(canvas, parseColor(colors[role])!, amount));
  return { ...colors, text: ink('text'), muted: ink('muted'), border: toHexAlpha(parseColor(colors.border)!, amount) };
}
