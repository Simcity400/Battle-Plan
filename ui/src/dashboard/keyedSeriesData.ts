import { countryFromKey, countryKey } from './countryKey.ts';
import type { CountryRef } from './countrySlots.ts';
import type { HourSpan, SeriesDrop, SeriesRun } from '../generated/dashboard';
import { type FollowedAnswer, rowKey, rowName } from './follow/message.ts';
import type { TileReply } from './tiles/tile';
import type { BoundWindow } from './regionTime';

export interface SeriesKey { tag: string; part: string }

export const keyId = (key: SeriesKey) => `${key.tag}\u0000${key.part}`;

export interface KeyedState { tag: string; part: string; hour: number; run: SeriesRun | null; drop: SeriesDrop | null; missing: string | null }
export interface KeyedStates { branchId: string; decimals: number; states: readonly KeyedState[] }

export interface SeriesPart { part: string; id: string | null; firstHour: number; lastHour: number }
export interface SeriesParts { tag: string; parts: readonly SeriesPart[]; missing: readonly { firstHour: number; lastHour: number; reason: string }[]; partBits: number }
export interface HeldPart extends SeriesPart { spans: readonly HourSpan[] }
export interface HeldParts extends Omit<SeriesParts, 'parts'> { parts: readonly HeldPart[] }

export function heldParts(pieces: readonly SeriesPart[]): HeldPart[] {
  const byPart = new Map<string, SeriesPart[]>();
  for (const piece of pieces) { const list = byPart.get(piece.part); if (list) list.push(piece); else byPart.set(piece.part, [piece]); }
  return [...byPart.values()].map(list => {
    const sorted = [...list].sort((a, b) => a.firstHour - b.firstHour);
    const spans: HourSpan[] = [];
    for (const piece of sorted) {
      const last = spans.at(-1);
      if (last && piece.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, piece.lastHour);
      else spans.push({ firstHour: piece.firstHour, lastHour: piece.lastHour });
    }
    return { part: sorted[0].part, id: sorted.find(piece => piece.id !== null)?.id ?? null, firstHour: spans[0].firstHour, lastHour: spans.at(-1)!.lastHour, spans };
  });
}

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const hour = (value: unknown): value is number => Number.isSafeInteger(value);
const raw = (value: unknown): value is string => typeof value === 'string' && /^-?\d+$/.test(value);
const run = (value: unknown): value is SeriesRun => object(value) && hour(value.firstHour) && hour(value.lastHour)
  && value.firstHour <= value.lastHour && raw(value.valueRaw) && typeof value.firstExact === 'boolean' && typeof value.lastExact === 'boolean';
const drop = (value: unknown): value is SeriesDrop => object(value) && hour(value.hour) && raw(value.beforeRaw) && raw(value.afterRaw);

export function validateKeyedStates(value: unknown): KeyedStates {
  if (!object(value) || typeof value.branchId !== 'string' || !Number.isInteger(value.decimals) || (value.decimals as number) < 0
    || (value.decimals as number) > 12 || !Array.isArray(value.states) || !value.states.every(state => object(state)
      && (typeof state.country === 'string' || typeof state.country === 'number') && typeof state.part === 'string' && hour(state.hour) && (state.run === null || run(state.run))
      && (state.drop === null || drop(state.drop)) && (state.missing === null || typeof state.missing === 'string')))
    throw new Error('The keyed series state reply is malformed.');
  const states = (value.states as (Omit<KeyedState, 'tag'> & { country: CountryRef })[]).map(({ country, ...state }) => ({ ...state, tag: countryKey(country) }));
  return { ...(value as unknown as KeyedStates), states };
}

function disjointParts(parts: readonly SeriesPart[]): boolean {
  const last = new Map<string, number>();
  for (const part of [...parts].sort((a, b) => a.firstHour - b.firstHour)) {
    const before = last.get(part.part);
    if (before !== undefined && part.firstHour <= before) return false;
    last.set(part.part, part.lastHour);
  }
  return true;
}

export function validateSeriesParts(value: unknown, tag: string): SeriesParts {
  const owner = object(value) && Array.isArray(value.owners) && value.owners.length === 1 ? value.owners[0] : undefined;
  if (!object(value) || !Number.isInteger(value.partBits) || (value.partBits as number) < 1 || (value.partBits as number) > 127
    || !object(owner) || (typeof owner.country !== 'string' && typeof owner.country !== 'number') || countryKey(owner.country) !== tag || !Array.isArray(owner.parts) || !owner.parts.every(part => object(part)
    && raw(part.part) && (part.id === undefined || typeof part.id === 'string') && hour(part.firstHour) && hour(part.lastHour)
    && part.firstHour <= part.lastHour) || !disjointParts(owner.parts as SeriesPart[])
    || !Array.isArray(owner.missing) || !owner.missing.every(span => object(span) && hour(span.firstHour) && hour(span.lastHour)
      && typeof span.reason === 'string'))
    throw new Error('The series parts reply is malformed.');
  const parts = (owner.parts as Record<string, unknown>[]).map(part => ({ part: part.part as string, id: (part.id as string | undefined) ?? null,
    firstHour: part.firstHour as number, lastHour: part.lastHour as number }));
  return { tag, parts, missing: owner.missing as SeriesParts['missing'], partBits: value.partBits as number };
}

export const withinParams = (tags: readonly string[], within: ReadonlyMap<string, readonly HourSpan[]> | undefined,
  span: (firstHour: number, lastHour: number) => BoundWindow | null) => within
  ? { within: tags.map(tag => [countryFromKey(tag), (within.get(tag) ?? []).flatMap(held => span(held.firstHour, held.lastHour) ?? [])] as const) } : {};

export function keyedTileParams(keys: readonly SeriesKey[], within: Readonly<Record<string, readonly HourSpan[]>> | undefined,
  span: (firstHour: number, lastHour: number) => BoundWindow | null) {
  const lines = keys.map(({ tag, part }) => ({ country: countryFromKey(tag), part }));
  if (!within) return { lines };
  const byTag: Record<string, readonly HourSpan[]> = {};
  for (const key of keys) {
    const spans = within[keyId(key)] ?? [];
    const held = byTag[key.tag];
    if (held && JSON.stringify(held) !== JSON.stringify(spans)) throw new Error('A country’s parts are drawn over one set of spans.');
    byTag[key.tag] = spans;
  }
  return { lines, within: Object.entries(byTag).map(([tag, spans]) => [countryFromKey(tag), spans.flatMap(held => span(held.firstHour, held.lastHour) ?? [])] as const) };
}

export function followedRow(answer: FollowedAnswer | undefined, key: SeriesKey, partBits: number): number {
  if (!answer || answer.kind === 'notRecorded') return -1;
  const mask = (1n << BigInt(partBits)) - 1n, part = BigInt(key.part);
  for (let row = 0; row < answer.rows; row++) {
    if (rowName(answer, row) === key.tag && (rowKey(answer, row) & mask) === part && rowKey(answer, row) >> BigInt(partBits) !== 0n) return row;
  }
  return -1;
}

export function keyedTileReply(reply: TileReply & { lines: readonly { country: CountryRef; part?: unknown }[] }, keys: readonly SeriesKey[]): TileReply {
  if (!object(reply) || !Array.isArray(reply.lines) || reply.lines.length !== keys.length
    || !reply.lines.every((line, at) => object(line) && countryKey(line.country) === keys[at].tag && (line as { part?: unknown }).part === keys[at].part))
    throw new Error('The keyed tile reply is malformed.');
  return { ...reply, lines: reply.lines.map((line, at) => ({ ...line, country: keyId(keys[at]) })) } as TileReply;
}

export interface ReachedKeyLine { key: SeriesKey; runs: SeriesRun[]; drops: SeriesDrop[]; missing: Map<number, string> }

export function reachedKeyLines(keys: readonly SeriesKey[], states: KeyedStates | undefined): ReachedKeyLine[] {
  const byKey = new Map<string, KeyedState[]>();
  for (const state of states?.states ?? []) {
    const id = keyId(state);
    const held = byKey.get(id);
    if (held) held.push(state); else byKey.set(id, [state]);
  }
  return keys.map(key => {
    const reached = byKey.get(keyId(key)) ?? [];
    const unique = new Map<string, SeriesRun>();
    for (const state of reached) if (state.run) unique.set(`${state.run.firstHour}|${state.run.lastHour}|${state.run.valueRaw}`, state.run);
    return { key, runs: [...unique.values()].sort((a, b) => a.firstHour - b.firstHour),
      drops: reached.flatMap(state => state.drop ? [state.drop] : []),
      missing: new Map(reached.flatMap(state => state.missing === null ? [] : [[state.hour, state.missing] as const])) };
  });
}
