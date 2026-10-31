/** The colour each country draws in a race widget's views: its recorded map
 * colour (from `useCountryLooks`), else the categorical palette, nudged apart in a
 * fixed order (by tag) when the app asks for fixed colours, so a country keeps its
 * colour as ranks change. */
import { useMemo, useRef } from 'react';
import { categoricalPalette, nudgeApart } from '../palette.ts';
import type { ChartTokens } from '../tokens';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';

export function useCountryColours(tags: readonly string[], color: (tag: string) => readonly number[] | null | undefined,
  tokens: Pick<ChartTokens, 'surface' | 'muted' | 'countryColours'>,
  revision: { matches?: (tag: string, hour: number) => boolean }): (tag: string, hour?: number) => string {
  // Callers rebuild the list each render; the same tags in the same order keep the colours.
  const kept = useRef(tags);
  if (kept.current !== tags && (kept.current.length !== tags.length || kept.current.some((tag, index) => tag !== tags[index]))) kept.current = tags;
  const listed = kept.current;
  // The colours themselves, not the looks object rebuilt as names and flags arrive, keep the function.
  const colorKey = listed.map(tag => color(tag)?.join(',') ?? '').join(';');
  const matches = revision.matches;
  return useMemo(() => {
    const palette = categoricalPalette(tokens.surface);
    const sorted = [...listed].sort(compareTags);
    const fallbackColors = new Map(sorted.map((tag, index) => [tag, palette[index % palette.length]]));
    const raw = sorted.map((tag, index) => {
      const recorded = color(tag);
      return recorded ? { r: recorded[0], g: recorded[1], b: recorded[2] } : parseColor(palette[index % palette.length])!;
    });
    const adjusted = tokens.countryColours === 'fixed' ? nudgeApart(raw, parseColor(tokens.surface) ?? { r: 0, g: 0, b: 0 }) : raw;
    const colors = new Map(sorted.map((tag, index) => [tag, toHex(adjusted[index])]));
    return (tag: string, hour?: number) => hour !== undefined && matches?.(tag, hour) === false
      ? fallbackColors.get(tag) ?? tokens.muted : colors.get(tag) ?? tokens.muted;
  }, [listed, colorKey, matches, tokens]); // eslint-disable-line react-hooks/exhaustive-deps
}
