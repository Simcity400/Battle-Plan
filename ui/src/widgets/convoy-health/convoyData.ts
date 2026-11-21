import type { HeatPeriod, HeatRow } from '../../charts/heat/layout';
import type { NameAt } from '../../dashboard/countryFacts';
import { object } from '../../dashboard/validation.ts';
import type { ConvoyCountry, ConvoyHealthReply, ConvoyView, HourSpan } from '../../generated/dashboard';
import type { ConvoyRun } from './types';

export const CONVOY_HEALTH = 'convoy-health';
const DAY = 24;

const isRuns = (value: unknown): value is ConvoyRun[] => Array.isArray(value)
  && value.every(run => Array.isArray(run) && run.length === 3 && run.every(Number.isSafeInteger) && run[0] <= run[1]);
const isRunLists = (value: unknown) => Array.isArray(value) && value.every(isRuns);
const isSpans = (value: unknown): value is HourSpan[] => Array.isArray(value)
  && value.every(span => object(span) && Number.isSafeInteger(span.firstHour) && Number.isSafeInteger(span.lastHour));
const isCountries = (value: unknown): value is ConvoyCountry[] => Array.isArray(value)
  && value.every(run => object(run) && Number.isSafeInteger(run.firstHour) && Number.isSafeInteger(run.lastHour) && (run.tag === null || typeof run.tag === 'string'));
const isText = (value: unknown) => value === null || typeof value === 'string';

export function validateConvoys(view: ConvoyView) {
  return (value: unknown): ConvoyHealthReply => {
    const reply = value as Record<string, unknown>;
    if (!object(value) || reply.view !== view || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string'
      || !Number.isSafeInteger(reply.scale) || (reply.scale as number) <= 0 || !Number.isSafeInteger(reply.firstHour) || !Number.isSafeInteger(reply.lastHour)
      || !Array.isArray(reply.known) || reply.known.length !== 3 || !reply.known.every(isSpans) || !isSpans(reply.unheld)
      || !Array.isArray(reply.regions) || !reply.regions.every(row => object(row) && Number.isSafeInteger(row.region) && isText(row.name) && isText(row.missing) && isRuns(row.efficiency))
      || !Array.isArray(reply.deals) || !reply.deals.every(row => object(row) && typeof row.id === 'string' && isCountries(row.exporter)
        && isRuns(row.resource) && isRuns(row.efficiency) && isRunLists(row.factors) && isRunLists(row.convoys))
      || !Array.isArray(reply.leases) || !reply.leases.every(row => object(row) && typeof row.handle === 'string' && isCountries(row.giver)
        && isRuns(row.bad) && isRunLists(row.convoys))
      || !Array.isArray(reply.resources) || !reply.resources.every(row => object(row) && Number.isSafeInteger(row.id) && typeof row.token === 'string' && isText(row.name))
      || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string')) throw new Error('Invalid convoy health reply');
    return value as unknown as ConvoyHealthReply;
  };
}

type Spans = readonly { firstHour: number; lastHour: number }[];

const within = (spans: Spans, hour: number) => {
  let low = 0, high = spans.length;
  while (low < high) { const middle = (low + high) >> 1; if (spans[middle].lastHour < hour) low = middle + 1; else high = middle; }
  return low < spans.length && spans[low].firstHour <= hour;
};

export function dayPeriods(extent: readonly [number, number], gaps: Spans): HeatPeriod[] {
  const periods: HeatPeriod[] = [];
  for (let start = Math.floor(extent[0] / DAY) * DAY; start <= extent[1]; start += DAY) {
    const first = Math.max(start, extent[0]), last = Math.min(start + DAY - 1, extent[1]);
    let read: number | null = last;
    while (read !== null && read >= first) {
      const hour: number = read;
      const gap = gaps.find(span => span.firstHour <= hour && hour <= span.lastHour);
      if (!gap) break;
      read = gap.firstHour - 1 >= first ? gap.firstHour - 1 : null;
    }
    periods.push({ firstHour: first, lastHour: last, readHour: read !== null && read >= first ? read : null });
  }
  return periods;
}

export function runAt(runs: readonly ConvoyRun[], hour: number): ConvoyRun | null {
  let low = 0, high = runs.length;
  while (low < high) { const middle = (low + high) >> 1; if (runs[middle][1] < hour) low = middle + 1; else high = middle; }
  return low < runs.length && runs[low][0] <= hour ? runs[low] : null;
}

function countryRunAt(runs: readonly ConvoyCountry[], hour: number): ConvoyCountry | null {
  let low = 0, high = runs.length;
  while (low < high) { const middle = (low + high) >> 1; if (runs[middle].lastHour < hour) low = middle + 1; else high = middle; }
  return low < runs.length && runs[low].firstHour <= hour ? runs[low] : null;
}

const valueAt = (runs: readonly ConvoyRun[] | undefined, hour: number) => runs ? runAt(runs, hour)?.[2] ?? null : null;

export const tableKnown = (reply: ConvoyHealthReply, index: number, hour: number) => within(reply.known[index] ?? [], hour);

export const countryAt = (runs: readonly ConvoyCountry[], hour: number) => countryRunAt(runs, hour)?.tag ?? null;

export const percent = (raw: number, scale: number) => `${(Math.trunc(raw * 10000 / scale) / 100).toFixed(2)}%`;

export interface ConvoyFact { label: string; value: string; detail?: string }
export interface CountryName { name: string; nameFailure?: Error; nameDetail?: string }
export type CountryNameAt = (tag: string | null, hour: number) => CountryName;

export interface ConvoyRow {
  id: string;
  label: string;
  nameDetail?: string;
  kind: 'region' | 'deal' | 'lease';
  valueAt: (hour: number) => number | null;
  unknownAt: (hour: number) => boolean;
  facts: (hour: number) => ConvoyFact[];
  labels: readonly string[];
  lines: { label: string; runs: readonly ConvoyRun[] }[];
  unit: 'percent' | 'count';
}

const convoysFact = (convoys: readonly (readonly ConvoyRun[])[], hour: number): ConvoyFact[] => {
  const assigned = valueAt(convoys[0], hour), required = valueAt(convoys[1], hour);
  return assigned === null && required === null ? [] : [{ label: 'Convoys assigned', value: `${assigned ?? '—'} of ${required ?? '—'}` }];
};

const latest = (runs: readonly ConvoyCountry[]) => { const run = runs.at(-1); return run ? { tag: run.tag, hour: run.lastHour } : { tag: null, hour: 0 }; };

export function labelDates(reply: ConvoyHealthReply): NameAt[] {
  const wanted: NameAt[] = [];
  for (const runs of [...reply.deals.map(deal => deal.exporter), ...reply.leases.map(lease => lease.giver)])
    for (const run of runs) if (run.tag !== null) wanted.push({ hour: run.lastHour, tag: run.tag });
  return wanted;
}

const datedCountry = (runs: readonly ConvoyCountry[], hour: number, names: CountryNameAt) => {
  const run = countryRunAt(runs, hour);
  return run ? names(run.tag, run.lastHour) : null;
};

export function convoyRows(reply: ConvoyHealthReply, names: CountryNameAt): ConvoyRow[] {
  const resources = new Map(reply.resources.map(resource => [resource.id, resource.name ?? 'Resource name unavailable']));
  if (reply.view === 'regions') return reply.regions.map(region => ({ id: `region:${region.region}`, kind: 'region' as const,
    label: region.name ?? `Sea region ${region.region}`, ...(region.missing ? { nameDetail: region.missing } : {}),
    valueAt: hour => valueAt(region.efficiency, hour), unknownAt: hour => !tableKnown(reply, 0, hour),
    lines: [{ label: 'Convoy efficiency', runs: region.efficiency }], unit: 'percent' as const, labels: ['Convoy efficiency'],
    facts: hour => { const value = valueAt(region.efficiency, hour); return value === null ? [] : [{ label: 'Convoy efficiency', value: percent(value, reply.scale) }]; } }));
  const deals: ConvoyRow[] = reply.deals.map(deal => {
    const exporter = latest(deal.exporter), resource = deal.resource.at(-1);
    const exporterName = names(exporter.tag, exporter.hour);
    return { id: `deal:${deal.id}`, kind: 'deal' as const,
      ...(exporterName.nameFailure ? { nameDetail: exporterName.nameFailure.message } : {}),
      label: `${exporterName.name} · ${resource ? resources.get(resource[2]) ?? 'Resource name unavailable' : 'Resource name unavailable'}`,
      valueAt: hour => valueAt(deal.efficiency, hour), unknownAt: hour => !tableKnown(reply, 1, hour),
      lines: [{ label: 'Trade efficiency', runs: deal.efficiency }, { label: 'First factor', runs: deal.factors[0] ?? [] }, { label: 'Second factor', runs: deal.factors[1] ?? [] }],
      unit: 'percent' as const, labels: ['Exporter', 'Trade efficiency', 'Factors', 'Convoys assigned'],
      facts: hour => {
        const value = valueAt(deal.efficiency, hour);
        if (value === null) return [];
        const [a, b] = [valueAt(deal.factors[0], hour), valueAt(deal.factors[1], hour)];
        const country = datedCountry(deal.exporter, hour, names);
        return [{ label: 'Exporter', value: country?.name ?? 'Country name unavailable', ...(country?.nameDetail ? { detail: country.nameDetail } : {}) },
          { label: 'Trade efficiency', value: percent(value, reply.scale) },
          ...a !== null && b !== null ? [{ label: 'Factors', value: `${percent(a, reply.scale)} × ${percent(b, reply.scale)}` }] : [],
          ...convoysFact(deal.convoys, hour)];
      } };
  });
  const leases: ConvoyRow[] = reply.leases.map(lease => {
    const giver = latest(lease.giver);
    const country = names(giver.tag, giver.hour);
    return { id: `lease:${lease.handle}`, kind: 'lease' as const,
      ...(country.nameFailure ? { nameDetail: country.nameFailure.message } : {}), label: `Lend-lease from ${country.name}`,
      valueAt: () => null, unknownAt: hour => runAt(lease.bad, hour) !== null || !tableKnown(reply, 2, hour),
      lines: [{ label: 'Convoys assigned', runs: lease.convoys[0] ?? [] }, { label: 'Convoys required', runs: lease.convoys[1] ?? [] }], unit: 'count' as const,
      labels: ['Efficiency', 'Route', 'Convoys assigned'],
      facts: hour => runAt(lease.bad, hour) === null ? [] : [{ label: 'Efficiency', value: 'Not recorded' },
        { label: 'Route', value: valueAt(lease.bad, hour) === 1 ? 'Drawn as a bad route' : 'Not drawn as a bad route' },
        ...convoysFact(lease.convoys, hour)] };
  });
  return [...deals, ...leases];
}

export function heatRows(rows: readonly ConvoyRow[], periods: readonly HeatPeriod[], color: string, count: number, scale: number): HeatRow[] {
  const cells = rows.map(row => {
    let active = 0;
    const made = periods.map(period => {
      const read = period.readHour;
      if (read === null) return { parts: [] };
      if (row.unknownAt(read)) { active++; return { parts: [], unknown: true }; }
      const value = row.valueAt(read);
      if (value === null) return { parts: [] };
      active++;
      return { parts: [{ id: row.id, label: row.label, color, share: Math.max(0, Math.min(1, value / scale)) }] };
    });
    return { row, cells: made, active };
  });
  return cells.sort((a, b) => b.active - a.active || a.row.label.localeCompare(b.row.label)).slice(0, count)
    .filter(entry => entry.active > 0).map(entry => ({ id: entry.row.id, label: entry.row.label, cells: entry.cells }));
}

export function factsOrReason(row: ConvoyRow | undefined, period: HeatPeriod | null, reply: ConvoyHealthReply): { facts: ConvoyFact[]; reason: string | null } {
  const read = period?.readHour ?? null;
  if (read === null) return { facts: [], reason: 'Not recorded' };
  if (within(reply.unheld, read)) return { facts: [], reason: 'The country was not held then' };
  const facts = row ? row.facts(read) : [];
  if (facts.length) return { facts, reason: null };
  if (!row || row.unknownAt(read)) return { facts: [], reason: 'Not recorded' };
  return { facts: [], reason: row.kind === 'region' ? 'No bad convoy route here' : 'No deal this day' };
}
