import type { Failure } from '../../dashboard/queryError';
import type { RaceItem } from './race';
import type { CountryFlagLook } from '../../components/CountryFlag';
import type { Run } from '../series';
import { lineEndHour, runAt } from '../series.ts';
import { buildStack, compactStack, compactStackExtent, forEachCompactStackSegment, forEachJoinedSegment, stackExtent, stackedSegments } from '../stack.ts';
import type { StackSegment } from '../stack';
import type { CompactStackModel, StackModel } from '../stack';

export interface CountryRaceLooks {
  name: (tag: string, hour?: number) => string;
  nameFailure?: (tag: string, hour?: number) => Failure | undefined;
  color: (tag: string, hour?: number) => string;
  flag?: (tag: string) => string | undefined;
  countryFlag?: (tag: string, hour?: number) => CountryFlagLook | undefined;
}

export function countryRaceStack(bands: readonly (readonly Run[])[], compact = false): StackModel | CompactStackModel {
  return compact ? compactStack(bands) : buildStack(bands);
}
export function forEachCountryRaceSegment(stack: StackModel | CompactStackModel, from: number, to: number, hoursPerPixel: number,
  held: boolean, visit: (segment: StackSegment) => void): void {
  if ('bands' in stack) forEachCompactStackSegment(stack, from, to, hoursPerPixel / 2,
    segment => visit(held ? heldStackSegment(segment) : segment));
  else countryRaceSegments(stack, from, to, hoursPerPixel, held).forEach(visit);
}
export function forEachJoinedCountryRaceSegment(stack: StackModel | CompactStackModel, from: number, to: number, hoursPerPixel: number,
  held: boolean, visit: (segment: StackSegment) => void): void {
  forEachJoinedSegment(piece => forEachCountryRaceSegment(stack, from, to, hoursPerPixel, held, piece), visit);
}

export function countryRaceStackExtent(stack: StackModel | CompactStackModel, from: number, to: number): [number, number] | null {
  return 'bands' in stack ? compactStackExtent(stack, from, to) : stackExtent(stack, from, to);
}

export interface CountryRacePart { id: string; label: string; color?: string; value?: import('react').ReactNode }
export interface CountryRaceEntry {
  tag: string; value: number; label: string; stretch: number | string | null;
  entityKey?: string; hour?: number;
  hatched?: number;
  inset?: number;
  parts?: readonly { id: string; value: number; hatched?: number; hatchInside?: boolean; color?: string }[];
}
export interface LiveRaceEntries {
  entries(): readonly CountryRaceEntry[];
  hour(): number;
  bind(wake: () => void): () => void;
}

export interface CountryRaceLine {
  tag: string; runs: readonly Run[];
  id?: string; measure?: string;
  color?: string; area?: boolean; dash?: readonly number[]; endDot?: boolean; muted?: boolean;
  endLabel?: (hour: number, name: string) => string | { text: string; suffix: string };
}

export function countryRaceReadHour(tag: string, moment: number, hours?: ReadonlyMap<string, number>,
  readHour?: (tag: string | null, hour: number) => number | null): number | null {
  return readHour ? readHour(tag, moment) : hours ? hours.get(tag) ?? null : moment;
}

export function countryRaceLookHours(tags: readonly string[], view: string, linesView: boolean, moment: number,
  labelHours: ReadonlyMap<string, number>, countryLookHours?: ReadonlyMap<string, number>,
  readHour?: (tag: string, hour: number) => number | null): ReadonlyMap<string, number> | undefined {
  if (readHour) return new Map(tags.flatMap(tag => {
    const read = readHour(tag, linesView ? labelHours.get(tag) ?? moment : moment);
    return read === null ? [] : [[tag, read] as const];
  }));
  if (view === 'race') return countryLookHours;
  if (!linesView || !labelHours.size) return undefined;
  return new Map(tags.map(tag => [tag, labelHours.get(tag) ?? moment] as const));
}

export function countryRacePickTag(view: string, lines: readonly CountryRaceLine[], mixTag?: string): string | null {
  return view === 'mix' ? mixTag ?? null : lines.length === 1 ? lines[0].tag : null;
}

export function partColours(parts: readonly CountryRacePart[], palette: readonly string[], catalogue?: ReadonlyMap<string, string>): Map<string, string> {
  return new Map(parts.map((part, index) => [part.id, part.color ?? catalogue?.get(part.id) ?? palette[index % Math.max(1, palette.length)]]));
}

const insideHatch = (value: number, hatched: number) => value > 0 ? Math.min(Math.max(0, hatched), value) : 0;

export const countryRaceItems = (entries: readonly CountryRaceEntry[], colours: ReadonlyMap<string, string>, country: (tag: string) => string,
  missingColour: string, exact = true): RaceItem[] => entries.map(entry => countryRaceItem(entry, colours, country, missingColour, exact));

export function countryRaceItem(entry: CountryRaceEntry, colours: ReadonlyMap<string, string>, country: (tag: string) => string,
  missingColour: string, exact = true): RaceItem {
  return ({ key: entry.entityKey ?? entry.tag, tag: entry.tag, hour: entry.hour, value: entry.value, label: entry.label,
    stretch: exact ? null : entry.stretch,
    ...(entry.inset !== undefined ? { inset: { value: entry.inset, color: country(entry.tag) } } : {}),
    segments: entry.parts?.map(part => {
      const hatched = part.hatchInside ? insideHatch(part.value, part.hatched ?? 0) : part.hatched;
      return { id: part.id, value: part.value - (part.hatchInside ? hatched ?? 0 : 0), ...(hatched !== undefined ? { hatched } : {}),
        color: part.color ?? colours.get(part.id) ?? missingColour };
    }) ?? [{ id: 'total', value: entry.value - insideHatch(entry.value, entry.hatched ?? 0), color: country(entry.tag),
      ...(entry.hatched !== undefined ? { hatched: insideHatch(entry.value, entry.hatched) } : {}) }],
  });
}

export function countryRaceHours(lines: readonly CountryRaceLine[]): number[] {
  return [...new Set(lines.flatMap(line => line.runs.map(run => run.firstHour)))].sort((a, b) => a - b);
}

export interface CountryRaceEnd { id: string; color: string; value: number; text: string; suffix?: string; muted?: boolean }

export function countryRaceEnds(lines: readonly CountryRaceLine[], from: number, to: number, until: number | null,
  name: (tag: string, hour?: number) => string, colour: (tag: string, hour?: number) => string,
  format: (value: number) => string): CountryRaceEnd[] {
  return lines.flatMap(line => {
    const hour = lineEndHour(line.runs, from, Math.min(to, until ?? to));
    if (hour === null) return [];
    const run = runAt(line.runs, hour);
    if (!run) return [];
    const label = line.endLabel?.(hour, name(line.tag, hour))
      ?? { text: `${name(line.tag, hour)}${line.measure ? ` · ${line.measure}` : ''}`, suffix: format(run.value) };
    return [{ id: line.id ?? line.tag, color: line.color ?? colour(line.tag, hour), value: run.value,
      ...(typeof label === 'string' ? { text: label } : label),
      ...(line.muted !== undefined ? { muted: line.muted } : {}) }];
  });
}

export function countryRaceTooltipTags(hour: number, tags: readonly string[], related?: (hour: number, tags: readonly string[]) => readonly string[]): string[] {
  return [...new Set([...tags, ...(related?.(hour, tags) ?? [])])];
}

export function countryRaceSegments(stack: StackModel, from: number, to: number, hoursPerPixel: number, held = true): StackSegment[] {
  const segments = stackedSegments(stack, from, to, hoursPerPixel / 2);
  return held ? segments.map(heldStackSegment) : segments;
}

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

export function raceLabelAt(label: string | ((hour: number) => string) | undefined, hour: number): string | undefined {
  return typeof label === 'function' ? label(hour) : label;
}
