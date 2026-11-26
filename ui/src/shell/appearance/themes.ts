import { contrast, extendColor, isDark, mix, mixOklab, mixRgb, normalizeColor, oklchHex, oklchToRgb, parseColor, readableText, rgbToOklch, solveLightness, toHex, toHexAlpha } from './color.ts';

export type ThemeMode = 'dark' | 'light';

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
  variants: Partial<Record<ThemeMode, ThemeColors>>;
}

export interface ThemeSeeds { canvas: string; accent: string; glow?: string }

export const DEFAULT_SEEDS: Readonly<Record<ThemeMode, ThemeSeeds>> = {
  dark: { canvas: '#0a0f16', accent: '#5cb2ff' },
  light: { canvas: '#e9eef4', accent: '#1f6fd1' },
};

export const THEME_JSON_EXAMPLE = '{ "name": "Dusk", "canvas": "#14121b", "accent": "#f2a65a" }';

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

export function deriveColors(seeds: ThemeSeeds): ThemeColors {
  const canvasRgb = parseColor(seeds.canvas) ?? parseColor(DEFAULT_SEEDS.dark.canvas)!;
  const accentRgb = parseColor(seeds.accent) ?? parseColor(DEFAULT_SEEDS.dark.accent)!;
  const dark = isDark(canvasRgb);
  const canvas = rgbToOklch(canvasRgb), accent = rgbToOklch(accentRgb);
  const hue = canvas.c > 0.012 ? canvas.h : accent.h;
  const tint = clamp(Math.max(canvas.c, accent.c * 0.16), 0.004, 0.035);
  const shade = (lightness: number, chroma = tint) => oklchHex({ l: clamp(lightness, 0.03, 0.992), c: chroma, h: hue });
  const surfaceL = dark ? canvas.l + 0.04 : canvas.l < 0.95 ? canvas.l + (0.995 - canvas.l) * 0.78 : canvas.l - 0.015;
  const surface = shade(surfaceL, dark ? tint : tint * 0.45);
  const chrome = shade(dark ? canvas.l + 0.022 : canvas.l + (0.995 - canvas.l) * 0.5, tint * (dark ? 1.1 : 0.6));
  const raised = shade(dark ? canvas.l + 0.09 : surfaceL - 0.035, tint * 1.1);
  const overlay = shade(dark ? canvas.l + 0.065 : 0.992, tint);
  const textStart = { l: dark ? 0.96 : 0.2, c: Math.min(0.02, accent.c * 0.12), h: accent.h };
  const text = oklchToRgb(solveLightness(textStart, canvasRgb, 8, dark ? 'lighter' : 'darker'));
  const fills = [canvasRgb, ...[chrome, surface, overlay].map(parseColor)]
    .map(fill => mixRgb(fill!, text, dark ? 0.06 : 0.05)).concat(parseColor(raised)!);
  const muted = oklchToRgb(fills.reduce((solved, fill) => solveLightness(solved, fill, 4.6, dark ? 'lighter' : 'darker'), { ...textStart, l: dark ? 0.7 : 0.5 }));
  const glow = normalizeColor(seeds.glow) ?? oklchHex({ l: accent.l, c: Math.max(accent.c, 0.05), h: (accent.h + 35) % 360 });
  const accentHex = toHex(accentRgb);
  return {
    canvas: toHex(canvasRgb),
    chrome,
    surface,
    raised,
    overlay,
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

export const seedsOf = (colors: ThemeColors): ThemeSeeds => ({ canvas: colors.canvas, accent: colors.accent, glow: colors.glow });

export function customRoles(colors: ThemeColors): ThemeRole[] {
  const derived = deriveColors(seedsOf(colors));
  return themeRoles.filter(role => normalizeColor(colors[role]) !== derived[role]);
}

export function reseed(colors: ThemeColors, seed: keyof ThemeSeeds, value: string, kept: readonly ThemeRole[]): ThemeColors {
  const next = normalizeColor(value);
  if (!next || next === normalizeColor(colors[seed])) return colors;
  const derived = deriveColors({ ...seedsOf(colors), [seed]: next });
  for (const role of kept) if (role !== seed) derived[role] = colors[role];
  return derived;
}

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
  const canvas = parseColor(colors.canvas);
  return !canvas || isDark(canvas) ? 'dark' : 'light';
}

function inkEnds(colors: Pick<ThemeColors, 'surface' | 'text'>): { readonly light: string; readonly dark: string } {
  const textSide = extendColor(colors.surface, colors.text), surfaceSide = extendColor(colors.text, colors.surface);
  return colorsMode({ canvas: colors.surface }) === 'dark' ? { light: textSide, dark: surfaceSide } : { light: surfaceSide, dark: textSide };
}

export function themeVariables(colors: ThemeColors): Record<string, string> {
  const ink = inkEnds(colors), accentText = parseColor(colors.accentText)!;
  const opposite = toHex({ r: 255 - accentText.r, g: 255 - accentText.g, b: 255 - accentText.b });
  return { '--ink-light': ink.light, '--ink-dark': ink.dark, '--accent-hover': mix(colors.accent, opposite, 0.14) };
}

export function contrastWarnings(colors: ThemeColors): string[] {
  const rgb = (role: ThemeRole) => parseColor(colors[role])!;
  const checks: [ThemeRole, ThemeRole, number, string][] = [
    ['text', 'surface', 4.5, 'Text is hard to read on panels'],
    ['muted', 'surface', 4.5, 'Secondary text is hard to read on panels'],
    ['accentText', 'accent', 4.5, 'Text on accent buttons is hard to read'],
    ['text', 'canvas', 4.5, 'Text is hard to read on the canvas'],
  ];
  return checks.filter(([a, b, ratio]) => contrast(rgb(a), rgb(b)) < ratio).map(([, , , message]) => message);
}

const theme = (id: string, name: string, dark: ThemeSeeds, light: ThemeSeeds, overrides: Partial<Record<ThemeMode, Partial<ThemeColors>>> = {}): ThemeDefinition => ({
  id, name, builtIn: true,
  variants: { dark: { ...deriveColors(dark), ...overrides.dark }, light: { ...deriveColors(light), ...overrides.light } },
});

export const DEFAULT_THEME_ID = 'carbon';

const carbon: ThemeDefinition = {
  id: 'carbon', name: 'Carbon', builtIn: true,
  variants: {
    dark: {
      canvas: '#0a0a0a', chrome: '#000000', surface: '#111111', raised: '#1e1e1e', overlay: '#191919', border: '#1f1f1f',
      text: '#f5f5f5', muted: '#929292', accent: '#346bf1', accentText: '#ffffff', selection: '#f5f5f5', glow: '#346bf1',
      positive: '#22c55e', warning: '#fe9a00', error: '#fb414a',
    },
    light: {
      canvas: '#fcfcfc', chrome: '#fafafa', surface: '#ffffff', raised: '#f4f4f5', overlay: '#ffffff', border: '#e4e4e7',
      text: '#27272a', muted: '#6a6a74', accent: '#1b4ed8', accentText: '#ffffff', selection: '#27272a', glow: '#1b4ed8',
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

export function themeColors(theme: ThemeDefinition | undefined, mode: ThemeMode): ThemeColors {
  return theme?.variants[mode] ?? BUILT_IN_THEMES[0].variants[mode]!;
}

export function contrastColors(colors: ThemeColors, percent: number): ThemeColors {
  if (percent >= 100) return colors;
  const amount = Math.max(0, percent) / 100;
  const canvas = parseColor(colors.canvas)!;
  const direction = isDark(canvas) ? 'lighter' : 'darker';
  const text = mixOklab(canvas, parseColor(colors.text)!, amount);
  const fills = (['canvas', 'chrome', 'surface', 'overlay'] as const).map(role => mixRgb(parseColor(colors[role])!, text, direction === 'lighter' ? 0.06 : 0.05))
    .concat(parseColor(colors.raised)!);
  const ink = (role: 'text' | 'muted') => {
    const full = parseColor(colors[role])!;
    const faded = fills.reduce((solved, fill) => solveLightness(solved, fill, Math.min(4.6, contrast(full, fill)), direction),
      rgbToOklch(role === 'text' ? text : mixOklab(canvas, full, amount)));
    const rgb = oklchToRgb(faded);
    return toHex(contrast(rgb, canvas) > contrast(full, canvas) ? full : rgb);
  };
  return { ...colors, text: ink('text'), muted: ink('muted'), border: toHexAlpha(parseColor(colors.border)!, amount) };
}
