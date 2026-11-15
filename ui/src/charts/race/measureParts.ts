/** A ranked measure's parts: what a race bar is split into, and what one country's breakdown stacks. A part is either
 * a declared measure of its own (a factory group's count beside the count of every group) or one of a sub-keyed
 * series' parts (a weapon type's stockpile). Missing is never zero: a bar with any part not known is drawn whole. */
import type { CountryRaceEntry } from './countryRace';
import type { RankedMeasure } from './measureRaceData';
import { CELL, cellNumber, cellState, cellValue, rowKey, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { SeriesState, SeriesStateReply } from '../../generated/dashboard';

/** One part as a measure of its own; with `hatchOf`, the amount hatched inside part `hatchOf`'s bar (damaged factories
 * inside the built ones), drawn in no bar and no stack of its own. */
export interface PartMeasure { id: string; label: string; measure: RankedMeasure; hatchOf?: string }

/** The parts drawn as bars and layers: every part but those hatched inside another. */
export const drawnItems = (items: readonly PartMeasure[]) => items.filter(item => !item.hatchOf);

/** Each drawn part's hatch part, by the drawn part's id. */
export const hatchesOf = (items: readonly PartMeasure[]) => new Map(items.flatMap(item => item.hatchOf ? [[item.hatchOf, item.id] as const] : []));

export type MeasureParts =
  /** Each part a declared measure of the ranked measure's series kind, keyed like it. */
  | { kind: 'measures'; label: string; items: readonly PartMeasure[]; decimals: number;
    /** Bars split in proportion to the parts' values, at the total's length (a score the game multiplies after
     * adding its parts); otherwise each part is drawn at its own value. */
    proportional?: boolean }
  /** The parts of a sub-keyed series an owner holds, by key; `name` titles a part's key. */
  | { kind: 'keyed'; label: string; family: RankedMeasure; partBits: number; decimals: number; name: (part: string) => string };

/** A country's parts at an hour, by part id: a value, or null where it is not known. */
export type PartValues = ReadonlyMap<string, number | null>;

const NUMBER = (raw: number | bigint, decimals: number) => Number(raw) / 10 ** decimals;

/** Each asked tag's value in a followed answer of one measure (rows named by tag, or the tags asked in order); a row
 * holding no value is null. */
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

/** Each owner's parts in a followed answer of a sub-keyed series (rows named by tag, the part the key's low `partBits`
 * bits): part key to value, null where the row holds none. An owner the answer names no row of is absent. */
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

/** Each tag's value at `hour` in a state reply of one measure; a state with no run is null. */
export function measureStates(reply: SeriesStateReply | undefined, hour: number): Map<string, number | null> {
  const out = new Map<string, number | null>();
  for (const state of reply?.states ?? []) if (state.hour === hour) out.set(state.tag, state.run ? NUMBER(BigInt(state.run.valueRaw), reply!.decimals) : null);
  return out;
}

/** Each owner's parts at `hour` in a state reply of a sub-keyed series (each state names its part). */
export function keyedStates(reply: SeriesStateReply | undefined, hour: number): Map<string, Map<string, number | null>> {
  const out = new Map<string, Map<string, number | null>>();
  for (const state of reply?.states ?? [] as SeriesState[]) {
    if (state.hour !== hour || state.part == null) continue;
    const parts = out.get(state.tag) ?? new Map<string, number | null>();
    parts.set(state.part, state.run ? NUMBER(BigInt(state.run.valueRaw), reply!.decimals) : null);
    out.set(state.tag, parts);
  }
  return out;
}

/** The parts of a `measures` split, by tag, from each part's values by tag. */
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

/** `entry` split into `parts` (in that order) where every part is known: at their own values, or in proportion to
 * them at the entry's length (`proportional`); otherwise the entry whole. A bar is never split by a guess. */
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

/** `entry` whole with its hatch parts' amounts summed hatched inside it, where every one is known; otherwise as it is. */
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

/** The parts a keyed split draws, in key order of every owner's parts shown. */
export function keyedIds(values: Iterable<PartValues>): string[] {
  const ids = new Set<string>();
  for (const parts of values) for (const id of parts.keys()) ids.add(id);
  return [...ids].sort((a, b) => (BigInt(a) < BigInt(b) ? -1 : BigInt(a) > BigInt(b) ? 1 : 0));
}
