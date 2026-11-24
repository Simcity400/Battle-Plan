import type { GatedTime, PinnedSpec } from '../../dashboard/regionTime';
import { usNumber } from '../../charts/numberFormat.ts';
import { sortRows } from '../../charts/table/logic.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { LogRow } from '../../charts/log/logData';
import { fixedNumber } from '../../charts/valueAxis.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { createWidget } from '../../dashboard/model.ts';
import { subjectToken } from '../../dashboard/chosen.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
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

export function experienceRatio(facts: CareerFacts): number | null {
  if (facts.experienceRaw === null) return null;
  const value = fixedNumber(facts.experienceRaw, 5);
  return value >= 0 && value <= 1 ? value : null;
}

export const CASUALTIES_TAKEN_NOTE = 'The game’s own count of the men this division has lost, as shown on its history tab in the game. It leaves out the men lost when a division is destroyed and losses from paradrop landings.';

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

export function careerTooltip(row: CareerRow, reply: DivisionCareersReply, hour: number, country: string, template: string | null): TooltipContent {
  const facts = row.facts;
  const rows: TooltipRow[] = [
    { label: 'Country', value: country },
    { label: 'Template', value: template ?? 'Template name unavailable' },
    { label: 'Medals', value: facts.medals === null ? 'Not recorded' : usNumber.format(facts.medals), divider: true },
  ];
  for (const award of facts.awards) {
    const medal = award.medal === null ? undefined : reply.medals[award.medal];
    rows.push({ label: award.medal === null ? 'Medal not identified' : medalName(medal), lookupKey: medal?.key, value: award.count === 1 ? undefined : `×${award.count}`, indent: true });
  }
  const ratio = experienceRatio(facts);
  rows.push({ label: 'Experience', value: facts.levelName === null ? 'Not recorded' : `${facts.levelName}${ratio === null ? '' : ` (${Math.round(ratio * 100)}%)`}`, divider: true },
    { label: 'Casualties taken', value: facts.casualtiesTaken === null ? 'Not recorded' : usNumber.format(facts.casualtiesTaken), detail: CASUALTIES_TAKEN_NOTE });
  return { title: divisionName(row.name), subtitle: `As of ${formatDateTime(hour)}`, rows };
}

export function pinnedRecord(source: WidgetSpec, division: string, track: Pick<BoundTrack, 'id' | 'branch' | 'source'>, time: GatedTime): PinnedSpec {
  const base = createWidget('division-service-record', track.id);
  return { ...base, campaign: source.campaign, trackIds: [...source.trackIds], countries: source.countries, time,
    contentScale: source.contentScale, options: { division: subjectToken(track.source.metadata.campaignKey, division) } };
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
