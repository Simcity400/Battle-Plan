import { isEngineHour, formatDateTime } from '../../dashboard/calendar.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { contentKey, contentSet, setToken } from '../../dashboard/options.ts';
import { int, object } from '../../dashboard/validation.ts';
import { newestFirst } from '../../charts/log/logData.ts';
import type { LogRow } from '../../charts/log/logData';
import type { TooltipContent } from '../../charts/tooltip';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { DivisionLogEntry, DivisionLogReply, DivisionLogManifestReply, DivisionLogTotalsReply } from '../../generated/dashboard';

const hour = (v: unknown): v is number => typeof v === 'number' && isEngineHour(v);
const text = (v: unknown) => v === null || typeof v === 'string';
const at = (v: unknown, read: number) => v === null || hour(v) && v <= read;
const id = (v: unknown) => typeof v === 'string' && /^[0-9]+$/u.test(v);
const warnings = (v: unknown) => Array.isArray(v) && v.every(w => typeof w === 'string');
const span = (v: unknown) => object(v) && hour(v.firstHour) && hour(v.lastHour) && v.firstHour <= v.lastHour;
function entry(v: unknown): boolean {
  return object(v) && typeof v.id === 'string' && id(v.division) && hour(v.readHour) && hour(v.hour) && v.hour <= v.readHour
    && typeof v.by === 'boolean' && at(v.afterHour, v.readHour) && (v.afterHour === null || Number(v.afterHour) < v.readHour)
    && ['created', 'lost', 'transfer'].includes(String(v.kind)) && typeof v.status === 'string'
    && text(v.tag) && at(v.countryHour, v.readHour) && v.countryHour !== null && text(v.name) && text(v.nameMissing)
    && text(v.icon) && text(v.iconMissing) && at(v.presentationHour, v.readHour)
    && (v.template === null || id(v.template)) && text(v.templateName) && text(v.templateMissing) && at(v.templateHour, v.readHour)
    && (v.province === null || int(v.province) && v.province >= 0 && v.province <= 4294967295)
    && at(v.placeHour, v.readHour) && typeof v.placeAtEvent === 'boolean'
    && text(v.killerTag) && at(v.killerHour, v.readHour) && (v.killerDivision === null || id(v.killerDivision));
}
export function validateDivisionLog(v: unknown): DivisionLogReply {
  if (!object(v) || !int(v.window) || v.window < 0 || !span(v.interval) || !Array.isArray(v.rows) || !v.rows.every(entry)
    || new Set(v.rows.map(row => row.id)).size !== v.rows.length || !warnings(v.contentWarnings)
    || !Array.isArray(v.warnings) || !v.warnings.every(w => object(w) && hour(w.hour) && typeof w.reason === 'string' && int(w.count) && w.count > 0))
    throw new Error('The division log reply is malformed.');
  return v as unknown as DivisionLogReply;
}
export function validateDivisionManifest(v: unknown): DivisionLogManifestReply {
  if (!object(v) || !Array.isArray(v.windows) || !v.windows.every(span) || typeof v.available !== 'boolean'
    || typeof v.lossesAvailable !== 'boolean' || !warnings(v.contentWarnings) || !Array.isArray(v.templates)
    || !v.templates.every(t => object(t) && id(t.id) && text(t.name)) || new Set(v.templates.map(t => t.id)).size !== v.templates.length)
    throw new Error('The division log manifest is malformed.');
  return v as unknown as DivisionLogManifestReply;
}
export function validateDivisionTotals(v: unknown): DivisionLogTotalsReply {
  if (!object(v) || ![v.created, v.lost, v.transferred].every(n => int(n) && n >= 0) || !warnings(v.warnings))
    throw new Error('The division log totals are malformed.');
  return v as unknown as DivisionLogTotalsReply;
}
export const templateToken = (campaign: string, branch: string, template: string) => setToken(campaignToken(campaign, `${branch}:${template}`));
/** Null selects every type. An unavailable saved choice selects none here. */
export function selectedTemplates(value: string, campaign: string, branch: string): string[] | null {
  const chosen = contentSet(value);
  if (!chosen.length) return null;
  const prefix = campaignToken(campaign, `${branch}:`);
  return chosen.map(contentKey).filter(token => token.startsWith(prefix)).map(token => token.slice(prefix.length));
}
export interface DivisionLogItem extends DivisionLogEntry { windowIndex: number }
export function divisionRows(windows: ReadonlyMap<number, DivisionLogReply>, at: number, first: number,
  events: string, templates: readonly string[] | null, matches: (tag: string, hour: number) => boolean, all: boolean): LogRow<DivisionLogItem>[] {
  const selected = templates === null ? null : new Set(templates);
  const rows: LogRow<DivisionLogItem>[] = [];
  for (const [windowIndex, reply] of [...windows].sort(([a], [b]) => a - b)) for (const row of reply.rows) {
    if (row.readHour > at || row.hour < first || events === 'created' && row.kind !== 'created' || events === 'lost' && row.kind === 'created'
      || selected !== null && (row.template === null || !selected.has(row.template))
      || !all && (row.tag === null || !matches(row.tag, row.countryHour))) continue;
    rows.push({ key: row.id, hour: row.hour, by: row.by, item: { ...row, windowIndex } });
  }
  return newestFirst(rows);
}
export function divisionStatus(row: DivisionLogEntry): string {
  const names: Record<string, string> = {
    first_observed: 'First seen', division_destroyed: 'Destroyed', division_overrun: 'Overrun',
    division_deleted: 'Removed', division_disbanded: 'Disbanded', division_capitulation: 'Removed at capitulation',
    division_scripted: 'Removed by script', division_template_removed: 'Template removed', unknown: 'Removed (cause unknown)',
    returned: 'Returned', merged: 'Merged', hq_transferred: 'HQ transfer', volunteer_transferred: 'Volunteer transfer', exile_transferred: 'Exile transfer',
  };
  return names[row.status] ?? 'Removal cause not known';
}
export const divisionName = (row: DivisionLogEntry) => row.name ?? 'Division name not recorded';
export function divisionTooltip(row: DivisionLogEntry, country: ReturnType<CountryLabel>, killer: ReturnType<CountryLabel>, place: string): TooltipContent {
  return { title: divisionName(row), icon: row.icon ?? undefined, subtitle: divisionStatus(row), rows: [
    { label: 'Country', value: country.name, countryFlag: row.tag === null ? undefined
      : { flag: country.flag, flagReason: country.flagReason, flagPending: country.flagPending } },
    { label: 'Template', value: row.templateName ?? 'Template name not recorded' },
    { label: row.placeAtEvent ? 'Location' : 'Last seen at', value: place },
    { label: 'When', value: `${row.by ? 'By ' : ''}${formatDateTime(row.hour)}` },
    ...(row.afterHour === null ? [] : [{ label: 'First appeared after', value: formatDateTime(row.afterHour) }]),
    ...(row.kind === 'created' ? [] : [{ label: 'Killer', value: killer.name, countryFlag: row.killerTag === null ? undefined
      : { flag: killer.flag, flagReason: killer.flagReason, flagPending: killer.flagPending } }]),
  ] };
}
