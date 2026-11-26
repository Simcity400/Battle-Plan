import { normalizeColor } from './color.ts';
import { DEFAULT_SEEDS, colorsMode, completeColors, themeRoles } from './themes.ts';
import type { ThemeColors, ThemeDefinition, ThemeMode } from './themes.ts';

export const THEME_FILE_FORMAT = 'battleplan-theme';
export const MAX_THEME_NAME = 48;
export const MAX_THEME_FILE_BYTES = 512 * 1024;

export interface ThemeFile {
  format: typeof THEME_FILE_FORMAT;
  version: 1;
  name: string;
  variants: Partial<Record<ThemeMode, ThemeColors>>;
}

export interface ImportedTheme { name: string; variants: Partial<Record<ThemeMode, ThemeColors>>; source: 'theme' | 'vscode'; notes: string[] }

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);

export function serializeTheme(theme: Pick<ThemeDefinition, 'name' | 'variants'>): string {
  const file: ThemeFile = { format: THEME_FILE_FORMAT, version: 1, name: theme.name, variants: {} };
  for (const mode of ['dark', 'light'] as const) {
    const colors = theme.variants[mode];
    if (colors) file.variants[mode] = Object.fromEntries(themeRoles.map(role => [role, colors[role]])) as ThemeColors;
  }
  return `${JSON.stringify(file, null, 2)}\n`;
}

export function parseJsonc(text: string): unknown {
  const output: string[] = [];
  let comma = -1;
  for (let index = 0; index < text.length; index++) {
    const char = text[index];
    if (char === '"') {
      const start = index;
      for (index++; index < text.length && text[index] !== '"'; index++) if (text[index] === '\\') index++;
      output.push(text.slice(start, index + 1));
      comma = -1;
    } else if (char === '/' && text[index + 1] === '/') {
      while (index < text.length && text[index] !== '\n') index++;
      output.push('\n');
    } else if (char === '/' && text[index + 1] === '*') {
      index += 2;
      while (index < text.length && !(text[index] === '*' && text[index + 1] === '/')) index++;
      index++;
    } else {
      if ((char === '}' || char === ']') && comma >= 0) output[comma] = '';
      if (char === ',') comma = output.length;
      else if (!/\s/.test(char)) comma = -1;
      output.push(char);
    }
  }
  return JSON.parse(output.join(''));
}

function cleanName(value: unknown, fallback: string): string {
  const name = typeof value === 'string' ? value.replace(/\s+/g, ' ').trim() : '';
  return (name || fallback).slice(0, MAX_THEME_NAME);
}

function variantFrom(input: unknown, mode: ThemeMode, notes: string[]): ThemeColors | null {
  if (!object(input)) return null;
  const unknown = Object.keys(input).filter(key => !(themeRoles as string[]).includes(key));
  if (unknown.length) notes.push(`Ignored unknown ${mode} colours: ${unknown.slice(0, 6).join(', ')}${unknown.length > 6 ? '…' : ''}`);
  const invalid = themeRoles.filter(role => role in input && !normalizeColor(input[role]));
  if (invalid.length) notes.push(`Replaced unreadable ${mode} colours: ${invalid.join(', ')}`);
  if (!normalizeColor(input.canvas) && !normalizeColor(input.accent)) return null;
  return completeColors(input, DEFAULT_SEEDS[mode]);
}

function parseOwnTheme(input: Record<string, unknown>): ImportedTheme {
  const notes: string[] = [];
  const variants: ImportedTheme['variants'] = {};
  if (object(input.variants)) {
    for (const mode of ['dark', 'light'] as const) {
      const colors = variantFrom(input.variants[mode], mode, notes);
      if (colors) variants[mode] = colors;
    }
  } else {
    const colors = object(input.colors) ? input.colors : input;
    const hinted = input.appearance === 'light' || input.mode === 'light' ? 'light' : input.appearance === 'dark' || input.mode === 'dark' ? 'dark' : null;
    const canvas = normalizeColor(colors.canvas);
    const mode = hinted ?? (canvas ? colorsMode({ canvas }) : 'dark');
    const variant = variantFrom(Object.fromEntries(Object.entries(colors).filter(([key]) => (themeRoles as string[]).includes(key))), mode, notes);
    if (variant) variants[mode] = variant;
  }
  if (!variants.dark && !variants.light) throw new Error('This theme has no readable canvas or accent colour.');
  return { name: cleanName(input.name, 'Imported theme'), variants, source: 'theme', notes };
}

function parseVsCodeTheme(input: Record<string, unknown>): ImportedTheme {
  const colors = input.colors as Record<string, unknown>;
  const pick = (...keys: string[]) => keys.map(key => colors[key]).find(value => normalizeColor(value));
  const background = normalizeColor(pick('editor.background', 'sideBar.background', 'panel.background'));
  if (!background) throw new Error('This VS Code theme doesn’t set an editor background.');
  const type = String(input.type ?? '').toLowerCase();
  const mode: ThemeMode = type.includes('light') ? 'light' : type.includes('dark') || type === 'hc' ? 'dark' : colorsMode({ canvas: background });
  const over = (value: unknown) => normalizeColor(value, background) ?? undefined;
  const accent = over(pick('button.background', 'focusBorder', 'activityBarBadge.background', 'textLink.foreground', 'progressBar.background'));
  const mapped: Record<string, string | undefined> = {
    canvas: background,
    accent,
    chrome: over(pick('sideBar.background', 'activityBar.background', 'titleBar.activeBackground')),
    surface: over(pick('editorWidget.background', 'panel.background', 'sideBarSectionHeader.background')),
    raised: over(pick('input.background', 'dropdown.background', 'button.secondaryBackground')),
    overlay: over(pick('menu.background', 'quickInput.background', 'editorSuggestWidget.background', 'editorWidget.background')),
    border: over(pick('panel.border', 'editorGroup.border', 'sideBar.border', 'contrastBorder', 'widget.border')),
    text: over(pick('foreground', 'editor.foreground', 'sideBar.foreground')),
    muted: over(pick('descriptionForeground', 'tab.inactiveForeground', 'editorLineNumber.foreground')),
    accentText: over(pick('button.foreground', 'activityBarBadge.foreground')),
    glow: over(pick('textLink.foreground', 'terminal.ansiCyan', 'terminal.ansiMagenta')),
    positive: over(pick('terminal.ansiGreen', 'gitDecoration.addedResourceForeground', 'testing.iconPassed')),
    warning: over(pick('editorWarning.foreground', 'list.warningForeground', 'terminal.ansiYellow')),
    error: over(pick('editorError.foreground', 'errorForeground', 'terminal.ansiRed')),
  };
  if (mapped.accent) mapped.selection = mapped.accent;
  const variant = completeColors(Object.fromEntries(Object.entries(mapped).filter(([, value]) => value)), { canvas: background, accent: accent ?? DEFAULT_SEEDS[mode].accent });
  const notes = ['Converted from a VS Code theme. Unset colours were derived from its background and accent.'];
  return { name: cleanName(input.name, 'VS Code theme'), variants: { [mode]: variant }, source: 'vscode', notes };
}

export function parseThemeText(text: string): ImportedTheme {
  if (text.length > MAX_THEME_FILE_BYTES) throw new Error('Theme files must be smaller than 512 KB.');
  let input: unknown;
  try { input = parseJsonc(text); } catch { throw new Error('This isn’t a valid theme file. Themes are JSON.'); }
  if (!object(input)) throw new Error('This isn’t a valid theme file.');
  if (object(input.colors) && Object.keys(input.colors).some(key => key.includes('.'))) return parseVsCodeTheme(input);
  if (input.format !== undefined && input.format !== THEME_FILE_FORMAT) throw new Error('This file isn’t a BattlePlan theme.');
  if (input.version !== undefined && input.version !== 1) throw new Error(`Theme file version ${String(input.version)} isn’t supported.`);
  return parseOwnTheme(input);
}

export function uniqueThemeName(name: string, taken: readonly string[], suffix = ''): string {
  const lower = new Set(taken.map(item => item.trim().toLowerCase()));
  const base = name.trim();
  for (let index = 1; ; index++) {
    const tail = index === 1 ? suffix : `${suffix} ${index}`;
    const candidate = `${base.slice(0, MAX_THEME_NAME - tail.length).trimEnd()}${tail}`;
    if (!lower.has(candidate.toLowerCase())) return candidate;
  }
}
