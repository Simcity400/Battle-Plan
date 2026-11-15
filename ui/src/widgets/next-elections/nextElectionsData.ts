import { countryNameDetails } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import type { SortValue } from '../../charts/table/logic';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { dayLabel } from '../../charts/draw.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { holderKey, integerAt, integerText, isHolderTags, isPoliticsRuns, tagAt, validatePoliticsHistory, valueAt } from '../../dashboard/politicsHistory.ts';
import type { PoliticsHistoryReply } from '../../generated/dashboard';

/** The `elections` history: every holder's eligibility, interval, next date and term start. */
export function validateElectionsReply(value: unknown): PoliticsHistoryReply {
  return validatePoliticsHistory(value, 'elections', reply => Array.isArray(reply.elections) && reply.elections.every(entry =>
    !!entry && typeof entry === 'object' && isHolderTags(entry.tags) && isPoliticsRuns(entry.held, held => held === '0' || held === '1')
    && isPoliticsRuns(entry.refused, refused => refused === '1') && isPoliticsRuns(entry.interval, integerText) && isPoliticsRuns(entry.next, integerText) && isPoliticsRuns(entry.last, integerText)));
}

/** One country holding elections over the hours its schedule holds. Null operands were not recorded. */
export interface ElectionRow {
  key: string; tag: string;
  /** Signed months between elections. */
  interval: number | null;
  next: number | null;
  /** The term's start: the last election's hour. */
  termStart: number | null;
}

/** Share of `row`'s term passed at `hour`; null unless interval, next date and term start are all recorded. */
export const termAt = (row: ElectionRow, hour: number): number | null =>
  row.interval !== null && row.next !== null && row.termStart !== null && row.next > row.termStart ? (hour - row.termStart) / (row.next - row.termStart) : null;
/** Hours from `hour` to `row`'s next election. */
export const hoursLeftAt = (row: ElectionRow, hour: number): number | null => row.next === null ? null : row.next - hour;
export interface ElectionsView {
  rows: ElectionRow[];
  /** Countries whose schedule was read at this hour but whose eligibility was not. */
  unknown: number;
}

/** Countries whose recorded game predicate says they hold elections at `hour`, narrowed by `matches`.
 * Known non-holders are left out; countries with unknown eligibility are counted, never listed. */
export function electionsAt(reply: Pick<PoliticsHistoryReply, 'elections'>, hour: number, matches: (tag: string, hour: number) => boolean): ElectionsView {
  const rows: ElectionRow[] = [];
  const keys = new Set<string>();
  let unknown = 0;
  for (const entry of reply.elections) {
    const tag = tagAt(entry.tags, hour);
    if (tag === null || !matches(tag, hour)) continue;
    const held = valueAt(entry.held, hour);
    if (held === null) {
      // The reply dates every refused eligibility; a country with no schedule read holds none.
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

/** What the widget says instead of its table at an hour: the chosen group's membership not recorded there
 * (`membership`), then eligibility not recorded, before no country holding elections. Null when it lists rows. */
export function electionsState(view: ElectionsView, membership: string | null): { kind: 'unavailable' | 'empty'; message: string } | null {
  if (membership) return { kind: 'unavailable', message: membership };
  if (view.rows.length) return null;
  // A country whose eligibility was not read may hold elections: no row is not proof of none.
  if (view.unknown) return { kind: 'unavailable', message: 'Election eligibility was not recorded at this date.' };
  return { kind: 'empty', message: 'No country holds elections at this date.' };
}

/** Each row's Term sort operand at an hour, keyed `liveKey(row.key, 'term')` as the table's live values are. */
export function termSortValues(rows: readonly ElectionRow[]): (hour: number) => LiveValues<SortValue> {
  const byKey = new Map(rows.map(row => [liveKey(row.key, 'term'), row]));
  return hour => ({ get: key => { const row = byKey.get(key); return row ? termAt(row, hour) : undefined; }, missing: () => 'Not recorded' });
}

/** Every hour at which a row of the elections view may change: each run's and tag's start, the hour after each
 * run's end, and `matching` (where the countries chosen change), ascending. Between two of them the table is the
 * same; only the countdown moves. */
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

/** The last of `hours` at or before `hour`: the hour the table at `hour` is drawn at. `hour` itself before the first. */
export function settledHour(hours: readonly number[], hour: number): number {
  let low = 0, high = hours.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (hours[middle] <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  return found < 0 ? hour : hours[found];
}

/** The countdown cells at `hour`, by row: days left and term passed, painted each frame. */
export function countdownTexts(rows: readonly ElectionRow[], hour: number): Map<string, string> {
  const texts = new Map<string, string>();
  for (const row of rows) {
    if (row.next !== null) texts.set(`days\u0000${row.key}`, daysText(row.next - hour));
    const term = termAt(row, hour);
    texts.set(`term\u0000${row.key}`, term === null ? '—' : termText(term));
  }
  return texts;
}

/** Whole days to the next election, counting a part day as a day. */
export function daysLeft(hours: number): number {
  return Math.ceil(hours / 24) || 0;
}
export const daysText = (hours: number): string => {
  const days = daysLeft(hours);
  return days === 1 ? '1 day' : `${usNumber.format(days)} days`;
};
export const intervalText = (months: number): string => `Every ${usNumber.format(months)} ${months === 1 ? 'month' : 'months'}`;
export const termText = (term: number): string => `${usNumber.format(Math.floor(term * 100))}%`;

/** Exact sort operands of the columns other than Country at `hour`. */
export const electionValues = (hour: number): Readonly<Record<'term' | 'days' | 'next', (row: ElectionRow) => SortValue>> => ({
  term: row => termAt(row, hour),
  days: row => hoursLeftAt(row, hour),
  next: row => row.next,
});

/** `row`'s tooltip at the shown `hour`. */
export function electionTooltip(row: ElectionRow, country: { nameFailure?: Error; nameDetail?: string; name: string; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean }, hour: number): TooltipContent {
  const notRecorded = 'Not recorded';
  const hoursLeft = hoursLeftAt(row, hour);
  return { ...countryNameDetails(country), title: country.name, countryFlag: { flag: country.flag, flagReason: country.flagReason, ...(country.flagDetail !== undefined ? { flagDetail: country.flagDetail } : {}), flagPending: country.flagPending }, rows: [
    { label: 'Election interval', value: row.interval === null ? notRecorded : intervalText(row.interval) },
    { label: 'Next election', value: row.next === null ? notRecorded : dayLabel(row.next) },
    { label: 'Days left', value: hoursLeft === null ? notRecorded : daysText(hoursLeft) },
  ] };
}

/** Counted diagnostics behind what the table cannot show. */
export function electionWarnings(reply: Pick<PoliticsHistoryReply, 'warnings'>, view: ElectionsView | null): string[] {
  return view?.unknown ? [...reply.warnings, `${view.unknown} refused election eligibility: not recorded at this hour`] : reply.warnings;
}
