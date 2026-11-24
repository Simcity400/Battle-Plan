import { countryKey } from '../../dashboard/countryKey.ts';
import type { CountryRaceEntry, LiveRaceEntries } from './countryRace';
import type { Decoded } from '../chartSummary';
import { chartSummaryLines } from '../chartSummary.ts';
import { CELL, cellNumber, cellState, cellValue, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { SeriesRankedLine, SeriesState, SeriesStateReply } from '../../generated/dashboard';

export type RankedMeasure = { kind: string; measure: string };
export type RankedLine = Decoded<SeriesRankedLine>;


const isInteger = (value: unknown): value is number => Number.isSafeInteger(value);
const isRaw = (value: unknown): value is string => typeof value === 'string' && /^-?\d+$/.test(value);

function validState(value: unknown): value is SeriesState {
  if (!value || typeof value !== 'object') return false;
  const state = value as Record<string, unknown>;
  const run = state.run as Record<string, unknown> | null;
  return (typeof state.country === 'string' || typeof state.country === 'number') && isInteger(state.hour) && (state.missing === null || typeof state.missing === 'string')
    && (run === null || (!!run && typeof run === 'object' && isRaw(run.valueRaw) && isInteger(run.firstHour) && isInteger(run.lastHour)));
}

export function validateRankReply(value: unknown): SeriesStateReply {
  const reply = value as Partial<SeriesStateReply> | null;
  if (!reply || typeof reply !== 'object' || !isInteger(reply.decimals) || !Array.isArray(reply.states) || !reply.states.every(validState)) {
    throw new Error('The rank reply is malformed.');
  }
  return reply as SeriesStateReply;
}

export function seriesStateProvenInterval(data: unknown, hour: number): readonly [number, number] | null {
  const reply = data as SeriesStateReply;
  let from = -Infinity;
  let until = Infinity;
  for (const state of reply.states) {
    if (!state.run) return [hour, hour];
    from = Math.max(from, state.run.firstHour);
    until = Math.min(until, state.run.lastHour);
  }
  return from <= hour && hour <= until ? [from, until] : [hour, hour];
}

export function validateRankedLines(value: unknown): { decimals: number; lines: RankedLine[] } {
  const reply = value as { decimals?: unknown; lines?: unknown } | null;
  const lines = chartSummaryLines<RankedLine>(reply?.lines, line => typeof line.tag === 'string' && Array.isArray(line.membership)
    && (line.membership as unknown[]).every(span => {
      const { firstHour, lastHour } = (span ?? {}) as { firstHour?: unknown; lastHour?: unknown };
      return isInteger(firstHour) && isInteger(lastHour) && firstHour <= lastHour;
    })
    && Array.isArray(line.missing));
  if (!reply || !isInteger(reply.decimals) || !lines) throw new Error('The ranked chart reply is malformed.');
  return { decimals: reply.decimals, lines };
}

export const rawNumber = (raw: string | bigint | number, decimals: number) => Number(raw) / 10 ** decimals;

export function stateEntries(states: readonly SeriesState[], decimals: number, format: (raw: string, decimals: number) => string,
  hour: number, slots: number): CountryRaceEntry[] {
  const held = states.flatMap(state => state.run ? [{ tag: countryKey(state.country), raw: BigInt(state.run.valueRaw), text: state.run.valueRaw }] : []);
  held.sort((a, b) => (a.raw < b.raw ? 1 : a.raw > b.raw ? -1 : 0) || compareTags(a.tag, b.tag));
  return held.slice(0, Math.max(0, slots)).map(({ tag, text }) => ({ tag, value: rawNumber(text, decimals), label: format(text, decimals), stretch: null, hour }));
}

export function followedEntries(answer: FollowedAnswer | undefined, asked: readonly string[] | null, member: ((tag: string, hour: number) => boolean) | null,
  hour: number, slots: number, decimals: number, format: (raw: string, decimals: number) => string): CountryRaceEntry[] {
  if (!answer || answer.kind === 'notRecorded') return [];
  const rows: { tag: string; raw: number | bigint }[] = [];
  for (let row = 0; row < answer.rows; row++) {
    const at = row * answer.columns;
    if (cellState(answer, at) !== CELL.value) continue;
    const tag = rowName(answer, row) ?? asked?.[row];
    if (!tag || (member && !member(tag, hour))) continue;
    rows.push({ tag, raw: cellNumber(answer, at) ?? cellValue(answer, at) });
  }
  rows.sort((a, b) => (a.raw < b.raw ? 1 : a.raw > b.raw ? -1 : 0) || compareTags(a.tag, b.tag));
  return rows.slice(0, Math.max(0, slots)).map(({ tag, raw }) => ({ tag, value: rawNumber(raw, decimals), label: format(String(raw), decimals), stretch: null, hour }));
}

export function followedRace(entry: { answer: FollowedAnswer | undefined; through: number }, asked: readonly string[] | null,
  member: ((tag: string, hour: number) => boolean) | null, hour: number, slots: number, decimals: number, format: (raw: string, decimals: number) => string) {
  const shown = Math.min(hour, entry.through);
  if (entry.answer?.kind === 'notRecorded') return { entries: [] as CountryRaceEntry[], reason: entry.answer.reason, shown };
  return { entries: followedEntries(entry.answer, asked, member, shown, slots, decimals, format), shown };
}

export function followedRowTags(answer: FollowedAnswer | undefined, asked: readonly string[] | null): string[] {
  if (!answer || answer.kind === 'notRecorded') return [];
  const out: string[] = [];
  for (let row = 0; row < answer.rows; row++) { const tag = rowName(answer, row) ?? asked?.[row]; if (tag) out.push(tag); }
  return out;
}

export function raceLookTags(race: boolean, followed: readonly string[] | null, lineTags: readonly string[]): readonly string[] | undefined {
  return race ? followed ?? undefined : lineTags;
}

export const rowsOf = (entries: readonly CountryRaceEntry[]) => entries.map(entry => entry.tag).sort().join(',');

const sameEntry = (a: CountryRaceEntry, b: CountryRaceEntry) => a.tag === b.tag && a.label === b.label && a.value === b.value && a.hatched === b.hatched
  && (a.parts?.length ?? 0) === (b.parts?.length ?? 0) && (a.parts ?? []).every((part, at) => part.id === b.parts![at]!.id && part.value === b.parts![at]!.value && part.hatched === b.parts![at]!.hatched);

export function liveRace(clock: { getSnapshot(): { position: number } }, axisOffset: number, read: (hour: number) => readonly CountryRaceEntry[],
  version: () => number, byHour: boolean, dated: (hour: number) => number = hour => hour): LiveRaceEntries & { wake(): void } {
  const wakes = new Set<() => void>();
  let made: { hour: number; version: number; entries: readonly CountryRaceEntry[] } | null = null;
  const hour = () => Math.floor(clock.getSnapshot().position) + axisOffset;
  return {
    hour: () => dated(hour()),
    entries: () => {
      const at = hour(), now = version();
      if (!made || (byHour && made.hour !== at) || made.version !== now) {
        const next = read(at);
        const same = made !== null && made.entries.length === next.length
          && next.every((entry, index) => sameEntry(made!.entries[index]!, entry));
        made = { hour: at, version: now, entries: same ? made!.entries : next };
      }
      return made.entries;
    },
    bind: wake => { wakes.add(wake); return () => { wakes.delete(wake); }; },
    wake: () => { for (const wake of [...wakes]) wake(); },
  };
}

export function formatRaw(raw: string, decimals: number, digits = 0): string {
  const negative = raw.startsWith('-'), body = (negative ? raw.slice(1) : raw).padStart(decimals + 1, '0');
  const whole = body.slice(0, body.length - decimals), fraction = body.slice(body.length - decimals, body.length - decimals + Math.min(digits, decimals)).replace(/0+$/, '');
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  const sign = negative && (/[1-9]/.test(whole) || fraction !== '') ? '−' : '';
  return `${sign}${grouped}${fraction ? `.${fraction}` : ''}`;
}
