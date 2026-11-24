import { usNumber } from '../../charts/numberFormat.ts';
import { sortRows } from '../../charts/table/logic.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { LogRow } from '../../charts/log/logData';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { subjectToken } from '../../dashboard/chosen.ts';
import { DEFAULT_SORT } from '../most-decorated-divisions/definition.ts';
import type { CareerFacts, CareerMedal, CareerRow, DivisionCareersReply, DivisionRecordReply, RecordEntry } from '../../generated/dashboard';

const isArray = Array.isArray;
const validFacts = (facts: CareerFacts | null) => facts === null || (isArray(facts.awards) && (facts.medals === null || Number.isInteger(facts.medals)));

export function validateDivisionCareers(value: unknown): DivisionCareersReply {
  const reply = value as DivisionCareersReply;
  if (!reply || !isArray(reply.divisions) || !isArray(reply.medals)
    || !isArray(reply.templates) || !isArray(reply.warnings) || reply.divisions.some(row => typeof row.division !== 'string' || !validFacts(row.facts)))
    throw new Error('Invalid division careers reply');
  return reply;
}

export function validateDivisionRecord(value: unknown): DivisionRecordReply {
  const reply = value as DivisionRecordReply;
  if (!reply || typeof reply.division !== 'string'
    || typeof reply.absent !== 'boolean' || !isArray(reply.entries) || !isArray(reply.medals) || !validFacts(reply.facts)
    || reply.entries.some(entry => !Number.isInteger(entry.hour) || typeof entry.award !== 'boolean'))
    throw new Error('Invalid division record reply');
  return reply;
}

export const divisionName = (name: string | null) => name ?? 'Division name unavailable';
export const medalName = (medal: CareerMedal | undefined) => medal?.name ?? 'Medal name unavailable';

export function headerDetail(templateName: string | null, facts: CareerFacts | null): string {
  const medals = facts?.medals == null ? '—' : `${usNumber.format(facts.medals)} ${facts.medals === 1 ? 'medal' : 'medals'}`;
  const level = facts?.levelName ?? '—';
  const casualties = facts?.casualtiesTaken == null ? '—' : `${usNumber.format(facts.casualtiesTaken)} casualties taken`;
  return [templateName ?? '—', medals, level, casualties].join(' · ');
}

export type RecordItem = { entry: RecordEntry; medal: CareerMedal | undefined };

export function recordRows(reply: DivisionRecordReply, show: string): ReadonlyArray<LogRow<RecordItem>> {
  return reply.entries
    .filter(entry => show === 'both' || (show === 'awards') === entry.award)
    .map(entry => ({ key: `${entry.award ? 'award' : 'career'}:${entry.ordinal}`, hour: entry.hour,
      item: { entry, medal: entry.medal === null ? undefined : reply.medals[entry.medal] } }))
    .sort((a, b) => b.hour - a.hour || (b.item.entry.award ? 1 : 0) - (a.item.entry.award ? 1 : 0) || b.item.entry.ordinal - a.item.entry.ordinal);
}

export const entryText = (item: RecordItem) => item.entry.description ?? 'Entry description unavailable';

export function entryTooltip(item: RecordItem): TooltipContent {
  const rows: TooltipRow[] = [{ label: 'Date', value: formatDateTime(item.entry.hour) }];
  if (item.entry.award) rows.push({ label: 'Medal', value: medalName(item.medal), lookupKey: item.medal?.key });
  return { title: entryText(item), rows };
}

export const divisionKey = (row: CareerRow) => row.division;

export const careersStructure = (reply: DivisionCareersReply) => JSON.stringify([reply.missing, reply.divisions, reply.medals, reply.templates, reply.warnings]);
export const recordStructure = (reply: DivisionRecordReply) => JSON.stringify(reply);
export const medalsValue = (row: CareerRow) => row.facts.medals;
const defaultSortValue = { medals: medalsValue }[DEFAULT_SORT.column];
export function topDivision(rows: readonly CareerRow[]): CareerRow | null {
  const first = sortRows(rows, defaultSortValue, DEFAULT_SORT.direction, divisionKey)[0];
  return first && (first.facts.medals ?? 0) > 0 ? first : null;
}

export function divisionChoices(reply: DivisionCareersReply, campaignKey: string, country: (tag: string) => { name: string; nameDetail?: string }) {
  return reply.divisions.map(row => {
    const label = row.tag ? country(row.tag) : undefined;
    return { value: subjectToken(campaignKey, row.division), name: divisionName(row.name),
      ...(label?.nameDetail ? { detail: label.nameDetail } : {}), label: `${divisionName(row.name)} · ${label?.name ?? 'Country not recorded'}` };
  }).sort((a, b) => a.name.localeCompare(b.name) || a.value.localeCompare(b.value)).map(({ name: _name, ...choice }) => choice);
}
