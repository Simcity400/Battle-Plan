import { useLayoutEffect, useState } from 'react';

export const colorLabels = {
  background: 'Page background', surface: 'Panels', raised: 'Raised surfaces', border: 'Borders',
  text: 'Primary text', muted: 'Secondary text', accent: 'Highlight', selection: 'Selected navigation',
  positive: 'Positive', warning: 'Warning', error: 'Error',
} as const;
export type ThemeColors = Record<keyof typeof colorLabels, string>;
export interface Theme { id: string; name: string; mode: 'dark' | 'light'; colors: ThemeColors }
export const themes: readonly Theme[] = [
  { id: 'atlas', name: 'Midnight atlas', mode: 'dark', colors: { background: '#07121a', surface: '#0b1c27', raised: '#112a3b', border: '#223e50', text: '#dcebf6', muted: '#90adc2', accent: '#71b7e6', selection: '#71b7e6', positive: '#69ba95', warning: '#d9b779', error: '#ef7e89' } },
  { id: 'graphite', name: 'Graphite', mode: 'dark', colors: { background: '#111316', surface: '#191d22', raised: '#252c35', border: '#353e49', text: '#e5e9f0', muted: '#a0aab9', accent: '#adbce9', selection: '#536291', positive: '#7fbdac', warning: '#d3b785', error: '#e8949f' } },
  { id: 'field', name: 'Field journal', mode: 'dark', colors: { background: '#131914', surface: '#1d261f', raised: '#2b372c', border: '#3c4d3e', text: '#e4e8db', muted: '#adb8a2', accent: '#b4c991', selection: '#706247', positive: '#86c09b', warning: '#d7ba7b', error: '#e29c8e' } },
  { id: 'paper', name: 'Daylight', mode: 'light', colors: { background: '#e9eef2', surface: '#f9fbfd', raised: '#e1e9f0', border: '#bccbd6', text: '#193246', muted: '#526d80', accent: '#176a9a', selection: '#176a9a', positive: '#287454', warning: '#946719', error: '#ae394a' } },
];
const storageKey = 'hoi4-replay.appearance.v1';
const hex = /^#[0-9a-f]{6}$/i;
function restore(): Theme {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(storageKey) ?? 'null');
    if (saved && typeof saved === 'object' && 'colors' in saved && saved.colors && typeof saved.colors === 'object'
      && 'mode' in saved && (saved.mode === 'light' || saved.mode === 'dark')
      && Object.keys(colorLabels).every((key) => hex.test(String((saved.colors as Record<string, unknown>)[key])))) {
      const colors = Object.fromEntries(Object.keys(colorLabels).map((key) => [key, (saved.colors as Record<string, string>)[key]])) as ThemeColors;
      // Upgrade the old built-in red selections while retaining custom palettes.
      for (const [id, selection] of [['atlas', '#963d49'], ['paper', '#9d3e50']]) {
        const preset = themes.find(theme => theme.id === id)!;
        if (saved.mode === preset.mode && colors.selection.toLowerCase() === selection
          && Object.keys(colorLabels).every(key => key === 'selection' || colors[key as keyof ThemeColors].toLowerCase() === preset.colors[key as keyof ThemeColors])) return preset;
      }
      return { id: 'custom', name: 'Custom theme', mode: saved.mode, colors };
    }
  } catch { /* A blocked storage area still permits a session theme. */ }
  return themes[0];
}
export function useTheme() {
  const [theme, setTheme] = useState<Theme>(restore);
  const [notice, setNotice] = useState<string | null>(null);
  useLayoutEffect(() => {
    const root = document.documentElement;
    for (const [key, value] of Object.entries(theme.colors)) root.style.setProperty(`--${key}`, value);
    root.style.colorScheme = theme.mode;
    root.dataset.theme = theme.id;
    for (const key of ['accent', 'selection'] as const) {
      const rgb = theme.colors[key].slice(1).match(/../g)!.map((v) => {
        const channel = parseInt(v, 16) / 255;
        return channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4;
      });
      root.style.setProperty(`--on-${key}`, rgb[0] * .2126 + rgb[1] * .7152 + rgb[2] * .0722 > .179 ? '#07121a' : '#ffffff');
    }
  }, [theme]);
  function change(next: Theme) {
    setTheme(next);
    try { localStorage.setItem(storageKey, JSON.stringify(next)); setNotice(null); }
    catch { setNotice('Appearance is applied for this session. This browser could not save it.'); }
  }
  return { theme, change, notice };
}
