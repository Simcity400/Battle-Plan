import { resolveKitTokens } from '../draw/kit.ts';
import { mix, rgbaColor } from '../shell/appearance/color.ts';
import { BUILT_IN_THEMES, colorsMode, themeColors, type ThemeColors } from '../shell/appearance/themes.ts';
import type { FamilyTheme } from './contract.ts';

export function familyThemeOf(colors: ThemeColors): FamilyTheme {
  const dark = colorsMode(colors) === 'dark';
  const derived = resolveKitTokens({ surface: colors.surface, raised: colors.raised, text: colors.text, muted: colors.muted, accent: colors.accent, red: colors.error, green: colors.positive });
  return {
    surface: colors.surface,
    ...derived,
    text: colors.text,
    muted: colors.muted,
    line: colors.border,
    lineSoft: mix(colors.border, colors.surface, 0.45),
    accent: colors.accent,
    blue: colors.accent,
    red: colors.error,
    amber: colors.warning,
    green: colors.positive,
    grey: colors.muted,
    hatch: rgbaColor(colors.text, dark ? 0.38 : 0.42),
    hatchEdge: rgbaColor(colors.text, dark ? 0.2 : 0.22),
    font: "'Segoe UI', system-ui, sans-serif",
    fontSize: { small: 11, body: 12 },
    lineWidth: 1,
    hatchSpacing: 8,
  };
}

export const DEFAULT_FAMILY_THEME: FamilyTheme = familyThemeOf(themeColors(BUILT_IN_THEMES[0], 'dark'));
