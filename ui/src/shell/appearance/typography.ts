import { fontStack, MONOSPACE_STACK } from './fonts.ts';

export type FontRole = 'interface' | 'display' | 'tables' | 'diagnostics';
export type FontFamilies = Record<FontRole, string>;
export interface TypographySettings {
  advanced: boolean;
  sizes: Record<FontRole, number>;
  smoothing: boolean;
  wordWrap: boolean;
}
export const FONT_SIZE_LIMITS = { interface: [12, 20], display: [12, 20], tables: [10, 18], diagnostics: [8, 20] } as const;
export const DEFAULT_TYPOGRAPHY: TypographySettings = {
  advanced: false, sizes: { interface: 16, display: 14, tables: 13, diagnostics: 12 }, smoothing: true, wordWrap: true,
};
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
export function parseTypography(input: unknown, hasHeadingsOverride = false): TypographySettings {
  const data = object(input) ? input : {};
  const sizes = object(data.sizes) ? data.sizes : {};
  return {
    advanced: typeof data.advanced === 'boolean' ? data.advanced : hasHeadingsOverride,
    sizes: Object.fromEntries((Object.keys(FONT_SIZE_LIMITS) as FontRole[]).map(role => {
      const value = sizes[role], [min, max] = FONT_SIZE_LIMITS[role];
      return [role, typeof value === 'number' && Number.isFinite(value) ? Math.round(Math.max(min, Math.min(max, value))) : DEFAULT_TYPOGRAPHY.sizes[role]];
    })) as Record<FontRole, number>,
    smoothing: typeof data.smoothing === 'boolean' ? data.smoothing : DEFAULT_TYPOGRAPHY.smoothing,
    wordWrap: typeof data.wordWrap === 'boolean' ? data.wordWrap : DEFAULT_TYPOGRAPHY.wordWrap,
  };
}

/** Diagnostics follow tables in Simple; Advanced uses their independent preference. */
export function typographyVariables(fonts: FontFamilies, settings: TypographySettings): Record<string, string> {
  const ui = fontStack(fonts.interface);
  const diagnostics = settings.advanced ? fonts.diagnostics : fonts.tables;
  const diagnosticsSize = settings.advanced ? settings.sizes.diagnostics : settings.sizes.tables;
  return {
    '--font-ui': ui, '--font-display': fontStack(fonts.display, ui),
    '--font-mono': fontStack(fonts.tables, MONOSPACE_STACK), '--font-diagnostics': fontStack(diagnostics, MONOSPACE_STACK),
    '--font-scale': String(settings.sizes.interface / 16),
    '--font-size-ui': `${settings.sizes.interface}px`, '--font-size-display': `${settings.sizes.display}px`,
    '--font-size-mono': `${settings.sizes.tables}px`, '--font-size-diagnostics': `${diagnosticsSize}px`,
    ...Object.fromEntries([['xs', 11], ['sm', 12], ['md', 13], ['lg', 14]].map(([name, base]) => [`--type-${name}`, `${Number(base) * settings.sizes.interface / 16}px`])),
  };
}
