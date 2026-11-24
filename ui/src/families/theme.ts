import { readTokens } from '../charts/tokens.ts';
import { parseColor } from '../shell/appearance/color.ts';
import type { FamilyTheme } from './contract.ts';
import type { CountryLooksMap } from './looks.ts';

export type { FamilyTheme };

export function isDarkSurface(surface: string): boolean {
  const rgb = parseColor(surface);
  if (!rgb) return true;
  return 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b < 128;
}

export function rgbaColor(color: string, alpha: number, fallback: string): string {
  const rgb = parseColor(color);
  if (!rgb) return fallback;
  return `rgba(${Math.round(rgb.r)}, ${Math.round(rgb.g)}, ${Math.round(rgb.b)}, ${alpha})`;
}

export function derivePresentationTokens(base: Pick<FamilyTheme, 'surface' | 'text' | 'muted' | 'accent'> & { raised?: string; dark?: boolean }) {
  const dark = base.dark ?? isDarkSurface(base.surface);
  const ink = dark ? '255, 255, 255' : '15, 23, 42';
  return {
    dark,
    raised: base.raised ?? base.surface,
    hairline: `rgba(${ink}, ${dark ? 0.05 : 0.07})`,
    hairlineStrong: `rgba(${ink}, ${dark ? 0.085 : 0.11})`,
    bandDivider: `rgba(${ink}, ${dark ? 0.12 : 0.15})`,
    headerBand: `rgba(${ink}, ${dark ? 0.025 : 0.03})`,
    rowHover: `rgba(${ink}, ${dark ? 0.045 : 0.05})`,
    rowSelected: rgbaColor(base.accent, 0.14, dark ? 'rgba(96, 165, 250, 0.14)' : 'rgba(37, 99, 235, 0.14)'),
    childTint: `rgba(${ink}, ${dark ? 0.018 : 0.025})`,
    shortage: dark ? '#ff6b6f' : '#e5484d',
    barMuted: `rgba(${ink}, ${dark ? 0.085 : 0.095})`,
    barTrack: `rgba(${ink}, ${dark ? 0.08 : 0.1})`,
    gridLine: `rgba(${ink}, ${dark ? 0.055 : 0.07})`,
    zeroLine: `rgba(${ink}, ${dark ? 0.18 : 0.22})`,
    tailLine: rgbaColor(base.muted, 0.22, `rgba(${ink}, 0.22)`),
    playhead: `rgba(${ink}, ${dark ? 0.42 : 0.46})`,
    crosshair: `rgba(${ink}, ${dark ? 0.5 : 0.52})`,
    dateTabBg: base.accent,
    dateTabText: dark ? '#0b1018' : '#ffffff',
    hoverTabBg: base.raised ?? base.surface,
    hoverTabText: base.text,
  };
}

export function readFamilyTheme(
  countryColours?: ReadonlyMap<string, string>,
  countryLooks?: CountryLooksMap,
): FamilyTheme | undefined {
  if (typeof document === 'undefined') return undefined;
  const tokens = readTokens();
  const rootStyle = getComputedStyle(document.documentElement);
  const hatch = rootStyle.getPropertyValue('--hatch').trim() || tokens.muted;
  const hatchEdge = rootStyle.getPropertyValue('--hatch-edge').trim() || tokens.border;
  const derived = derivePresentationTokens({
    surface: tokens.surface,
    raised: tokens.raised,
    text: tokens.text,
    muted: tokens.muted,
    accent: tokens.accent,
  });
  return {
    surface: tokens.surface,
    ...derived,
    text: tokens.text,
    muted: tokens.muted,
    line: tokens.border,
    lineSoft: tokens.grid,
    accent: tokens.accent,
    blue: tokens.accent,
    red: tokens.error,
    amber: tokens.warning,
    green: tokens.positive,
    grey: tokens.muted,
    hatch,
    hatchEdge,
    font: tokens.font,
    fontSize: { small: tokens.fontSize.small, body: tokens.fontSize.body },
    lineWidth: tokens.lineWidth.grid,
    hatchSpacing: tokens.hatchSpacing,
    country: countryColours ? Object.fromEntries(countryColours) : undefined,
    countryLooks,
  };
}
