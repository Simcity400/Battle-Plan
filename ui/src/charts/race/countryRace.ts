/** Country races share presentation; a widget supplies only its recorded values. */
import type { RaceItem } from './race';
import type { Run } from '../series';
import type { StackSegment } from '../stack';

export interface CountryRacePart { id: string; label: string }
export interface CountryRaceEntry {
  tag: string; value: number; label: string; stretch: number | string | null;
  parts?: readonly { id: string; value: number; hatched?: number }[];
}
export interface CountryRaceLine { tag: string; runs: readonly Run[] }

/** Content order selects palette positions, so renamed types keep their colour. */
export function partColours(parts: readonly CountryRacePart[], palette: readonly string[]): Map<string, string> {
  return new Map(parts.map((part, index) => [part.id, palette[index % Math.max(1, palette.length)]]));
}

export function countryRaceItems(entries: readonly CountryRaceEntry[], colours: ReadonlyMap<string, string>, country: (tag: string) => string,
  missingColour: string, exact = true): RaceItem[] {
  return entries.map(entry => ({ key: entry.tag, value: entry.value, label: entry.label,
    // Null prevents amount easing; the race still eases rows to their ranks.
    stretch: exact ? null : entry.stretch,
    segments: entry.parts?.map(part => ({ ...part, color: colours.get(part.id) ?? missingColour }))
      ?? [{ id: 'total', value: entry.value, color: country(entry.tag) }],
  }));
}

/** The shared stack's exact samples expanded into held bands rather than ramps. */
export function heldStackSegment(segment: StackSegment): StackSegment {
  const hours: number[] = [];
  const bands = segment.bands.map(() => ({ low: [] as number[], high: [] as number[] }));
  segment.hours.forEach((hour, index) => {
    if (index > 0) {
      hours.push(hour);
      bands.forEach((band, layer) => { band.low.push(segment.bands[layer].low[index - 1]); band.high.push(segment.bands[layer].high[index - 1]); });
    }
    hours.push(hour);
    bands.forEach((band, layer) => { band.low.push(segment.bands[layer].low[index]); band.high.push(segment.bands[layer].high[index]); });
  });
  return { hours, bands };
}
