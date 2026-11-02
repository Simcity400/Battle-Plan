import { countryNameDetails } from '../../charts/tooltip.ts';
import type { PoliticsHistoryReply } from '../../generated/dashboard';
import type { TooltipContent } from '../../charts/tooltip';
import type { SortValue } from '../../charts/table/logic';
import { dayLabel } from '../../charts/draw.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { holderKey, integerAt, integerText, isHolderTags, isPoliticsRuns, tagAt, validatePoliticsHistory, valueAt } from '../../dashboard/politicsHistory.ts';

/** The `elections` history: every holder's eligibility, interval, next date and term start. */
export function validateElectionsReply(value: unknown): PoliticsHistoryReply {
  return validatePoliticsHistory(value, 'elections', reply => Array.isArray(reply.elections) && reply.elections.every(entry =>
    !!entry && typeof entry === 'object' && isHolderTags(entry.tags) && isPoliticsRuns(entry.held, held => held === '0' || held === '1')
    && isPoliticsRuns(entry.refused, refused => refused === '1') && isPoliticsRuns(entry.interval, integerText) && isPoliticsRuns(entry.next, integerText) && isPoliticsRuns(entry.last, integerText)));
}

/** One country holding elections at the shown hour. Null operands were not recorded. */
export interface ElectionRow {
  key: string; tag: string;
  /** Signed months between elections. */
  interval: number | null;
  next: number | null;
  /** The term's start: the last election's hour. */
  termStart: number | null;
  /** Share of the term passed; null unless interval, next date and term start are all recorded. */
  term: number | null;
  /** Hours from the shown hour to the next election. */
  hoursLeft: number | null;
}
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
    rows.push({ key, tag, interval, next, termStart, hoursLeft: next === null ? null : next - hour,
      term: interval !== null && next !== null && termStart !== null && next > termStart ? (hour - termStart) / (next - termStart) : null });
  }
  return { rows, unknown };
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

/** Exact sort operands of the columns other than Country. */
export const ELECTION_VALUES: Readonly<Record<'term' | 'days' | 'next', (row: ElectionRow) => SortValue>> = {
  term: row => row.term,
  days: row => row.hoursLeft,
  next: row => row.next,
};

export function electionTooltip(row: ElectionRow, country: { nameFailure?: Error; nameDetail?: string; name: string; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean }): TooltipContent {
  const notRecorded = 'Not recorded';
  return { ...countryNameDetails(country), title: country.name, countryFlag: { flag: country.flag, flagReason: country.flagReason, ...(country.flagDetail !== undefined ? { flagDetail: country.flagDetail } : {}), flagPending: country.flagPending }, rows: [
    { label: 'Election interval', value: row.interval === null ? notRecorded : intervalText(row.interval) },
    { label: 'Next election', value: row.next === null ? notRecorded : dayLabel(row.next) },
    { label: 'Days left', value: row.hoursLeft === null ? notRecorded : daysText(row.hoursLeft) },
  ] };
}

/** Counted diagnostics behind what the table cannot show. */
export function electionWarnings(reply: Pick<PoliticsHistoryReply, 'warnings'>, view: ElectionsView | null): string[] {
  return view?.unknown ? [...reply.warnings, `${view.unknown} refused election eligibility: not recorded at this hour`] : reply.warnings;
}
