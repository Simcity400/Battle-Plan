import { countryKey } from '../../dashboard/countryKey.ts';
import type { CountryRaceEntry } from './countryRace';
import type { RankedMeasure } from './measureRaceData';
import { CELL, cellNumber, cellState, cellValue, rowKey, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { SeriesState, SeriesStateReply } from '../../generated/dashboard';

export interface PartMeasure { id: string; label: string; measure: RankedMeasure; hatchOf?: string }

export const drawnItems = (items: readonly PartMeasure[]) => items.filter(item => !item.hatchOf);

export const hatchesOf = (items: readonly PartMeasure[]) => new Map(items.flatMap(item => item.hatchOf ? [[item.hatchOf, item.id] as const] : []));

export type MeasureParts =
  | { kind: 'measures'; label: string; items: readonly PartMeasure[]; decimals: number;
    proportional?: boolean }
  | { kind: 'keyed'; label: string; family: RankedMeasure; partBits: number; decimals: number; name: (part: string) => string };

export type PartValues = ReadonlyMap<string, number | null>;

const NUMBER = (raw: number | bigint, decimals: number) => Number(raw) / 10 ** decimals;

export function measureAnswer(answer: FollowedAnswer | undefined, asked: readonly string[], decimals: number): Map<string, number | null> {
  const out = new Map<string, number | null>();
  if (!answer || answer.kind === 'notRecorded') return out;
  for (let row = 0; row < answer.rows; row++) {
    const tag = rowName(answer, row) ?? asked[row];
    if (!tag) continue;
    const at = row * answer.columns;
    out.set(tag, cellState(answer, at) === CELL.value ? NUMBER(cellNumber(answer, at) ?? cellValue(answer, at), decimals) : null);
  }
  return out;
}

export function keyedAnswer(answer: FollowedAnswer | undefined, partBits: number, decimals: number): Map<string, Map<string, number | null>> {
  const out = new Map<string, Map<string, number | null>>();
  if (!answer || answer.kind === 'notRecorded') return out;
  const mask = (1n << BigInt(partBits)) - 1n;
  for (let row = 0; row < answer.rows; row++) {
    const tag = rowName(answer, row);
    const key = rowKey(answer, row);
    if (!tag || key >> BigInt(partBits) === 0n) continue;
    const at = row * answer.columns;
    const parts = out.get(tag) ?? new Map<string, number | null>();
    parts.set(String(key & mask), cellState(answer, at) === CELL.value ? NUMBER(cellNumber(answer, at) ?? cellValue(answer, at), decimals) : null);
    out.set(tag, parts);
  }
  return out;
}

export function measureStates(reply: SeriesStateReply | undefined, hour: number): Map<string, number | null> {
  const out = new Map<string, number | null>();
  for (const state of reply?.states ?? []) if (state.hour === hour) out.set(countryKey(state.country), state.run ? NUMBER(BigInt(state.run.valueRaw), reply!.decimals) : null);
  return out;
}

export function keyedStates(reply: SeriesStateReply | undefined, hour: number): Map<string, Map<string, number | null>> {
  const out = new Map<string, Map<string, number | null>>();
  for (const state of reply?.states ?? [] as SeriesState[]) {
    if (state.hour !== hour || state.part == null) continue;
    const parts = out.get(countryKey(state.country)) ?? new Map<string, number | null>();
    parts.set(state.part, state.run ? NUMBER(BigInt(state.run.valueRaw), reply!.decimals) : null);
    out.set(countryKey(state.country), parts);
  }
  return out;
}

export function measureParts(items: readonly PartMeasure[], byPart: ReadonlyMap<string, ReadonlyMap<string, number | null>>, tags: readonly string[]): Map<string, PartValues> {
  const out = new Map<string, PartValues>();
  for (const tag of tags) {
    const values = new Map<string, number | null>();
    for (const item of items) {
      const value = byPart.get(item.id)?.get(tag);
      values.set(item.id, value === undefined ? null : value);
    }
    out.set(tag, values);
  }
  return out;
}

export function splitEntry(entry: CountryRaceEntry, ids: readonly string[], values: PartValues | undefined, proportional: boolean,
  hatches?: ReadonlyMap<string, string>): CountryRaceEntry {
  if (!values || !ids.length) return entry;
  const known = ids.map(id => values.get(id));
  if (known.some(value => value === null || value === undefined)) return hatchEntry(entry, hatches, values);
  const amounts = known as number[];
  const hatched = (id: string) => {
    const value = hatches?.has(id) ? values.get(hatches.get(id)!) : undefined;
    return value === null || value === undefined ? {} : { hatched: value, hatchInside: true };
  };
  if (!proportional) return { ...entry, parts: ids.map((id, index) => ({ id, value: amounts[index], ...hatched(id) })) };
  const sum = amounts.reduce((total, value) => total + value, 0);
  if (amounts.some(value => value < 0) || entry.value < 0 || (sum === 0 && entry.value !== 0)) return entry;
  return { ...entry, parts: ids.map((id, index) => ({ id, value: sum === 0 ? 0 : entry.value * amounts[index] / sum })) };
}

export function hatchEntry(entry: CountryRaceEntry, hatches: ReadonlyMap<string, string> | undefined, values: PartValues | undefined): CountryRaceEntry {
  if (!hatches?.size || !values) return entry;
  let sum = 0;
  for (const id of hatches.values()) {
    const value = values.get(id);
    if (value === null || value === undefined) return entry;
    sum += value;
  }
  return { ...entry, hatched: sum };
}

export function keyedIds(values: Iterable<PartValues>): string[] {
  const ids = new Set<string>();
  for (const parts of values) for (const id of parts.keys()) ids.add(id);
  return [...ids].sort((a, b) => (BigInt(a) < BigInt(b) ? -1 : BigInt(a) > BigInt(b) ? 1 : 0));
}
