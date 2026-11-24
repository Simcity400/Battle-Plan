import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryNameDetails } from '../../charts/tooltip.ts';
import { formatDateTime, isEngineHour } from '../../dashboard/calendar.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { contentKey, contentSet, setToken } from '../../dashboard/options.ts';
import { object } from '../../dashboard/validation.ts';
import { placeKey } from '../../dashboard/placeNamesData.ts';
import { newestPageRows } from '../../charts/log/logPages.ts';
import type { LogPage, LogRow } from '../../charts/log/logData';
import type { LogCounts, LogReplyRow } from '../../charts/log/logSource';
import type { TooltipContent } from '../../charts/tooltip';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { CountryRef, DivisionLogManifestReply, LogCell, PlaceName } from '../../generated/dashboard';

export const KINDS = { created: 0, lost: 1, transfer: 2 } as const;
export type DivisionKind = keyof typeof KINDS;
const KIND_NAMES: readonly DivisionKind[] = ['created', 'lost', 'transfer'];

export interface DivisionLogEntry {
  division: string; kind: DivisionKind; status: string;
  readHour: number;
  hour: number; by: boolean; afterHour: number | null;
  tag: string | null; name: string | null; nameMissing: string | null; icon: string | null; iconMissing: string | null;
  presentationHour: number | null; template: string | null; templateName: string | null; templateMissing: string | null;
  province: number | null; placeHour: number | null; placeAtEvent: boolean;
  killerTag: string | null; killerDivision: string | null; matchedDeaths: number | null;
}

const number = (cell: LogCell | undefined) => cell?.kind === 'number' ? Number(cell.value) : null;
const text = (cell: LogCell | undefined) => cell?.kind === 'text' ? cell.value : null;

export function divisionEntry(row: LogReplyRow): LogRow<DivisionLogEntry> {
  const cells = row.cells;
  const readHour = number(cells.readHour) ?? row.hour;
  const item: DivisionLogEntry = {
    division: text(cells.division) ?? row.key, kind: KIND_NAMES[number(cells.kind) ?? 1] ?? 'lost', status: text(cells.status) ?? 'unknown',
    readHour, hour: number(cells.hour) ?? readHour, by: number(cells.by) !== 0, afterHour: number(cells.afterHour),
    tag: text(cells.tag), name: text(cells.name), nameMissing: text(cells.nameMissing), icon: text(cells.icon), iconMissing: text(cells.iconMissing),
    presentationHour: number(cells.presentationHour), template: text(cells.template), templateName: text(cells.templateName),
    templateMissing: text(cells.templateMissing), province: number(cells.province), placeHour: number(cells.placeHour),
    placeAtEvent: number(cells.placeAtEvent) === 1, killerTag: text(cells.killerTag), killerDivision: text(cells.killerDivision),
    matchedDeaths: number(cells.matchedDeaths),
  };
  return { key: row.key, hour: readHour, by: false, item };
}

const hour = (v: unknown): v is number => typeof v === 'number' && isEngineHour(v);
const id = (v: unknown) => typeof v === 'string' && /^[0-9]+$/u.test(v);
const texts = (v: unknown) => Array.isArray(v) && v.every(w => typeof w === 'string');
export function validateDivisionManifest(v: unknown): DivisionLogManifestReply {
  if (!object(v) || typeof v.available !== 'boolean' || typeof v.lossesAvailable !== 'boolean' || !texts(v.warnings)
    || !Array.isArray(v.gaps) || !v.gaps.every(gap => object(gap) && hour(gap.firstHour) && hour(gap.lastHour) && gap.firstHour <= gap.lastHour && typeof gap.reason === 'string')
    || !Array.isArray(v.templates) || !v.templates.every(t => object(t) && id(t.id) && (t.name === null || typeof t.name === 'string'))
    || new Set(v.templates.map(t => t.id)).size !== v.templates.length)
    throw new Error('The division log manifest is malformed.');
  return v as unknown as DivisionLogManifestReply;
}

export const templateToken = (campaign: string, branch: string, template: string) => setToken(campaignToken(campaign, `${branch}:${template}`));
export function selectedTemplates(value: string, campaign: string, branch: string): string[] | null {
  const chosen = contentSet(value);
  if (!chosen.length) return null;
  const prefix = campaignToken(campaign, `${branch}:`);
  return chosen.map(contentKey).filter(token => token.startsWith(prefix)).map(token => token.slice(prefix.length));
}

export const shownKinds = (events: string): DivisionKind[] => events === 'created' ? ['created'] : events === 'lost' ? ['lost', 'transfer'] : ['created', 'lost', 'transfer'];

export type DivisionFacet = { name: string; values: number[]; countries: CountryRef[] };

export function divisionFacets(kinds: readonly DivisionKind[], templates: readonly string[] | null, tags: readonly string[] | null): DivisionFacet[] {
  const facets: DivisionFacet[] = [{ name: 'kind', values: kinds.map(kind => KINDS[kind]), countries: [] }];
  if (templates !== null) facets.push({ name: 'template', values: templates.map(Number), countries: [] });
  if (tags !== null) facets.push({ name: 'slot', values: [], countries: tags.map(countryFromKey) });
  return facets;
}

export function kindTotals(list: LogCounts, kinds: ReadonlyMap<DivisionKind, LogCounts>, pages: readonly LogPage<DivisionLogEntry>[],
  shown: number, until: number | null): Map<DivisionKind, number> | null {
  const out = new Map<DivisionKind, number>();
  const bin = until === null ? list.counts.length : Math.floor((until - list.firstHour) / list.binHours);
  const through = (counts: LogCounts, end: number) => {
    let total = 0;
    for (let at = 0; at < Math.min(end, counts.counts.length); at++) total += counts.counts[at];
    return total;
  };
  for (const [kind, counts] of kinds) out.set(kind, bin >= counts.counts.length ? counts.total : bin < 0 ? 0 : through(counts, bin));
  if (bin < 0 || bin >= list.counts.length) return out;
  const first = through(list, bin);
  for (let at = first; at < shown; at++) {
    const page = pages.find(held => held.start <= at && at <= held.end);
    const row = page?.rows?.[at - page.start];
    if (!row) return null;
    const kind = row.item.kind;
    if (out.has(kind)) out.set(kind, out.get(kind)! + 1);
  }
  return out;
}

export function gapWarnings(manifest: DivisionLogManifestReply, first: number, last: number): string[] {
  return manifest.gaps.filter(gap => gap.lastHour >= first && gap.firstHour <= last)
    .map(gap => `${gap.reason}: ${formatDateTime(Math.max(first, gap.firstHour))} – ${formatDateTime(Math.min(last, gap.lastHour))}`);
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
export const divisionName = (row: DivisionLogEntry) => row.name ?? row.nameMissing ?? 'Division name not recorded';
export function divisionTooltip(row: DivisionLogEntry, country: ReturnType<CountryLabel>, killer: ReturnType<CountryLabel>, place: string): TooltipContent {
  return { title: divisionName(row), icon: row.icon ?? undefined, subtitle: divisionStatus(row), rows: [
    { ...countryNameDetails(country), label: 'Country', value: country.name, countryFlag: row.tag === null ? undefined
      : { flag: country.flag, flagReason: country.flagReason, ...(country.flagDetail !== undefined ? { flagDetail: country.flagDetail } : {}), flagPending: country.flagPending } },
    { label: 'Template', value: row.templateName ?? row.templateMissing ?? 'Template name not recorded' },
    { label: row.placeAtEvent ? 'Location' : 'Last seen at', value: place },
    { label: 'When', value: `${row.by ? 'By ' : ''}${formatDateTime(row.hour)}` },
    ...(row.readHour !== row.hour ? [{ label: 'Recorded', value: formatDateTime(row.readHour) }] : []),
    ...(row.afterHour === null ? [] : [{ label: 'First appeared after', value: formatDateTime(row.afterHour) }]),
    ...(row.kind === 'created' ? [] : [{ ...countryNameDetails(killer), label: 'Killer', value: killer.name, countryFlag: row.killerTag === null ? undefined
      : { flag: killer.flag, flagReason: killer.flagReason, ...(killer.flagDetail !== undefined ? { flagDetail: killer.flagDetail } : {}), flagPending: killer.flagPending } }]),
  ] };
}

export interface DivisionPlaceRead { pending: boolean; failed: boolean }

export const PLACE_LOADING = 'Loading place…';

export function divisionPlace(row: DivisionLogEntry, known: ReadonlyMap<string, PlaceName>, read: DivisionPlaceRead): string {
  if (row.province === null || row.placeHour === null) return 'Location not recorded';
  return divisionPlaceText(known.get(placeKey(row.province, row.placeHour)), read);
}

export function divisionPlaceText(place: PlaceName | undefined, read: DivisionPlaceRead): string {
  if (!place) return read.failed ? 'Place name couldn’t load' : read.pending ? PLACE_LOADING : 'Unnamed province (the names reply left it out)';
  return place.name ?? place.missing ?? 'Unnamed province';
}

export function divisionNamedRows(follow: boolean, pages: readonly LogPage<DivisionLogEntry>[], newest: number,
  drawn: readonly LogRow<DivisionLogEntry>[]): readonly LogRow<DivisionLogEntry>[] {
  if (!follow) return drawn;
  const held = newestPageRows(pages, newest), keys = new Set(held.map(row => row.key));
  const beyond = drawn.filter(row => !keys.has(row.key));
  return beyond.length ? [...held, ...beyond] : held;
}

export function divisionPlaces(rows: readonly LogRow<DivisionLogEntry>[], at: number | null = null): { provinceId: number; hour: number }[] {
  const ordered = at === null ? rows : [...rows.filter(row => row.hour <= at).sort((a, b) => b.hour - a.hour), ...rows.filter(row => row.hour > at)];
  const wanted = new Map<string, { provinceId: number; hour: number }>();
  for (const { item } of ordered) {
    if (item.province === null || item.placeHour === null) continue;
    const key = placeKey(item.province, item.placeHour);
    if (!wanted.has(key)) wanted.set(key, { provinceId: item.province, hour: item.placeHour });
  }
  return [...wanted.values()];
}
