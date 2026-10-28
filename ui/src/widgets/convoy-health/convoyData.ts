import type { ConvoyHealthReply, EconomyRun } from '../../generated/dashboard';
import type { HeatPeriod, HeatRow } from '../../charts/heat/layout';
import { object } from '../../dashboard/validation.ts';
import { fixedText, isEconomyRuns, isHolderMap, runAt, tagAt, validateEconomyReply } from '../../dashboard/economyHistory.ts';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts.ts';
import type { NameAt } from '../../dashboard/countryFacts';

export const CONVOY_HEALTH = 'convoy-health';
export type ConvoyView = 'regions' | 'trade';
const DAY = 24;

export function validateConvoys(view: ConvoyView) {
  return (value: unknown): ConvoyHealthReply => validateEconomyReply<ConvoyHealthReply>(value, reply => reply.view === view
    && Number.isSafeInteger(reply.scale) && isHolderMap(reply.holders)
    && Array.isArray(reply.known) && reply.known.length === 3 && reply.known.every(runs => isEconomyRuns(runs))
    && Array.isArray(reply.regions) && reply.regions.every(row => object(row) && Number.isSafeInteger(row.region)
      && (row.name === null || typeof row.name === 'string') && isEconomyRuns(row.efficiency))
    && Array.isArray(reply.deals) && reply.deals.every(row => object(row) && typeof row.id === 'string'
      && ['exporter', 'resource', 'efficiency'].every(key => isEconomyRuns(row[key])) && Array.isArray(row.factors) && row.factors.every(runs => isEconomyRuns(runs))
      && Array.isArray(row.convoys) && row.convoys.every(runs => isEconomyRuns(runs)))
    && Array.isArray(reply.leases) && reply.leases.every(row => object(row) && typeof row.handle === 'string' && isEconomyRuns(row.giver)
      && isEconomyRuns(row.bad) && Array.isArray(row.convoys) && row.convoys.every(runs => isEconomyRuns(runs)))
    && Array.isArray(reply.resources), 'convoy health');
}

/** Calendar days over `extent`, each read at its last recorded hour; a day the
 * recording missed whole is a gap (null read). */
type Gaps = readonly { firstHour: number; lastHour: number }[];

export function dayPeriods(extent: readonly [number, number], gaps: Gaps): HeatPeriod[] {
  const periods: HeatPeriod[] = [];
  for (let start = Math.floor(extent[0] / DAY) * DAY; start <= extent[1]; start += DAY) {
    const first = Math.max(start, extent[0]), last = Math.min(start + DAY - 1, extent[1]);
    periods.push({ firstHour: first, lastHour: last, readHour: lastRead(first, last, gaps) });
  }
  return periods;
}

/** The latest recorded hour from `first` through `last`; null when the recording
 * missed them all. */
function lastRead(first: number, last: number, gaps: Gaps): number | null {
  let read: number | null = last;
  while (read !== null && read >= first) {
    const gap = gaps.find(span => span.firstHour <= read! && read! <= span.lastHour);
    if (!gap) break;
    read = gap.firstHour - 1 >= first ? gap.firstHour - 1 : null;
  }
  return read !== null && read >= first ? read : null;
}

const within = (spans: Gaps, hour: number) => spans.some(span => span.firstHour <= hour && hour <= span.lastHour);

/** The periods as shown at `reached` with what preparation has accepted. The day
 * under way reads the latest recorded hour reached, and a read preparation has not
 * accepted yet (`pending`) moves back to the period's latest prepared hour, so a
 * partly prepared day shows its prepared part. A period with nothing prepared has no
 * read: the chart draws the pending band there, never a gap or a cell, and keys skip
 * it. Cells, tooltips, picks and the drill all use these reads. Only the periods
 * whose read changes are copied; the rest are shared. */
export function shownPeriods(periods: readonly HeatPeriod[], gaps: Gaps, reached: number | null, pending: Gaps): readonly HeatPeriod[] {
  let shown: HeatPeriod[] | null = null;
  const set = (index: number, readHour: number | null) => {
    const current = (shown ?? periods)[index];
    if (current.readHour !== readHour) (shown ??= periods.slice())[index] = { ...current, readHour };
  };
  if (reached !== null) {
    const index = periodIndex(periods, reached);
    const read = index === null ? null : periods[index].readHour;
    if (index !== null && read !== null && read > reached) set(index, lastRead(periods[index].firstHour, reached, gaps));
  }
  if (pending.length) {
    const excluded = [...gaps, ...pending];
    const first = Math.min(...pending.map(span => span.firstHour));
    for (let index = periodIndex(periods, first, true) ?? periods.length; index < periods.length; index++) {
      const { firstHour, readHour } = (shown ?? periods)[index];
      if (readHour !== null && within(pending, readHour)) set(index, lastRead(firstHour, readHour, excluded));
    }
  }
  return shown ?? periods;
}

/** The period holding `hour`; with `after`, else the first period after it. */
function periodIndex(periods: readonly HeatPeriod[], hour: number, after = false): number | null {
  let low = 0, high = periods.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (periods[middle].lastHour < hour) low = middle + 1; else high = middle;
  }
  return low < periods.length && (after || periods[low].firstHour <= hour) ? low : null;
}

/** The hours still being prepared over `extent`: the reply's pending spans and the
 * tail after its last prepared hour, in order. */
export function convoyPending(reply: { lastHour: number | null }, pending: Gaps, extent: readonly [number, number]): { firstHour: number; lastHour: number }[] {
  const spans = pending.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour }));
  const from = Math.max(reply.lastHour === null ? extent[0] : reply.lastHour + 1, (spans.at(-1)?.lastHour ?? -Infinity) + 1);
  if (from <= extent[1]) spans.push({ firstHour: from, lastHour: extent[1] });
  return spans;
}

/** Whether a period without a read is still being prepared rather than missed. */
export const periodPending = (period: HeatPeriod, pending: Gaps) =>
  pending.some(span => span.firstHour <= period.lastHour && period.firstHour <= span.lastHour);

/** One heat row: what it shows, and its value at each period's read. */
export interface ConvoyRow {
  id: string;
  label: string;
  kind: 'region' | 'deal' | 'lease';
  /** Fixed5 efficiency at a read; null where none was read. */
  valueAt: (hour: number) => bigint | null;
  /** Unknown at a read: the row's table was not read complete then, or (for
   * lend-lease receipts) no numeric efficiency is recorded. */
  unknownAt: (hour: number) => boolean;
  detail: (hour: number) => { label: string; value: string }[];
  /** The row's own history for its drill: percentages, or convoy counts. */
  lines: { label: string; runs: readonly EconomyRun[] }[];
  unit: 'percent' | 'count';
}

const percent = (raw: bigint, scale: number) => `${fixedText(raw * 100n, BigInt(scale), 2)}%`;
const big = (runs: readonly EconomyRun[], hour: number) => { const run = runAt(runs, hour); return run ? BigInt(run[2]) : null; };
const convoysText = (convoys: readonly (readonly EconomyRun[])[], hour: number) => {
  const assigned = big(convoys[0] ?? [], hour), required = big(convoys[1] ?? [], hour);
  return assigned === null && required === null ? [] : [{ label: 'Convoys assigned', value: `${assigned ?? '—'} of ${required ?? '—'}` }];
};

/** Each country label a row shows, with the dated read it names it at: the
 * hour the widget asks names for, so no other date is ever looked up. Every
 * date is kept, however many rows there are. */
export function labelDates(reply: ConvoyHealthReply): { wanted: NameAt[]; maxHours: number } {
  const wanted: NameAt[] = [];
  for (const runs of [...reply.deals.map(deal => deal.exporter), ...reply.leases.map(lease => lease.giver)]) {
    const latest = runs.at(-1);
    const tag = latest ? tagAt(reply.holders[latest[2]], latest[1]) : null;
    if (latest && tag) wanted.push({ hour: latest[1], tag });
  }
  return { wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size) };
}

/** Whether table `index` (regional, deals, clients) was read complete at `hour`. */
export const tableKnown = (reply: ConvoyHealthReply, index: number, hour: number) => runAt(reply.known[index] ?? [], hour) !== null;

export function convoyRows(reply: ConvoyHealthReply, names: { country: (tag: string | null, hour: number) => string }): ConvoyRow[] {
  const resources = new Map(reply.resources.map(resource => [String(resource.id), resource.name ?? 'Resource name unavailable']));
  if (reply.view === 'regions') return reply.regions.map(region => ({ id: `region:${region.region}`, kind: 'region' as const,
    label: region.name ?? `Sea region ${region.region}`, valueAt: hour => big(region.efficiency, hour),
    unknownAt: hour => !tableKnown(reply, 0, hour),
    lines: [{ label: 'Convoy efficiency', runs: region.efficiency }], unit: 'percent' as const,
    detail: hour => { const value = big(region.efficiency, hour); return value === null ? [] : [{ label: 'Convoy efficiency', value: percent(value, reply.scale) }]; } }));
  const deals: ConvoyRow[] = reply.deals.map(deal => {
    const latest = deal.exporter.at(-1), resource = deal.resource.at(-1);
    const exporter = latest ? tagAt(reply.holders[latest[2]], latest[1]) : null;
    // The exporter is named at its latest read, the date the widget asks for.
    const exporterName = names.country(exporter, latest?.[1] ?? 0);
    return { id: `deal:${deal.id}`, kind: 'deal' as const,
      label: `${exporterName} · ${resource ? resources.get(resource[2]) ?? 'Resource name unavailable' : 'Resource name unavailable'}`,
      valueAt: hour => big(deal.efficiency, hour), unknownAt: hour => !tableKnown(reply, 1, hour),
      lines: [{ label: 'Trade efficiency', runs: deal.efficiency }, { label: 'First factor', runs: deal.factors[0] ?? [] }, { label: 'Second factor', runs: deal.factors[1] ?? [] }],
      unit: 'percent' as const,
      detail: hour => {
        const value = big(deal.efficiency, hour);
        if (value === null) return [];
        const [a, b] = deal.factors.map(runs => big(runs, hour));
        return [{ label: 'Exporter', value: runAt(deal.exporter, hour) ? exporterName : 'Country name unavailable' },
          { label: 'Trade efficiency', value: percent(value, reply.scale) },
          ...a !== null && b !== null ? [{ label: 'Factors', value: `${percent(a, reply.scale)} × ${percent(b, reply.scale)}` }] : [],
          ...convoysText(deal.convoys, hour)];
      } };
  });
  const leases: ConvoyRow[] = reply.leases.map(lease => {
    const latest = lease.giver.at(-1);
    return { id: `lease:${lease.handle}`, kind: 'lease' as const,
      label: `Lend-lease from ${names.country(latest ? tagAt(reply.holders[latest[2]], latest[1]) : null, latest?.[1] ?? 0)}`,
      // Every recorded client has a route flag, even while its giver is unidentified.
      valueAt: () => null, unknownAt: hour => runAt(lease.bad, hour) !== null || !tableKnown(reply, 2, hour),
      lines: [{ label: 'Convoys assigned', runs: lease.convoys[0] ?? [] }, { label: 'Convoys required', runs: lease.convoys[1] ?? [] }], unit: 'count' as const,
      detail: hour => runAt(lease.bad, hour) === null ? [] : [{ label: 'Efficiency', value: 'Not recorded' },
        { label: 'Route', value: big(lease.bad, hour) === 1n ? 'Drawn as a bad route' : 'Not drawn as a bad route' },
        ...convoysText(lease.convoys, hour)] };
  });
  return [...deals, ...leases];
}

/** The heat rows over [reachedPeriods]: each period's cell is its read's share,
 * blank where nothing was read, hatched where it is not recorded (lend-lease). The
 * most active rows (most days read through `through`) come first, at most `count`. */
export function heatRows(rows: readonly ConvoyRow[], periods: readonly HeatPeriod[], color: string, count: number, through: number | null = null): HeatRow[] {
  const cells = rows.map(row => ({ row, cells: periods.map(period => {
    const read = period.readHour;
    if (read === null) return { parts: [] };
    if (row.unknownAt(read)) return { parts: [], unknown: true };
    const value = row.valueAt(read);
    if (value === null) return { parts: [] };
    const share = Math.max(0, Math.min(1, Number(value) / 100000));
    return { parts: [{ id: row.id, label: row.label, color, share }] };
  }) }));
  // Ranks by the reads reached (the day under way at its reached read), so later
  // days never pick the rows.
  const ranked = cells.map(entry => {
    let active = 0;
    for (let index = 0; index < periods.length && index < entry.cells.length; index++) {
      const read = periods[index].readHour, cell = entry.cells[index];
      if (read !== null && (through === null || read <= through) && (cell.parts.length || cell.unknown)) active++;
    }
    return { ...entry, active };
  });
  return ranked.sort((a, b) => b.active - a.active || a.row.label.localeCompare(b.row.label)).slice(0, count)
    .filter(entry => entry.active > 0).map(entry => ({ id: entry.row.id, label: entry.row.label, cells: entry.cells }));
}
