/** Country races share presentation; a widget supplies only its recorded values. */
import type { RaceItem } from './race';
import type { CountryFlagLook } from '../../components/CountryFlag';
import type { Run } from '../series';
import { lineEndHour, runAt } from '../series.ts';
import { buildStack, compactStack, compactStackExtent, forEachCompactStackSegment, stackExtent, stackedSegments } from '../stack.ts';
import type { StackSegment } from '../stack';
import type { CompactStackModel, StackModel } from '../stack';

export interface CountryRaceLooks {
  name: (tag: string, hour?: number) => string;
  color: (tag: string, hour?: number) => string;
  flag?: (tag: string) => string | undefined;
  /** Explicit native-country presence; generic entities leave this absent. */
  countryFlag?: (tag: string, hour?: number) => CountryFlagLook | undefined;
}

/** Full part inventories opt into compact storage at the actual chart boundary. */
export function countryRaceStack(bands: readonly (readonly Run[])[], compact = false): StackModel | CompactStackModel {
  return compact ? compactStack(bands) : buildStack(bands);
}
export function forEachCountryRaceSegment(stack: StackModel | CompactStackModel, from: number, to: number, hoursPerPixel: number,
  held: boolean, visit: (segment: StackSegment) => void): void {
  if ('bands' in stack) forEachCompactStackSegment(stack, from, to, hoursPerPixel / 2,
    segment => visit(held ? heldStackSegment(segment) : segment));
  else countryRaceSegments(stack, from, to, hoursPerPixel, held).forEach(visit);
}
export function countryRaceStackExtent(stack: StackModel | CompactStackModel, from: number, to: number): [number, number] | null {
  return 'bands' in stack ? compactStackExtent(stack, from, to) : stackExtent(stack, from, to);
}

export interface CountryRacePart { id: string; label: string; color?: string; value?: string }
export interface CountryRaceEntry {
  tag: string; value: number; label: string; stretch: number | string | null;
  /** Animation identity; the dated tag still owns content and interactions. */
  entityKey?: string; hour?: number;
  /** Hatching included in an unsplit total, clamped inside its positive count. */
  hatched?: number;
  /** A secondary amount drawn inside the total, without adding to it. */
  inset?: number;
  parts?: readonly { id: string; value: number; hatched?: number; hatchInside?: boolean; color?: string }[];
}
export interface CountryRaceLine {
  tag: string; runs: readonly Run[];
  /** Distinguishes several measures of the same country; tag remains its opaque identity. */
  id?: string; measure?: string;
  /** Fixed measure looks bypass country names and colours (for a level and ceiling). */
  color?: string; area?: boolean; dash?: readonly number[]; endDot?: boolean; muted?: boolean;
  /** Exact recorded text, independent of the plotting number. */
  endLabel?: (hour: number, name: string) => string;
}

/** A supplied read dates country looks and Jump to; picks keep the displayed hour. */
export function countryRaceReadHour(tag: string, moment: number, hours?: ReadonlyMap<string, number>,
  readHour?: (tag: string | null, hour: number) => number | null): number | null {
  return readHour ? readHour(tag, moment) : hours ? hours.get(tag) ?? null : moment;
}

/** A single country is picked directly; several lines need a country selector. */
export function countryRacePickTag(view: string, lines: readonly CountryRaceLine[], mixTag?: string): string | null {
  return view === 'mix' ? mixTag ?? null : lines.length === 1 ? lines[0].tag : null;
}

/** Content order selects palette positions, so renamed types keep their colour. */
export function partColours(parts: readonly CountryRacePart[], palette: readonly string[], catalogue?: ReadonlyMap<string, string>): Map<string, string> {
  return new Map(parts.map((part, index) => [part.id, part.color ?? catalogue?.get(part.id) ?? palette[index % Math.max(1, palette.length)]]));
}

const insideHatch = (value: number, hatched: number) => value > 0 ? Math.min(Math.max(0, hatched), value) : 0;

export function countryRaceItems(entries: readonly CountryRaceEntry[], colours: ReadonlyMap<string, string>, country: (tag: string) => string,
  missingColour: string, exact = true): RaceItem[] {
  return entries.map(entry => ({ key: entry.entityKey ?? entry.tag, tag: entry.tag, hour: entry.hour, value: entry.value, label: entry.label,
    // Null prevents amount easing; the race still eases rows to their ranks.
    stretch: exact ? null : entry.stretch,
    ...(entry.inset !== undefined ? { inset: { value: entry.inset, color: country(entry.tag) } } : {}),
    segments: entry.parts?.map(part => {
      const hatched = part.hatchInside ? insideHatch(part.value, part.hatched ?? 0) : part.hatched;
      return { id: part.id, value: part.value - (part.hatchInside ? hatched ?? 0 : 0), ...(hatched !== undefined ? { hatched } : {}),
        color: part.color ?? colours.get(part.id) ?? missingColour };
    }) ?? [{ id: 'total', value: entry.value - insideHatch(entry.value, entry.hatched ?? 0), color: country(entry.tag),
      ...(entry.hatched !== undefined ? { hatched: insideHatch(entry.value, entry.hatched) } : {}) }],
  }));
}

/** Keyboard stops use only changes; stack boundary stops remain opt-in. */
export function countryRaceHours(lines: readonly CountryRaceLine[]): number[] {
  return [...new Set(lines.flatMap(line => line.runs.map(run => run.firstHour)))].sort((a, b) => a - b);
}

/** End labels stay on the last recorded point, including a view ending in a gap. */
export function countryRaceEnds(lines: readonly CountryRaceLine[], from: number, to: number, until: number | null,
  name: (tag: string, hour?: number) => string, colour: (tag: string, hour?: number) => string,
  format: (value: number) => string) {
  return lines.flatMap(line => {
    const hour = lineEndHour(line.runs, from, Math.min(to, until ?? to));
    if (hour === null) return [];
    const run = runAt(line.runs, hour);
    return run ? [{ id: line.id ?? line.tag, color: line.color ?? colour(line.tag, hour), value: run.value,
      text: line.endLabel?.(hour, name(line.tag, hour)) ?? `${name(line.tag, hour)}${line.measure ? ` · ${line.measure}` : ''} ${format(run.value)}`,
      ...(line.muted !== undefined ? { muted: line.muted } : {}) }] : [];
  });
}

/** Related tooltip countries are named at the same hovered hour; tags stay opaque. */
export function countryRaceTooltipTags(hour: number, tags: readonly string[], related?: (hour: number, tags: readonly string[]) => readonly string[]): string[] {
  return [...new Set([...tags, ...(related?.(hour, tags) ?? [])])];
}

/** Thin geometry on the drawing grid; held bands are the default for new races. */
export function countryRaceSegments(stack: StackModel, from: number, to: number, hoursPerPixel: number, held = true): StackSegment[] {
  const segments = stackedSegments(stack, from, to, hoursPerPixel / 2);
  return held ? segments.map(heldStackSegment) : segments;
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
  return { ...segment, hours, bands };
}
