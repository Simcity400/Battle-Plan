// Appearance preferences: theme choice per mode, the custom theme library,
// interface feel, typography and backdrop. Background images live in
// IndexedDB (backdrops.ts); only their IDs are stored here.
import { normalizeColor } from './color.ts';
import { isSidebarIconId } from './sidebarIcons.ts';
import type { SidebarIconColours } from './sidebarIcons.ts';
import { BUILT_IN_THEMES, DEFAULT_THEME_ID, completeColors, findTheme, themeColors, themeRoles } from './themes.ts';
import type { ThemeColors, ThemeDefinition, ThemeMode } from './themes.ts';
import { MAX_THEME_NAME } from './themeFile.ts';
import { DEFAULT_TYPOGRAPHY, parseTypography } from './typography.ts';
import type { FontFamilies, TypographySettings } from './typography.ts';

export const APPEARANCE_KEY = 'hoi4-replay.appearance.v2';
const LEGACY_KEY = 'hoi4-replay.appearance.v1';
export const MAX_CUSTOM_THEMES = 48;

export type ColorScheme = 'system' | ThemeMode;
export type MotionPreference = 'system' | 'full' | 'reduced';
export type BackdropKind = 'ambient' | 'solid' | 'image';
export interface Backdrop<Kind = BackdropKind> { kind: Kind; imageId: string | null; dim: number; blur: number }
/** How chart shapes glide between recorded samples. */
export type ChartMotion = 'snappy' | 'smooth' | 'cinematic';
/** Raw recorded map colours, or colours lightened or darkened to read on the theme and kept apart. */
export type CountryColours = 'raw' | 'fixed';
/** The top-bar flag at the content's large flag size, or enlarged to fill the bar with smoothing. */
export type HeaderFlag = 'native' | 'large';

export interface AppearanceSettings {
  version: 2;
  scheme: ColorScheme;
  themes: Record<ThemeMode, string>;
  custom: ThemeDefinition[];
  /** Text and border contrast, 50–100 % in steps of 5; 100 shows the theme as defined. */
  contrast: number;
  /** Opacity of the main view's panels, 40–100 %. 100 turns the frosted blur off. */
  glass: number;
  /** Opacity of the sidebar and top bar, 40–100 %, set apart from the main view's. */
  chromeGlass: number;
  /** Base corner radius in CSS pixels, 0–18. */
  radius: number;
  motion: MotionPreference;
  headerFlag: HeaderFlag;
  /** Font family names; empty uses the role's default stack. */
  fonts: FontFamilies;
  typography: TypographySettings;
  /** Behind the sidebar and top bar, and behind the main view while that uses the same. */
  backdrop: Backdrop;
  /** The main view's own backdrop; 'same' shows the sidebar and top bar's through it. */
  workspaceBackdrop: Backdrop<BackdropKind | 'same'>;
  charts: { motion: ChartMotion; countryColours: CountryColours };
  sidebarIcons: SidebarIconColours;
}

export const DEFAULT_APPEARANCE: AppearanceSettings = {
  version: 2, scheme: 'dark', themes: { dark: DEFAULT_THEME_ID, light: DEFAULT_THEME_ID }, custom: [],
  contrast: 100, glass: 78, chromeGlass: 82.4, radius: 8, motion: 'system', headerFlag: 'native', fonts: { interface: '', display: '', tables: '', diagnostics: '' },
  typography: structuredClone(DEFAULT_TYPOGRAPHY),
  backdrop: { kind: 'solid', imageId: null, dim: 55, blur: 0 },
  workspaceBackdrop: { kind: 'same', imageId: null, dim: 55, blur: 0 },
  charts: { motion: 'smooth', countryColours: 'raw' },
  sidebarIcons: {},
};

export const CONTRAST_STEP = 5;
export const LIMITS = { contrast: [50, 100], glass: [40, 100], chromeGlass: [40, 100], radius: [0, 18], dim: [0, 90], blur: [0, 40] } as const;

type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem'>;
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const id = (value: unknown): value is string => typeof value === 'string' && /^[a-zA-Z0-9_-]{1,64}$/.test(value);
const number = (value: unknown, [min, max]: readonly [number, number], fallback: number) =>
  typeof value === 'number' && Number.isFinite(value) ? Math.round(Math.min(max, Math.max(min, value))) : fallback;
/** As `number`, to a tenth: the sidebar and top bar glass a single glass setting gave is kept exactly. */
const tenth = (value: unknown, [min, max]: readonly [number, number], fallback: number) =>
  typeof value === 'number' && Number.isFinite(value) ? Math.round(Math.min(max, Math.max(min, value)) * 10) / 10 : fallback;
const fontName = (value: unknown) => typeof value === 'string' ? value.replace(/["'`;{}<>\\]/g, '').replace(/\s+/g, ' ').trim().slice(0, 200) : '';

function parseColors(input: unknown): ThemeColors | null {
  if (!object(input) || !themeRoles.every(role => normalizeColor(input[role]))) return null;
  return Object.fromEntries(themeRoles.map(role => [role, normalizeColor(input[role])!])) as ThemeColors;
}

export function parseCustomTheme(input: unknown): ThemeDefinition | null {
  if (!object(input) || !id(input.id) || findTheme(input.id, []) || typeof input.name !== 'string' || !object(input.variants)) return null;
  const name = input.name.trim().slice(0, MAX_THEME_NAME);
  const dark = parseColors(input.variants.dark), light = parseColors(input.variants.light);
  if (!name || (!dark && !light)) return null;
  return { id: input.id, name, variants: { ...(dark ? { dark } : {}), ...(light ? { light } : {}) } };
}

function parseBackdrop<Kind extends string>(input: unknown, kinds: readonly Kind[], fallback: Backdrop<Kind>): Backdrop<Kind> {
  const value = object(input) ? input : {};
  return {
    kind: kinds.includes(value.kind as Kind) ? value.kind as Kind : fallback.kind,
    imageId: id(value.imageId) ? value.imageId : null,
    dim: number(value.dim, LIMITS.dim, fallback.dim),
    blur: number(value.blur, LIMITS.blur, fallback.blur),
  };
}

/** Allowlist parse; anything unreadable falls back to its default. */
export function parseAppearance(input: unknown): AppearanceSettings {
  if (!object(input)) return structuredClone(DEFAULT_APPEARANCE);
  const custom: ThemeDefinition[] = [];
  for (const item of Array.isArray(input.custom) ? input.custom.slice(0, MAX_CUSTOM_THEMES) : []) {
    const theme = parseCustomTheme(item);
    if (theme && !custom.some(existing => existing.id === theme.id)) custom.push(theme);
  }
  const themes = object(input.themes) ? input.themes : {};
  const choose = (mode: ThemeMode) => id(themes[mode]) && findTheme(themes[mode] as string, custom) ? themes[mode] as string : DEFAULT_THEME_ID;
  const fonts = object(input.fonts) ? input.fonts : {};
  const charts = object(input.charts) ? input.charts : {};
  const sidebarIcons = object(input.sidebarIcons) ? input.sidebarIcons : {};
  const defaults = DEFAULT_APPEARANCE;
  const glass = number(input.glass, LIMITS.glass, defaults.glass);
  return {
    version: 2,
    sidebarIcons: Object.fromEntries(Object.entries(sidebarIcons).flatMap(([id, value]) => {
      const colour = isSidebarIconId(id) ? normalizeColor(value) : null;
      return colour ? [[id, colour]] : [];
    })),
    scheme: input.scheme === 'system' || input.scheme === 'light' || input.scheme === 'dark' ? input.scheme : defaults.scheme,
    themes: { dark: choose('dark'), light: choose('light') },
    custom,
    contrast: Math.round(number(input.contrast, LIMITS.contrast, defaults.contrast) / CONTRAST_STEP) * CONTRAST_STEP,
    glass,
    // Saved before the sidebar and top bar had their own setting, they keep the opacity they had.
    chromeGlass: tenth(input.chromeGlass, LIMITS.chromeGlass, input.glass === undefined ? defaults.chromeGlass : tenth(glass * 0.8 + 20, LIMITS.chromeGlass, defaults.chromeGlass)),
    radius: number(input.radius, LIMITS.radius, defaults.radius),
    motion: input.motion === 'full' || input.motion === 'reduced' || input.motion === 'system' ? input.motion : defaults.motion,
    headerFlag: input.headerFlag === 'native' || input.headerFlag === 'large' ? input.headerFlag : defaults.headerFlag,
    fonts: { interface: fontName(fonts.interface), display: fontName(fonts.display), tables: fontName(fonts.tables), diagnostics: fontName(fonts.diagnostics) },
    typography: parseTypography(input.typography, !!fontName(fonts.display)),
    backdrop: parseBackdrop(input.backdrop, ['ambient', 'solid', 'image'], defaults.backdrop),
    workspaceBackdrop: parseBackdrop(input.workspaceBackdrop, ['same', 'ambient', 'solid', 'image'], defaults.workspaceBackdrop),
    charts: {
      motion: charts.motion === 'snappy' || charts.motion === 'smooth' || charts.motion === 'cinematic' ? charts.motion : defaults.charts.motion,
      countryColours: charts.countryColours === 'raw' || charts.countryColours === 'fixed' ? charts.countryColours : defaults.charts.countryColours,
    },
  };
}

const legacyPresets = new Set(['atlas', 'graphite', 'field', 'paper']);

/** The first appearance format stored one flat palette. Built-in palettes map to
 * the new default; a customised palette becomes a custom theme. */
export function migrateLegacy(input: unknown): AppearanceSettings | null {
  if (!object(input) || !object(input.colors) || (input.mode !== 'dark' && input.mode !== 'light')) return null;
  const mode = input.mode;
  const settings = structuredClone(DEFAULT_APPEARANCE);
  settings.scheme = mode;
  if (typeof input.id === 'string' && legacyPresets.has(input.id)) return settings;
  const old = input.colors;
  const colors = completeColors({
    canvas: old.background, chrome: old.surface, surface: old.surface, raised: old.raised, overlay: old.surface, border: old.border,
    text: old.text, muted: old.muted, accent: old.accent, selection: old.selection, positive: old.positive, warning: old.warning, error: old.error,
  }, { canvas: mode === 'dark' ? '#0a0f16' : '#e9eef4', accent: '#5cb2ff' });
  settings.custom = [{ id: 'migrated-custom', name: 'My theme', variants: { [mode]: colors } }];
  settings.themes[mode] = 'migrated-custom';
  return settings;
}

export function loadAppearance(storage?: Storage): { settings: AppearanceSettings; warning: string | null } {
  try {
    const target = storage ?? localStorage;
    const raw = target.getItem(APPEARANCE_KEY);
    if (raw !== null) return { settings: parseAppearance(JSON.parse(raw)), warning: null };
    const legacy = target.getItem(LEGACY_KEY);
    if (legacy !== null) return { settings: migrateLegacy(JSON.parse(legacy)) ?? structuredClone(DEFAULT_APPEARANCE), warning: null };
  } catch {
    return { settings: structuredClone(DEFAULT_APPEARANCE), warning: 'Saved appearance could not be restored. The default theme is in use.' };
  }
  return { settings: structuredClone(DEFAULT_APPEARANCE), warning: null };
}

export function saveAppearance(settings: AppearanceSettings, storage?: Storage): string | null {
  try { (storage ?? localStorage).setItem(APPEARANCE_KEY, JSON.stringify(settings)); return null; }
  catch { return 'Appearance is applied for this session. This device could not save it.'; }
}

export interface ResolvedAppearance { mode: ThemeMode; theme: ThemeDefinition; colors: ThemeColors; reducedMotion: boolean }

export function resolveAppearance(settings: AppearanceSettings, systemDark: boolean, systemReducedMotion: boolean): ResolvedAppearance {
  const mode: ThemeMode = settings.scheme === 'system' ? (systemDark ? 'dark' : 'light') : settings.scheme;
  const theme = findTheme(settings.themes[mode], settings.custom) ?? BUILT_IN_THEMES[0];
  return { mode, theme, colors: themeColors(theme, mode), reducedMotion: settings.motion === 'reduced' || (settings.motion === 'system' && systemReducedMotion) };
}
