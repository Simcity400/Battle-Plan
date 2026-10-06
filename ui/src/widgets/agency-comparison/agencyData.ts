import type { AgencyComparisonParams, AgencyComparisonReply, AgencyRankField, AgencyValue, CountryGroupReply } from '../../generated/dashboard';
import { isOrderedSpans, runAt, spansCovered } from '../../charts/series.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { formatFixed } from '../../charts/valueAxis.ts';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { worldRequestTags } from '../../dashboard/groups.ts';
import { readAt } from '../../dashboard/reads.ts';
import type { CountrySelection } from '../../dashboard/model';
import { object, int, signed64 as raw } from '../../dashboard/validation.ts';

const text = (v: unknown) => v === null || typeof v === 'string';
const upgrade = (v: unknown) => object(v) && (v.count === null || int(v.count))
  && Array.isArray(v.items) && v.items.every(item => object(item) && typeof item.branchToken === 'string' && typeof item.token === 'string' && text(item.name) && 'icon' in item);
const value = (v: unknown) => object(v) && typeof v.created === 'boolean' && text(v.name) && raw(v.defenseRaw) && text(v.defenseText) && text(v.defenseValue)
  && int(v.activeCount) && int(v.maximum) && upgrade(v.upgrades);

export function validateAgencyComparisonReply(data: unknown): AgencyComparisonReply {
  if (!object(data) || typeof data.branchId !== 'string' || typeof data.available !== 'boolean' || !isOrderedSpans(data.observed) || !text(data.upgradesUnrecorded) || !text(data.defenseLabel)
    || !Array.isArray(data.operativeReadHours) || !data.operativeReadHours.every((hour, index, reads) => int(hour) && (!index || reads[index - 1] < hour))
    || !Array.isArray(data.series) || !data.series.every(series => object(series) && typeof series.tag === 'string' && series.tag.length > 0
      && isOrderedSpans(series.segments) && (series.membership === null || isOrderedSpans(series.membership)) && isOrderedSpans(series.entries)
      && isOrderedSpans(series.rosters) && series.rosters.every(roster => int(roster.count) && roster.count >= 0 && roster.count <= 0xffff_ffff)
      && series.entries.every(entry => int(entry.readHour) && entry.readHour <= entry.firstHour && value(entry.value)))
    || new Set(data.series.map(series => series.tag)).size !== data.series.length
    || !Array.isArray(data.warnings) || !data.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('The agency comparison reply is malformed.');
  }
  const reply = data as unknown as AgencyComparisonReply;
  const rosterReads = new Set(reply.operativeReadHours);
  for (const series of reply.series) {
    if (!spansCovered(series.entries, reply.observed)) throw new Error('The agency comparison reply crosses unobserved hours.');
    if (!spansCovered(series.entries, series.segments) || !spansCovered(series.rosters, series.segments)
      || series.rosters.some(roster => !rosterReads.has(roster.firstHour))) throw new Error('The agency comparison reply has an undated or unresolved roster.');
    for (const entry of series.entries) for (const item of entry.value.upgrades.items) {
      const original = item.icon;
      item.icon = drawableIcon(original);
      if (original !== null && item.icon === null) reply.warnings.push(`Artwork refused for agency upgrade ${item.branchToken}/${item.token}: not a drawable bounded PNG (1)`);
    }
  }
  return reply;
}
export interface AgencyRow {
  key: string; tag: string; hour: number; value: AgencyValue;
  operativeRosterCount: number | null; operativeReadHour: number | null;
}
export function agencyRequestParams(branchId: string, selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number,
  includeUncreated: boolean, field: AgencyRankField): AgencyComparisonParams | null {
  const countries = worldRequestTags(selection, fallback, groups, lastHour);
  if (!countries.ready) return null;
  const count = selection.kind === 'group' && selection.group.type === 'top' ? selection.group.count : null;
  return { branchId, tags: countries.tags, includeUncreated, top: count === null ? null : { count, field } };
}
/** Hourly agency scalars are independently read at every hour in an entry. Missing
 * rows stay absent, even where another country's entry or a prior value exists. */
export function agencyRows(reply: AgencyComparisonReply, hour: number, countries: ReadonlySet<string> | null): AgencyRow[] {
  if (!runAt(reply.observed, hour)) return [];
  const rows: AgencyRow[] = [];
  for (const series of reply.series) {
    if (countries && !countries.has(series.tag)) continue;
    if (!runAt(series.segments, hour) || (series.membership !== null && !runAt(series.membership, hour))) continue;
    const entry = runAt(series.entries, hour);
    if (entry) {
      const roster = runAt(series.rosters, hour);
      rows.push({ key: series.tag, tag: series.tag, hour, value: entry.value,
        operativeRosterCount: roster?.count ?? null, operativeReadHour: roster ? readAt(reply.operativeReadHours, hour) : null });
    }
  }
  return rows;
}
/** The column reads one adapter result; a future capture fills this without
 * mistaking work on the current upgrade for acquired upgrades. */
export const upgradesCell = (value: AgencyValue): string => value.upgrades.count === null ? 'Not recorded' : formatFixed(BigInt(value.upgrades.count), 0, 0);
export const operativesCell = (value: AgencyValue): string => `${formatFixed(BigInt(value.activeCount), 0, 0)} / ${formatFixed(BigInt(value.maximum), 0, 0)}`;
export function agencyTooltip(row: AgencyRow, name: string, upgradesUnrecorded: string | null): TooltipContent {
  const value = row.value;
  return { title: name, subtitle: value.name === null ? 'Agency name unknown' : value.name, rows: [
    { label: 'Agency', value: value.created ? 'Created' : 'Not created' },
    { label: 'Defence', value: value.defenseText ?? 'Unknown' },
    { label: 'Operatives active / maximum', value: operativesCell(value) },
    { label: 'Agency upgrades', value: upgradesCell(value) },
    ...(value.upgrades.count === null && upgradesUnrecorded ? [{ label: upgradesUnrecorded, tone: 'muted' as const }] : []),
    ...value.upgrades.items.map(item => ({ label: item.name ?? `Unknown upgrade (${item.branchToken}/${item.token})`, icon: item.icon ?? undefined })),
    { label: 'Recorded active-list entries', value: row.operativeRosterCount === null ? 'Not recorded' : String(row.operativeRosterCount) },
    { label: 'Active-list read', value: row.operativeReadHour === null ? 'Not recorded' : formatDateTime(row.operativeReadHour) },
  ], note: `Agency-screen values read ${formatDateTime(row.hour)}. The daily active list is separately dated and does not replace the screen’s active count.` };
}
