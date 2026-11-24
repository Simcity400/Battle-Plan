import { readTokens } from '../charts/tokens.ts';
import { resolveKitTokens } from '../draw/kit.ts';
import type { FamilyTheme } from './contract.ts';
import type { CountryLooksMap } from './looks.ts';

export function readFamilyTheme(
  countryColours?: ReadonlyMap<string, string>,
  countryLooks?: CountryLooksMap,
): FamilyTheme | undefined {
  if (typeof document === 'undefined') return undefined;
  const tokens = readTokens();
  const rootStyle = getComputedStyle(document.documentElement);
  const hatch = rootStyle.getPropertyValue('--hatch').trim() || tokens.muted;
  const hatchEdge = rootStyle.getPropertyValue('--hatch-edge').trim() || tokens.border;
  const derived = resolveKitTokens({
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
