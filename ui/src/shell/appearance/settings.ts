import { normalizeColor } from './color.ts';
import { isWidgetKind, parseIconStyles } from './iconStyle.ts';
import type { WidgetIconStyles } from './iconStyle.ts';
import { isSidebarIconId } from './sidebarIcons.ts';
import type { SidebarIconStyles } from './sidebarIcons.ts';
import { BUILT_IN_THEMES, DEFAULT_THEME_ID, findTheme, themeColors, themeRoles } from './themes.ts';
import type { ThemeColors, ThemeDefinition, ThemeMode } from './themes.ts';
import { MAX_THEME_NAME } from './themeFile.ts';
import { DEFAULT_TYPOGRAPHY, parseTypography } from './typography.ts';
import type { FontFamilies, TypographySettings } from './typography.ts';

export const APPEARANCE_KEY = 'battleplan.appearance.v2';
export const MAX_CUSTOM_THEMES = 48;

export type ColorScheme = 'system' | ThemeMode;
export type MotionPreference = 'system' | 'full' | 'reduced';
export type BackdropKind = 'ambient' | 'solid' | 'image';
export interface Backdrop<Kind = BackdropKind> { kind: Kind; imageId: string | null; dim: number; blur: number }
export type PlaybackFrameRate = '30' | '60' | '120' | 'uncapped';
export type ChartMotion = 'snappy' | 'smooth' | 'cinematic';
export type CountryColours = 'raw' | 'fixed';
export type HeaderFlag = 'native' | 'large';

export interface AppearanceSettings {
  version: 2;
  scheme: ColorScheme;
  themes: Record<ThemeMode, string>;
  custom: ThemeDefinition[];
  contrast: number;
  glass: number;
  chromeGlass: number;
  radius: number;
  motion: MotionPreference;
  headerFlag: HeaderFlag;
  fonts: FontFamilies;
  typography: TypographySettings;
  backdrop: Backdrop;
  workspaceBackdrop: Backdrop<BackdropKind | 'same'>;
  charts: { frameRate: PlaybackFrameRate; motion: ChartMotion; countryColours: CountryColours };
  sidebarIcons: SidebarIconStyles;
  widgetIcons: WidgetIconStyles;
}

export const DEFAULT_APPEARANCE: AppearanceSettings = {
  version: 2, scheme: 'dark', themes: { dark: DEFAULT_THEME_ID, light: DEFAULT_THEME_ID }, custom: [],
  contrast: 100, glass: 78, chromeGlass: 82.4, radius: 8, motion: 'system', headerFlag: 'native', fonts: { interface: '', display: '', tables: '', diagnostics: '' },
  typography: structuredClone(DEFAULT_TYPOGRAPHY),
  backdrop: { kind: 'solid', imageId: null, dim: 55, blur: 0 },
  workspaceBackdrop: { kind: 'same', imageId: null, dim: 55, blur: 0 },
  charts: { frameRate: '60', motion: 'smooth', countryColours: 'raw' },
  sidebarIcons: {},
  widgetIcons: {},
};

export const CONTRAST_STEP = 5;
export const LIMITS = { contrast: [50, 100], glass: [40, 100], chromeGlass: [40, 100], radius: [0, 18], dim: [0, 90], blur: [0, 40] } as const;

type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem'>;
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const id = (value: unknown): value is string => typeof value === 'string' && /^[a-zA-Z0-9_-]{1,64}$/.test(value);
const number = (value: unknown, [min, max]: readonly [number, number], fallback: number) =>
  typeof value === 'number' && Number.isFinite(value) ? Math.round(Math.min(max, Math.max(min, value))) : fallback;
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
  const defaults = DEFAULT_APPEARANCE;
  const glass = number(input.glass, LIMITS.glass, defaults.glass);
  return {
    version: 2,
    sidebarIcons: parseIconStyles(input.sidebarIcons, isSidebarIconId),
    widgetIcons: parseIconStyles(input.widgetIcons, isWidgetKind),
    scheme: input.scheme === 'system' || input.scheme === 'light' || input.scheme === 'dark' ? input.scheme : defaults.scheme,
    themes: { dark: choose('dark'), light: choose('light') },
    custom,
    contrast: Math.round(number(input.contrast, LIMITS.contrast, defaults.contrast) / CONTRAST_STEP) * CONTRAST_STEP,
    glass,
    chromeGlass: tenth(input.chromeGlass, LIMITS.chromeGlass, input.glass === undefined ? defaults.chromeGlass : tenth(glass * 0.8 + 20, LIMITS.chromeGlass, defaults.chromeGlass)),
    radius: number(input.radius, LIMITS.radius, defaults.radius),
    motion: input.motion === 'full' || input.motion === 'reduced' || input.motion === 'system' ? input.motion : defaults.motion,
    headerFlag: input.headerFlag === 'native' || input.headerFlag === 'large' ? input.headerFlag : defaults.headerFlag,
    fonts: { interface: fontName(fonts.interface), display: fontName(fonts.display), tables: fontName(fonts.tables), diagnostics: fontName(fonts.diagnostics) },
    typography: parseTypography(input.typography, !!fontName(fonts.display)),
    backdrop: parseBackdrop(input.backdrop, ['ambient', 'solid', 'image'], defaults.backdrop),
    workspaceBackdrop: parseBackdrop(input.workspaceBackdrop, ['same', 'ambient', 'solid', 'image'], defaults.workspaceBackdrop),
    charts: {
      frameRate: charts.frameRate === '30' || charts.frameRate === '120' || charts.frameRate === 'uncapped' ? charts.frameRate : '60',
      motion: charts.motion === 'snappy' || charts.motion === 'smooth' || charts.motion === 'cinematic' ? charts.motion : defaults.charts.motion,
      countryColours: charts.countryColours === 'raw' || charts.countryColours === 'fixed' ? charts.countryColours : defaults.charts.countryColours,
    },
  };
}

export function loadAppearance(storage?: Storage): { settings: AppearanceSettings; warning: string | null } {
  try {
    const target = storage ?? localStorage;
    const raw = target.getItem(APPEARANCE_KEY);
    if (raw !== null) return { settings: parseAppearance(JSON.parse(raw)), warning: null };
  } catch {
    return { settings: structuredClone(DEFAULT_APPEARANCE), warning: 'Saved appearance couldn’t be restored. The default theme is in use.' };
  }
  return { settings: structuredClone(DEFAULT_APPEARANCE), warning: null };
}

export function saveAppearance(settings: AppearanceSettings, storage?: Storage): string | null {
  try { (storage ?? localStorage).setItem(APPEARANCE_KEY, JSON.stringify(settings)); return null; }
  catch { return 'Appearance is applied for this session. This device couldn’t save it.'; }
}

export interface ResolvedAppearance { mode: ThemeMode; theme: ThemeDefinition; colors: ThemeColors; reducedMotion: boolean }

export function resolveAppearance(settings: AppearanceSettings, systemDark: boolean, systemReducedMotion: boolean): ResolvedAppearance {
  const mode: ThemeMode = settings.scheme === 'system' ? (systemDark ? 'dark' : 'light') : settings.scheme;
  const theme = findTheme(settings.themes[mode], settings.custom) ?? BUILT_IN_THEMES[0];
  return { mode, theme, colors: themeColors(theme, mode), reducedMotion: settings.motion === 'reduced' || (settings.motion === 'system' && systemReducedMotion) };
}
