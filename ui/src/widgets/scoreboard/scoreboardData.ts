import { isOrderedSpans, runAt, spansCovered } from '../../charts/series.ts';
import { sortRows } from '../../charts/table/logic.ts';
import type { Sort } from '../../charts/table/logic';
import { formatAmount } from '../../charts/valueAxis.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { OptionValue } from '../../dashboard/options';
import { readNote, readRunAt } from '../../dashboard/reads.ts';
import { countryIdentityKey } from '../../dashboard/countryIdentity.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { int, object } from '../../dashboard/validation.ts';
import type { ResolvedCountries } from '../../dashboard/groups';
import type { ScoreboardColumn, ScoreboardColumnKey, ScoreboardReply } from '../../generated/dashboard';
import { DEFAULT_SORT } from './definition.ts';

/** Only the backend registry owns measures and their content-derived labels. */
export function validateScoreboardReply(value: unknown): ScoreboardReply {
  if (!object(value) || typeof value.branchId !== 'string' || !Array.isArray(value.columns) || !Array.isArray(value.series)
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')) throw new Error('Invalid Scoreboard reply.');
  const keys = new Set<string>();
  for (const column of value.columns) {
    if (!object(column) || typeof column.key !== 'string' || keys.has(column.key) || typeof column.label !== 'string'
      || typeof column.unit !== 'string' || !int(column.decimals) || column.decimals < 0 || column.decimals > 18
      || typeof column.available !== 'boolean' || !(column.reason === null || typeof column.reason === 'string')
      || !(column.note === undefined || typeof column.note === 'string')) throw new Error('Invalid Scoreboard column.');
    keys.add(column.key);
  }
  if (!Array.isArray(value.defaultColumns) || !value.defaultColumns.every(key => typeof key === 'string' && keys.has(key))
    || typeof value.defaultSort !== 'string' || !keys.has(value.defaultSort)
    || !value.columns.some(column => object(column) && column.key === value.defaultSort && column.available === true)) throw new Error('Invalid Scoreboard defaults.');
  const tags = new Set<string>();
  for (const series of value.series) {
    if (!object(series) || typeof series.tag !== 'string' || tags.has(series.tag) || !isOrderedSpans(series.segments)
      || !(series.membership === null || isOrderedSpans(series.membership)) || !isOrderedSpans(series.identities)
      || !series.identities.every(identity => int(identity.slot) && identity.slot >= 0 && int(identity.countryId) && identity.countryId >= 0)
      || !spansCovered(series.identities, series.segments) || !Array.isArray(series.columns)) throw new Error('Invalid Scoreboard country.');
    tags.add(series.tag);
    const present = new Set<string>();
    for (const column of series.columns) {
      if (!object(column) || typeof column.key !== 'string' || !keys.has(column.key) || present.has(column.key)
        || !isOrderedSpans(column.runs) || !isOrderedSpans(column.readRuns)
        || !spansCovered(column.runs, series.segments) || !spansCovered(column.readRuns, series.segments)
        || !column.runs.every(run => (run.amount === null || (typeof run.amount === 'string' && /^-?\d{1,39}$/.test(run.amount)
          && BigInt(run.amount) >= -(1n << 127n) && BigInt(run.amount) < (1n << 127n))) && typeof run.startsSegment === 'boolean'
          && (run.atLeast === undefined || typeof run.atLeast === 'boolean'))
        || !column.readRuns.every(read => int(read.readHour) && read.readHour <= read.firstHour && typeof read.hourly === 'boolean'
          && (!read.hourly || read.readHour === read.firstHour))) throw new Error('Invalid Scoreboard values.');
      present.add(column.key);
    }
  }
  return value as unknown as ScoreboardReply;
}

export interface ScoreboardCell { amount: bigint | null; readHour: number | null; atLeast: boolean }
export interface ScoreboardRow { key: string; tag: string; hour: number; cells: ReadonlyMap<string, ScoreboardCell> }
export function scoreboardModel(reply: ScoreboardReply) {
  return {
    reply, tags: reply.series.map(series => series.tag),
    series: new Map(reply.series.map(series => [series.tag, {
      ...series, columns: new Map(series.columns.map(column => [column.key, { ...column,
        runs: column.runs.map(run => ({ ...run, amount: run.amount === null ? null : BigInt(run.amount) })),
      }])),
    }])),
  };
}
export type ScoreboardModel = ReturnType<typeof scoreboardModel>;

export function selectedColumns(registry: readonly ScoreboardColumn[], stored: OptionValue, defaults: readonly ScoreboardColumnKey[]): ScoreboardColumn[] {
  const selected = contentSet(stored).map(contentKey);
  const keys = selected.length ? selected : defaults;
  return keys.flatMap(key => { const column = registry.find(column => column.key === key); return column ? [column] : []; });
}

/** Unknown stored columns fall back honestly; unavailable columns remain sortable. */
export function scoreboardSort(registry: readonly ScoreboardColumn[], shown: readonly ScoreboardColumn[], requested: Sort, defaultColumn: ScoreboardColumnKey | ''): Sort {
  if (requested.column === 'country' || shown.some(column => column.key === requested.column)) return requested;
  const fallback = registry.find(column => column.key === defaultColumn && shown.includes(column)) ?? shown.find(column => column.available);
  return { column: fallback?.key ?? 'country', direction: DEFAULT_SORT.direction };
}

/** Country and unavailable-column sorting retain an available numerical top N. */
export function rankingSort(registry: readonly ScoreboardColumn[], sort: Sort, stored: OptionValue, defaultColumn: ScoreboardColumnKey | ''): Sort & { column: ScoreboardColumnKey } {
  const sorted = registry.find(column => column.key === sort.column && column.available);
  if (sorted) return { column: sorted.key, direction: sort.direction };
  const key = typeof stored === 'string' && stored ? contentKey(stored) : defaultColumn;
  const available = registry.find(column => column.key === key && column.available)
    ?? registry.find(column => column.key === defaultColumn && column.available)
    ?? registry.find(column => column.available);
  return { column: (available?.key ?? defaultColumn) as ScoreboardColumnKey, direction: DEFAULT_SORT.direction };
}

export function scoreboardRequest(branchId: string, resolved: ResolvedCountries, columns: readonly ScoreboardColumn[], rank: ScoreboardColumnKey, direction: Sort['direction']) {
  const requested = [...new Set([...columns.map(column => column.key), ...(resolved.top != null ? [rank] : [])])];
  return { branchId, columns: requested, tags: resolved.top == null ? resolved.tags : [],
    top: resolved.top == null ? null : { count: resolved.top, sort: rank, descending: direction === 'descending' } };
}

export function scoreboardRows(model: ScoreboardModel, tags: readonly string[], hour: number, rank: ScoreboardColumnKey, direction: Sort['direction'], top?: number): ScoreboardRow[] {
  const rows: ScoreboardRow[] = [];
  for (const tag of tags) {
    const series = model.series.get(tag);
    if (!series || !runAt(series.segments, hour) || (series.membership !== null && !runAt(series.membership, hour))) continue;
    const identity = runAt(series.identities, hour);
    if (!identity) continue;
    const cells = new Map<string, ScoreboardCell>();
    for (const [key, column] of series.columns) {
      const run = runAt(column.runs, hour);
      const readHour = readRunAt(column.readRuns, hour);
      cells.set(key, { amount: readHour === null ? null : run?.amount ?? null, readHour, atLeast: readHour !== null && run?.atLeast === true });
    }
    rows.push({ key: countryIdentityKey(identity), tag, hour, cells });
  }
  const ordered = sortRows(rows, row => row.cells.get(rank)?.amount, direction, row => row.key, row => row.tag);
  return top == null ? ordered : ordered.slice(0, top);
}

export function formatCell(column: ScoreboardColumn, amount: bigint | null, atLeast = false): string {
  if (amount === null) return '—';
  const percent = column.unit === 'percent';
  return `${atLeast ? '† ' : ''}${formatAmount(percent ? amount * 100n : amount, column.decimals, column.decimals)}${percent ? '%' : ''}`;
}

export function scoreboardTooltip(row: ScoreboardRow, columns: readonly ScoreboardColumn[], name: string) {
  return {
    title: name, subtitle: formatDateTime(row.hour),
    rows: columns.flatMap(column => {
      const cell = row.cells.get(column.key);
      const value = formatCell(column, cell?.amount ?? null, cell?.atLeast);
      const note = !column.available ? column.reason ?? 'Not available yet'
        : cell?.amount === null || !cell ? cell?.readHour == null ? 'Not recorded at this hour' : 'Not known from this recording at this hour'
          : readNote(cell.readHour, row.hour, `${column.label} follows its recorded read cadence.`);
      return [{ label: column.label, value }, ...(note ? [{ label: note, tone: 'muted' as const }] : []),
        ...(cell?.amount != null && cell.atLeast ? [{ label: 'Recorded subtotal (†): sorting uses this recorded amount. Missing history may omit positive losses or negative corrections.', tone: 'muted' as const }] : []),
        ...(column.note ? [{ label: column.note, tone: 'muted' as const }] : [])];
    }),
  };
}
