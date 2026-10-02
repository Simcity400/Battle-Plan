/** The colour each country draws in a race widget's views: its recorded map
 * colour (from `useCountryLooks`), else the categorical palette, nudged apart in a
 * fixed order (by tag) when the app asks for fixed colours, so a country keeps its
 * colour as ranks change. */
import { useMemo } from 'react';
import { categoricalPalette, nudgeApart } from '../palette.ts';
import type { ChartTokens } from '../tokens';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';

export function useCountryColours(tags: readonly string[], color: (tag: string) => readonly number[] | null | undefined,
  tokens: Pick<ChartTokens, 'surface' | 'muted' | 'countryColours'>,
  revision: { matches?: (tag: string, hour: number) => boolean }): (tag: string, hour?: number) => string {
  return useMemo(() => {
    const palette = categoricalPalette(tokens.surface);
    const sorted = [...tags].sort(compareTags);
    const fallbackColors = new Map(sorted.map((tag, index) => [tag, palette[index % palette.length]]));
    const raw = sorted.map((tag, index) => {
      const recorded = color(tag);
      return recorded ? { r: recorded[0], g: recorded[1], b: recorded[2] } : parseColor(palette[index % palette.length])!;
    });
    const adjusted = tokens.countryColours === 'fixed' ? nudgeApart(raw, parseColor(tokens.surface) ?? { r: 0, g: 0, b: 0 }) : raw;
    const colors = new Map(sorted.map((tag, index) => [tag, toHex(adjusted[index])]));
    return (tag: string, hour?: number) => hour !== undefined && revision.matches?.(tag, hour) === false
      ? fallbackColors.get(tag) ?? tokens.muted : colors.get(tag) ?? tokens.muted;
  // Tags are opaque (any non-NUL bytes, spaces included), so the list is keyed as JSON,
  // never joined on a delimiter that could make two lists alike.
  }, [JSON.stringify(tags), revision, tokens]); // eslint-disable-line react-hooks/exhaustive-deps
}
