import { countryNameDetails } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import type { SortValue } from '../../charts/table/logic';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { dayLabel } from '../../charts/draw.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { int, object, opaqueTag } from '../../dashboard/validation.ts';
import type { NextElectionsReply } from '../../generated/dashboard';

export function validateElectionsReply(value: unknown): NextElectionsReply {
  const reply = value as Record<string, unknown>;
  const ok = object(reply) && typeof reply.branchId === 'string' && typeof reply.recorded === 'boolean'
    && int(reply.unknown) && reply.unknown >= 0
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.rows) && reply.rows.every(entry => object(entry)
      && typeof entry.key === 'string' && entry.key.length > 0 && opaqueTag(entry.tag)
      && (entry.interval === null || int(entry.interval))
      && (entry.next === null || int(entry.next))
      && (entry.last === null || int(entry.last)));
  if (!ok) throw new Error('The next elections reply is malformed.');
  return value as NextElectionsReply;
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

export function electionsStructure(reply: NextElectionsReply): string {
  return `${reply.recorded ? '1' : '0'}:${reply.unknown}:${reply.rows.map(row => `${row.key}\0${row.tag}\0${row.interval ?? ''}\0${row.next ?? ''}\0${row.last ?? ''}`).join('\n')}`;
}

export function electionsAt(reply: Pick<NextElectionsReply, 'rows' | 'unknown'>, hour: number, matches: (tag: string, hour: number) => boolean): ElectionsView {
  const rows: ElectionRow[] = [];
  const keys = new Set<string>();
  for (const entry of reply.rows) {
    if (!matches(entry.tag, hour)) continue;
    let key = entry.key;
    if (keys.has(key)) key = `${key}#${rows.length}`;
    keys.add(key);
    rows.push({ key, tag: entry.tag, interval: entry.interval, next: entry.next, termStart: entry.last });
  }
  return { rows, unknown: reply.unknown };
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

export function electionWarnings(reply: Pick<NextElectionsReply, 'warnings'>, view: ElectionsView | null): string[] {
  return view?.unknown ? [...reply.warnings, `${view.unknown} refused election eligibility: not recorded at this hour`] : reply.warnings;
}
