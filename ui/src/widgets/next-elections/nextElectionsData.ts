import { countryNameDetails } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import type { SortValue } from '../../charts/table/logic';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { dayLabel } from '../../charts/draw.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { holderKey, integerAt, integerText, isHolderTags, isPoliticsRuns, tagAt, validatePoliticsHistory, valueAt } from '../../dashboard/politicsHistory.ts';
import type { PoliticsHistoryReply } from '../../generated/dashboard';

export function validateElectionsReply(value: unknown): PoliticsHistoryReply {
  return validatePoliticsHistory(value, 'elections', reply => Array.isArray(reply.elections) && reply.elections.every(entry =>
    !!entry && typeof entry === 'object' && isHolderTags(entry.tags) && isPoliticsRuns(entry.held, held => held === '0' || held === '1')
    && isPoliticsRuns(entry.refused, refused => refused === '1') && isPoliticsRuns(entry.interval, integerText) && isPoliticsRuns(entry.next, integerText) && isPoliticsRuns(entry.last, integerText)));
}

export interface ElectionRow {
  key: string; tag: string;
  interval: number | null;
  next: number | null;
  termStart: number | null;
}

export const termAt = (row: ElectionRow, hour: number): number | null =>
  row.interval !== null && row.next !== null && row.termStart !== null && row.next > row.termStart ? (hour - row.termStart) / (row.next - row.termStart) : null;
export const hoursLeftAt = (row: ElectionRow, hour: number): number | null => row.next === null ? null : row.next - hour;
export interface ElectionsView {
  rows: ElectionRow[];
  unknown: number;
}

export function electionsAt(reply: Pick<PoliticsHistoryReply, 'elections'>, hour: number, matches: (tag: string, hour: number) => boolean): ElectionsView {
  const rows: ElectionRow[] = [];
  const keys = new Set<string>();
  let unknown = 0;
  for (const entry of reply.elections) {
    const tag = tagAt(entry.tags, hour);
    if (tag === null || !matches(tag, hour)) continue;
    const held = valueAt(entry.held, hour);
    if (held === null) {
      if (valueAt(entry.refused, hour) !== null) unknown++;
      continue;
    }
    if (held !== '1') continue;
    const interval = integerAt(entry.interval, hour), next = integerAt(entry.next, hour), termStart = integerAt(entry.last, hour);
    let key = holderKey(entry.tags)!;
    if (keys.has(key)) key = `${key}#${rows.length}`;
    keys.add(key);
    rows.push({ key, tag, interval, next, termStart });
  }
  return { rows, unknown };
}

export function electionsState(view: ElectionsView, membership: string | null): { kind: 'unavailable' | 'empty'; message: string } | null {
  if (membership) return { kind: 'unavailable', message: membership };
  if (view.rows.length) return null;
  if (view.unknown) return { kind: 'unavailable', message: 'Election eligibility was not recorded at this date.' };
  return { kind: 'empty', message: 'No country holds elections at this date.' };
}

export function termSortValues(rows: readonly ElectionRow[]): (hour: number) => LiveValues<SortValue> {
  const byKey = new Map(rows.map(row => [liveKey(row.key, 'term'), row]));
  return hour => ({ get: key => { const row = byKey.get(key); return row ? termAt(row, hour) : undefined; }, missing: () => 'Not recorded' });
}

export function changeHours(reply: Pick<PoliticsHistoryReply, 'elections' | 'known'>, matching: readonly number[] = []): number[] {
  const hours = new Set<number>(matching);
  const spans = (runs: readonly { firstHour: number; lastHour: number }[]) => { for (const run of runs) { hours.add(run.firstHour); hours.add(run.lastHour + 1); } };
  spans(reply.known);
  for (const entry of reply.elections) {
    for (const tag of entry.tags) hours.add(tag.hour);
    for (const runs of [entry.held, entry.refused, entry.interval, entry.next, entry.last]) spans(runs);
  }
  return [...hours].sort((a, b) => a - b);
}

export function settledHour(hours: readonly number[], hour: number): number {
  let low = 0, high = hours.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (hours[middle] <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  return found < 0 ? hour : hours[found];
}

export function countdownTexts(rows: readonly ElectionRow[], hour: number): Map<string, string> {
  const texts = new Map<string, string>();
  for (const row of rows) {
    if (row.next !== null) texts.set(`days\u0000${row.key}`, daysText(row.next - hour));
    const term = termAt(row, hour);
    texts.set(`term\u0000${row.key}`, term === null ? '—' : termText(term));
  }
  return texts;
}

export function daysLeft(hours: number): number {
  return Math.ceil(hours / 24) || 0;
}
export const daysText = (hours: number): string => {
  const days = daysLeft(hours);
  return days === 1 ? '1 day' : `${usNumber.format(days)} days`;
};
export const intervalText = (months: number): string => `Every ${usNumber.format(months)} ${months === 1 ? 'month' : 'months'}`;
export const termText = (term: number): string => `${usNumber.format(Math.floor(term * 100))}%`;

export const electionValues = (hour: number): Readonly<Record<'term' | 'days' | 'next', (row: ElectionRow) => SortValue>> => ({
  term: row => termAt(row, hour),
  days: row => hoursLeftAt(row, hour),
  next: row => row.next,
});

export function electionTooltip(row: ElectionRow, country: { nameFailure?: Error; nameDetail?: string; name: string; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean }, hour: number): TooltipContent {
  const notRecorded = 'Not recorded';
  const hoursLeft = hoursLeftAt(row, hour);
  return { ...countryNameDetails(country), title: country.name, countryFlag: { flag: country.flag, flagReason: country.flagReason, ...(country.flagDetail !== undefined ? { flagDetail: country.flagDetail } : {}), flagPending: country.flagPending }, rows: [
    { label: 'Election interval', value: row.interval === null ? notRecorded : intervalText(row.interval) },
    { label: 'Next election', value: row.next === null ? notRecorded : dayLabel(row.next) },
    { label: 'Days left', value: hoursLeft === null ? notRecorded : daysText(hoursLeft) },
  ] };
}

export function electionWarnings(reply: Pick<PoliticsHistoryReply, 'warnings'>, view: ElectionsView | null): string[] {
  return view?.unknown ? [...reply.warnings, `${view.unknown} refused election eligibility: not recorded at this hour`] : reply.warnings;
}
