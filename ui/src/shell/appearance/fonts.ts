// Font choices are family names. The default is the system UI font, as in T3 Code
// (Segoe UI on Windows); Inter is bundled and any installed font can be named. A
// chosen font always sits in front of the default stack so missing characters render.
export const BUNDLED_FONT = 'Inter Variable';
const FALLBACK_STACK = `-apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif`;

export interface FontPreset { family: string; label: string; note: string }

export const FONT_PRESETS: readonly FontPreset[] = [
  { family: '', label: 'System', note: 'Default: Segoe UI on Windows' },
  { family: BUNDLED_FONT, label: 'Inter', note: 'Bundled with the app' },
  { family: 'Segoe UI Variable Text', label: 'Segoe UI Variable', note: 'Windows 11 variable font' },
  { family: 'Bahnschrift', label: 'Bahnschrift', note: 'Condensed, DIN-style' },
  { family: 'Georgia', label: 'Georgia', note: 'Serif' },
  { family: 'Cascadia Code', label: 'Cascadia Code', note: 'Monospace' },
];

/** The CSS stack for a family name ('' → system default). */
export function fontStack(family: string): string {
  const name = family.replace(/["'`;{}<>\\]/g, '').trim();
  return name ? `'${name}', ${FALLBACK_STACK}` : FALLBACK_STACK;
}

/** True when the browser renders `family` with its own glyphs rather than a fallback. */
export function fontAvailable(family: string): boolean {
  const name = family.replace(/["'`;{}<>\\]/g, '').trim();
  if (!name || name === BUNDLED_FONT) return true;
  const context = document.createElement('canvas').getContext('2d');
  if (!context) return true;
  const sample = 'mmmmmmmmmmlli1WQ@#&0123456789';
  return ['monospace', 'serif', 'sans-serif'].some(generic => {
    context.font = `72px ${generic}`;
    const fallback = context.measureText(sample).width;
    context.font = `72px '${name}', ${generic}`;
    return context.measureText(sample).width !== fallback;
  });
}

interface LocalFontData { family: string }
declare global { interface Window { queryLocalFonts?: () => Promise<LocalFontData[]> } }

/** Installed families where the Local Font Access API is permitted; otherwise null. */
export async function installedFonts(): Promise<string[] | null> {
  if (!window.queryLocalFonts) return null;
  try {
    const fonts = await window.queryLocalFonts();
    return [...new Set(fonts.map(font => font.family))].sort((a, b) => a.localeCompare(b));
  } catch { return null; }
}
