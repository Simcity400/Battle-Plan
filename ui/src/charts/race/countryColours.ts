import { useMemo, useRef } from 'react';
import { colour, nudgeApart } from '../palette.ts';
import type { ChartTokens } from '../tokens';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';

export function useCountryColours(tags: readonly string[], color: (tag: string) => readonly number[] | null | undefined,
  tokens: Pick<ChartTokens, 'surface' | 'muted' | 'countryColours'>,
  revision: { matches?: (tag: string, hour: number) => boolean }): (tag: string, hour?: number) => string {
  const kept = useRef(tags);
  if (kept.current !== tags && (kept.current.length !== tags.length || kept.current.some((tag, index) => tag !== tags[index]))) kept.current = tags;
  const listed = kept.current;
  const colorKey = listed.map(tag => color(tag)?.join(',') ?? '').join(';');
  const matches = revision.matches;
  return useMemo(() => {
    const { colors, fallbackColors } = countryColourMaps(listed, color, tokens);
    return (tag: string, hour?: number) => hour !== undefined && matches?.(tag, hour) === false
      ? fallbackColors.get(tag) ?? tokens.muted : colors.get(tag) ?? tokens.muted;
  }, [listed, colorKey, matches, tokens]);
}

export function countryColourMaps(tags: readonly string[], color: (tag: string) => readonly number[] | null | undefined,
  tokens: Pick<ChartTokens, 'surface' | 'countryColours'>): { colors: Map<string, string>; fallbackColors: Map<string, string> } {
  const sorted = [...tags].sort(compareTags);
  const fallbackColors = new Map(sorted.map((tag, index) => [tag, colour(tokens.surface, index)]));
  const raw = sorted.map((tag, index) => {
    const recorded = color(tag);
    return recorded ? { r: recorded[0], g: recorded[1], b: recorded[2] } : parseColor(colour(tokens.surface, index))!;
  });
  const adjusted = tokens.countryColours === 'fixed' ? nudgeApart(raw, parseColor(tokens.surface) ?? { r: 0, g: 0, b: 0 }) : raw;
  return { colors: new Map(sorted.map((tag, index) => [tag, toHex(adjusted[index]!)])), fallbackColors };
}
