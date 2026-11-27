export const BUNDLED_FONT = 'Inter Variable';
const FALLBACK_STACK = `-apple-system, BlinkMacSystemFont, 'Segoe UI', system-ui, sans-serif`;
export const MONOSPACE_STACK = `'SF Mono', 'SFMono-Regular', Menlo, Consolas, 'Liberation Mono', monospace`;

export interface FontPreset { family: string; label: string; note: string; monospace?: boolean }

export const FONT_PRESETS: readonly FontPreset[] = [
  { family: '', label: 'System', note: 'Platform default font' },
  { family: BUNDLED_FONT, label: 'Inter', note: 'Bundled with the app' },
  { family: 'Roboto Variable', label: 'Roboto Sans', note: 'Bundled sans-serif font' },
  { family: 'Roboto Condensed Variable', label: 'Condensed', note: 'Bundled condensed font' },
  { family: 'Source Serif 4 Variable', label: 'Serif', note: 'Bundled Source Serif 4 font' },
  { family: 'Cascadia Code Variable', label: 'Cascadia Code', note: 'Bundled monospace font', monospace: true },
];
const bundledFamilies = new Set(FONT_PRESETS.map(preset => preset.family));

export function familyNames(family: string): string[] {
  return family.replace(/["'`;{}<>\\]/g, '').split(',').map(name => name.trim()).filter(Boolean);
}
function fontList(family: string): string {
  return familyNames(family).map(name => /^(serif|sans-serif|monospace|system-ui|ui-monospace)$/i.test(name) ? name : `'${name}'`).join(', ');
}
export function fontStack(family: string, fallback = FALLBACK_STACK): string {
  const list = fontList(family);
  return list ? `${list}, ${fallback}` : fallback;
}

export function fontAvailable(family: string): boolean {
  const names = familyNames(family);
  if (!names.length || names.some(name => bundledFamilies.has(name) || /^(serif|sans-serif|monospace|system-ui|ui-monospace)$/i.test(name))) return true;
  const context = document.createElement('canvas').getContext('2d');
  if (!context) return false;
  const sample = 'mmmmmmmmmmlli1WQ@#&0123456789';
  return ['monospace', 'serif', 'sans-serif'].some(generic => {
    context.font = `72px ${generic}`;
    const fallback = context.measureText(sample).width;
    context.font = `72px ${fontList(family)}, ${generic}`;
    return context.measureText(sample).width !== fallback;
  });
}

export function canUseFontFamily(family: string, installed: readonly string[] | null = null): boolean {
  const name = family.trim();
  return !name || familyNames(name).some(name => installed?.includes(name)) || fontAvailable(name);
}

export function isMonospaceFamily(family: string): boolean {
  const names = familyNames(family);
  if (!names.length) return true;
  const firstPreset = FONT_PRESETS.find(preset => preset.family === names[0]);
  if (firstPreset) return !!firstPreset.monospace;
  if (names.length === 1 && /^(monospace|ui-monospace)$/i.test(names[0])) return true;
  const context = document.createElement('canvas').getContext('2d');
  if (!context) return false;
  return ['400', '700', 'italic 400', 'italic 700'].every(weight => {
    context.font = `${weight} 32px ${fontList(family)}, monospace`;
    const widths = ['i', 'M', 'W', '0', '@', '#', '.', ' '].map(glyph => context.measureText(glyph).width);
    return widths[0] > 0 && widths.every(width => Number.isFinite(width) && Math.abs(width - widths[0]) < .01);
  });
}

export function defaultFontLabel(monospace = false): string {
  const stack = monospace ? MONOSPACE_STACK : FALLBACK_STACK;
  for (const name of familyNames(stack)) {
    if (/^(monospace|system-ui|sans-serif|-apple-system|BlinkMacSystemFont)$/i.test(name)) continue;
    if (fontAvailable(name)) return name;
  }
  return monospace ? 'System monospace' : 'System UI';
}

interface LocalFontData { family: string }
declare global { interface Window { queryLocalFonts?: () => Promise<LocalFontData[]> } }

export async function installedFonts(): Promise<string[] | null> {
  if (!window.queryLocalFonts) return null;
  try {
    const fonts = await window.queryLocalFonts();
    return [...new Set(fonts.map(font => font.family))].sort((a, b) => a.localeCompare(b));
  } catch { return null; }
}
