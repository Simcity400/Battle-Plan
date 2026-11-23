import { countryKey } from '../../dashboard/countryKey.ts';
import { dayLabel } from '../../charts/draw.ts';
import { formatRatio } from '../../charts/butterfly/layout.ts';
import { formatFixed } from '../../charts/valueAxis.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { CELL, cellState, cellValue, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryRaceEntry } from '../../charts/race/countryRace';
import type { SeriesStateReply } from '../../generated/dashboard';

export const FAMILY = 'casualty-totals';
export const RANK = 'dealtAndTaken';
export const MEASURES = ['dealt', 'taken', 'unknown', 'partial'] as const;
export type CasualtyMeasure = typeof MEASURES[number];

export type Held = ReadonlyMap<string, bigint | null>;
export type HeldMeasures = Readonly<Record<CasualtyMeasure, Held | null>>;

export function answerHeld(answer: FollowedAnswer | undefined, asked: readonly string[]): Held | null {
  if (!answer || answer.kind === 'notRecorded') return null;
  const out = new Map<string, bigint | null>();
  for (let row = 0; row < answer.rows; row++) {
    const tag = rowName(answer, row) ?? asked[row];
    if (!tag) continue;
    const state = cellState(answer, row * answer.columns);
    if (state === CELL.noRow) continue;
    out.set(tag, state === CELL.value ? cellValue(answer, row * answer.columns) : null);
  }
  return out;
}

export function stateHeld(reply: SeriesStateReply | undefined): Held | null {
  if (!reply) return null;
  return new Map(reply.states.map(state => [countryKey(state.country), state.run ? BigInt(state.run.valueRaw) : null]));
}

export function rankedTags(answer: FollowedAnswer | undefined): string[] {
  if (!answer || answer.kind === 'notRecorded') return [];
  const out: string[] = [];
  for (let row = 0; row < answer.rows; row++) {
    const tag = rowName(answer, row);
    if (tag && cellState(answer, row * answer.columns) === CELL.value) out.push(tag);
  }
  return out;
}

export interface CountryCasualties {
  tag: string; hour: number; atLeast: boolean;
  dealt: bigint | null; taken: bigint | null; knownTaken: bigint | null; unknown: bigint | null;
}

export function casualtyRows(tags: readonly string[], held: HeldMeasures, hour: number, includeUnknown: boolean): CountryCasualties[] {
  const rows: CountryCasualties[] = [];
  for (const tag of tags) {
    if (!held.dealt?.has(tag) && !held.taken?.has(tag)) continue;
    const dealt = held.dealt?.get(tag) ?? null, knownTaken = held.taken?.get(tag) ?? null, unknown = held.unknown?.get(tag) ?? null;
    const partial = held.partial?.get(tag);
    rows.push({ tag, hour, atLeast: partial != null && partial !== 0n, dealt, knownTaken, unknown,
      taken: includeUnknown ? knownTaken == null || unknown == null ? null : knownTaken + unknown : knownTaken });
  }
  return rows;
}

export function rankCasualties(rows: readonly CountryCasualties[], limit = Infinity, positive = false): CountryCasualties[] {
  const total = (row: CountryCasualties) => row.dealt == null || row.knownTaken == null ? null : row.dealt + row.knownTaken;
  return rows.filter(row => !positive || total(row) !== 0n).sort((a, b) => {
    const x = total(a), y = total(b);
    return x == null ? y == null ? compareTags(a.tag, b.tag) : 1 : y == null ? -1 : x === y ? compareTags(a.tag, b.tag) : x > y ? -1 : 1;
  }).slice(0, limit);
}

export function casualtyRaceEntries(rows: readonly CountryCasualties[]): CountryRaceEntry[] {
  return rows.filter(row => row.taken !== null && row.knownTaken !== null).sort((a, b) => {
    const x = a.knownTaken!, y = b.knownTaken!;
    return x === y ? compareTags(a.tag, b.tag) : x > y ? -1 : 1;
  }).map(row => ({ tag: row.tag, value: casualtyBarValue(row.taken)!, label: formatCasualties(row.taken, row.atLeast), stretch: casualtyBarStretch(row), hour: row.hour }));
}

export const formatCasualties = (value: bigint | null, incomplete = false) => value == null ? '—' : `${incomplete ? '† ' : ''}${formatFixed(value, 0, 0)}`;
const signedCorrection = (row: CountryCasualties) => row.dealt !== null && row.dealt < 0n || row.taken !== null && row.taken < 0n;
export const casualtyRatio = (row: CountryCasualties) => row.atLeast || signedCorrection(row) ? '—' : formatRatio(row.dealt, row.taken);
export const casualtyBarValue = (value: bigint | null) => value === null ? null : Number(value > 0n ? value : 0n);
export const casualtyBarStretch = (row: CountryCasualties) => JSON.stringify([row.tag, row.taken !== null && row.taken > 0n, row.dealt !== null && row.dealt > 0n]);

export const SUBTOTAL_NOTE = '† Incomplete total; ratio unavailable.';

export function casualtyTooltip(row: CountryCasualties, name: string, unknownLabel: string): TooltipContent {
  return { title: name, subtitle: dayLabel(row.hour), rows: [
    { label: 'Dealt', value: formatCasualties(row.dealt, row.atLeast) },
    { label: 'Taken (known attackers)', value: formatCasualties(row.knownTaken, row.atLeast) },
    { label: unknownLabel, value: row.unknown == null ? 'Unknown' : formatCasualties(row.unknown), tone: 'muted' },
    { label: 'Taken', value: formatCasualties(row.taken, row.atLeast), strong: true },
    { label: 'Ratio', value: casualtyRatio(row), strong: true },
  ], note: [row.atLeast ? SUBTOTAL_NOTE : '',
    signedCorrection(row) ? 'Ratio unavailable for corrections.' : '',
    row.unknown == null ? 'Losses from unknown attackers are unavailable.' : ''].filter(Boolean).join(' ') || undefined };
}

export interface HeldCasualties<L> { readonly lives: readonly (L | undefined)[]; readonly members: readonly string[] }

export function heldCasualties<L>(held: HeldCasualties<L> | null, lives: readonly (L | undefined)[], members: readonly string[], following: boolean, refused: string | null, current = true) {
  if (!following || refused !== null) return { held: null, lives, members, awaiting: false };
  if (current && lives.every(live => live !== undefined)) return { held: { lives, members }, lives, members, awaiting: false };
  if (held) return { held, lives: held.lives, members: held.members, awaiting: false };
  return { held: null, lives, members, awaiting: members.length > 0 };
}

const retained = new WeakMap<object, unknown>();

export function retainedLive<E extends { readonly answer: unknown }>(live: { read(): E; subscribe(changed: () => void): () => void }) {
  let held = retained.get(live) as typeof live | undefined;
  if (!held) {
    let last: E | undefined;
    held = { read: () => { const now = live.read(); if (now.answer !== undefined) last = { ...now }; return last ?? now; }, subscribe: changed => live.subscribe(changed) };
    retained.set(live, held);
  }
  return held;
}
