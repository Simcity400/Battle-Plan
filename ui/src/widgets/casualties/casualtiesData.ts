import { dayLabel } from '../../charts/draw.ts';
import { formatRatio } from '../../charts/butterfly/layout.ts';
import { formatFixed } from '../../charts/valueAxis.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { CELL, cellState, cellValue, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryRaceEntry } from '../../charts/race/countryRace';
import type { SeriesStateReply } from '../../generated/dashboard';

/** The declared series of each country identity's casualty totals, and the measure its top countries rank by. */
export const FAMILY = 'casualty-totals';
export const RANK = 'dealtAndTaken';
export const MEASURES = ['dealt', 'taken', 'unknown', 'partial'] as const;
export type CasualtyMeasure = typeof MEASURES[number];

/** One measure's raw value of each country it holds a row for; null where that row's value is not recorded. */
export type Held = ReadonlyMap<string, bigint | null>;
export type HeldMeasures = Readonly<Record<CasualtyMeasure, Held | null>>;

/** A followed answer's values by tag: its named rows, or the tags asked in order. A country with no row is absent. */
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

/** A `series-state` reply's values by tag. */
export function stateHeld(reply: SeriesStateReply | undefined): Held | null {
  if (!reply) return null;
  return new Map(reply.states.map(state => [state.tag, state.run ? BigInt(state.run.valueRaw) : null]));
}

/** The countries a ranked answer names, in its order. */
export function rankedTags(answer: FollowedAnswer | undefined): string[] {
  if (!answer || answer.kind === 'notRecorded') return [];
  const out: string[] = [];
  for (let row = 0; row < answer.rows; row++) {
    const tag = rowName(answer, row);
    if (tag && cellState(answer, row * answer.columns) === CELL.value) out.push(tag);
  }
  return out;
}

/** `atLeast` marks a total whose history lost a lifetime's last hours in a gap: a subtotal, not a lower bound. */
export interface CountryCasualties {
  tag: string; hour: number; atLeast: boolean;
  dealt: bigint | null; taken: bigint | null; knownTaken: bigint | null; unknown: bigint | null;
}

/** Each of `tags` that holds a casualty row at `hour`, its taken losses with unknown attackers' when asked. */
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

/** Greatest dealt plus known taken first (unknown never ranks), rows not known last, ties by tag; the first `limit`.
 * `positive` (a top N): a row known to total zero is not ranked, so the top is never padded with 0/0 rows in tag order. */
export function rankCasualties(rows: readonly CountryCasualties[], limit = Infinity, positive = false): CountryCasualties[] {
  const total = (row: CountryCasualties) => row.dealt == null || row.knownTaken == null ? null : row.dealt + row.knownTaken;
  return rows.filter(row => !positive || total(row) !== 0n).sort((a, b) => {
    const x = total(a), y = total(b);
    return x == null ? y == null ? compareTags(a.tag, b.tag) : 1 : y == null ? -1 : x === y ? compareTags(a.tag, b.tag) : x > y ? -1 : 1;
  }).slice(0, limit);
}

/** The race of losses taken among ranked rows: ordered by known taken (unknown never ranks), drawn at taken, which holds
 * unknown attackers' losses when asked; a row whose taken is not known is not raced. */
export function casualtyRaceEntries(rows: readonly CountryCasualties[]): CountryRaceEntry[] {
  return rows.filter(row => row.taken !== null && row.knownTaken !== null).sort((a, b) => {
    const x = a.knownTaken!, y = b.knownTaken!;
    return x === y ? compareTags(a.tag, b.tag) : x > y ? -1 : 1;
  }).map(row => ({ tag: row.tag, value: casualtyBarValue(row.taken)!, label: formatCasualties(row.taken, row.atLeast), stretch: casualtyBarStretch(row), hour: row.hour }));
}

export const formatCasualties = (value: bigint | null, incomplete = false) => value == null ? '—' : `${incomplete ? '† ' : ''}${formatFixed(value, 0, 0)}`;
const signedCorrection = (row: CountryCasualties) => row.dealt !== null && row.dealt < 0n || row.taken !== null && row.taken < 0n;
export const casualtyRatio = (row: CountryCasualties) => row.atLeast || signedCorrection(row) ? '—' : formatRatio(row.dealt, row.taken);
/** Exact signed labels; bars are positive-only geometry. */
export const casualtyBarValue = (value: bigint | null) => value === null ? null : Number(value > 0n ? value : 0n);
/** A nonpositive read clears old positive geometry at once. */
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
