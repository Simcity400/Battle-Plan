import type { GatedHour } from '../../dashboard/regionTime';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { Sort } from '../../charts/table/logic';
import { liveKey, valuesOf } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { TooltipContent } from '../../charts/tooltip';
import { formatAmount } from '../../charts/valueAxis.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { CELL, cellState, cellValue, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { CountryRef } from '../../generated/dashboard';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { OptionValue } from '../../dashboard/options';
import { int, object } from '../../dashboard/validation.ts';
import { DEFAULT_SORT } from './definition.ts';
import { namedValues } from '../../draw/tables/sources.ts';
import type { TableCellModel, TableReading } from '../../draw/tables/surface';
import type { ScoreboardColumn, ScoreboardColumnKey, ScoreboardColumnsReply, ScoreboardParams, ScoreboardReply } from '../../generated/dashboard';

const family = (value: unknown) => value === null || (object(value) && typeof value.kind === 'string' && typeof value.measure === 'string');

export function validateColumnsReply(value: unknown): ScoreboardColumnsReply {
  if (!object(value) || typeof value.branchId !== 'string' || !Array.isArray(value.columns) || !Array.isArray(value.warnings)
    || !value.warnings.every(warning => typeof warning === 'string')) throw new Error('Invalid Scoreboard columns.');
  const keys = new Set<string>();
  for (const column of value.columns) {
    if (!object(column) || typeof column.key !== 'string' || keys.has(column.key) || typeof column.label !== 'string'
      || typeof column.unit !== 'string' || !int(column.decimals) || column.decimals < 0 || column.decimals > 18
      || typeof column.available !== 'boolean' || !(column.reason === null || typeof column.reason === 'string')
      || !(column.note === undefined || typeof column.note === 'string') || !family(column.family) || !family(column.partial)
      || column.available !== (column.family !== null) || typeof column.noneAtZero !== 'boolean') throw new Error('Invalid Scoreboard column.');
    keys.add(column.key);
  }
  if (!Array.isArray(value.defaultColumns) || !value.defaultColumns.every(key => typeof key === 'string' && keys.has(key))
    || typeof value.defaultSort !== 'string' || !keys.has(value.defaultSort)) throw new Error('Invalid Scoreboard defaults.');
  return value as unknown as ScoreboardColumnsReply;
}

export function validateScoreboardReply(value: unknown, columns: number): ScoreboardReply {
  if (!object(value) || typeof value.branchId !== 'string' || !int(value.hour) || !(value.missing === null || typeof value.missing === 'string')
    || !Array.isArray(value.rows) || !Array.isArray(value.warnings)) throw new Error('Invalid Scoreboard reply.');
  const countries = new Set<string>();
  for (const row of value.rows) {
    if (!object(row) || typeof row.tag !== 'string' || !(typeof row.country === 'string' || int(row.country))
      || countries.has(countryKey(row.country as CountryRef)) || !Array.isArray(row.cells) || row.cells.length !== columns
      || !row.cells.every(cell => object(cell) && typeof cell.partial === 'boolean'
        && (typeof cell.valueRaw === 'string' ? /^-?\d{1,39}$/.test(cell.valueRaw) && cell.missing === null : cell.valueRaw === null && typeof cell.missing === 'string')))
      throw new Error('Invalid Scoreboard row.');
    countries.add(countryKey(row.country as CountryRef));
  }
  return value as unknown as ScoreboardReply;
}

export function selectedColumns(registry: readonly ScoreboardColumn[], stored: OptionValue, defaults: readonly ScoreboardColumnKey[]): ScoreboardColumn[] {
  const selected = contentSet(stored).map(contentKey);
  const keys = selected.length ? selected : defaults;
  return keys.flatMap(key => { const column = registry.find(column => column.key === key); return column ? [column] : []; });
}

export function scoreboardSort(registry: readonly ScoreboardColumn[], shown: readonly ScoreboardColumn[], requested: Sort, defaultColumn: ScoreboardColumnKey | ''): Sort {
  if (requested.column === 'country' || shown.some(column => column.key === requested.column)) return requested;
  const fallback = registry.find(column => column.key === defaultColumn && shown.includes(column)) ?? shown.find(column => column.available);
  return { column: fallback?.key ?? 'country', direction: DEFAULT_SORT.direction };
}

export function rankingColumn(registry: readonly ScoreboardColumn[], sort: Sort, stored: OptionValue, defaultColumn: ScoreboardColumnKey | ''): ScoreboardColumn | undefined {
  const sorted = registry.find(column => column.key === sort.column && column.available);
  if (sorted) return sorted;
  const key = typeof stored === 'string' && stored ? contentKey(stored) : defaultColumn;
  return registry.find(column => column.key === key && column.available)
    ?? registry.find(column => column.key === defaultColumn && column.available)
    ?? registry.find(column => column.available);
}

export const rankingDescending = (sort: Sort, rank: ScoreboardColumn | undefined) => !rank || sort.column !== rank.key || sort.direction === 'descending';

export function scoreboardParams(branchId: string, hour: GatedHour, columns: readonly ScoreboardColumn[], tags: readonly string[],
  top: number | null, rank: ScoreboardColumn | undefined, descending = true): (Omit<ScoreboardParams, 'hour'> & { hour: GatedHour }) | null {
  if (!columns.length || (top == null ? !tags.length : !rank)) return null;
  const keys = columns.map(column => column.key);
  return top == null ? { branchId, hour, columns: keys, countries: tags.map(countryFromKey) }
    : { branchId, hour, columns: keys, countries: [], top: { count: top, sort: rank!.key, descending } };
}

export interface Cell { amount: bigint | null; missing: string | null; partial: boolean }
export interface BoardRow { key: string; tag: string; hour: number; cells: ReadonlyMap<string, Cell> }

const NO_CELLS: ReadonlyMap<string, Cell> = new Map();

export function replyRows(reply: ScoreboardReply, asked: readonly ScoreboardColumnKey[]): BoardRow[] {
  return reply.rows.map(row => ({ key: countryKey(row.country), tag: row.tag, hour: reply.hour,
    cells: new Map(asked.map((key, at) => {
      const cell = row.cells[at]!;
      return [key, { amount: cell.valueRaw === null ? null : BigInt(cell.valueRaw), missing: cell.missing, partial: cell.partial }];
    })) }));
}

export const followedRow = (tag: string, hour: number): BoardRow => ({ key: tag, tag, hour, cells: NO_CELLS });

export const followedValues = (answer: FollowedAnswer | undefined): LiveValues<bigint> => namedValues(answer, cellValue);

export const partialSource = (key: ScoreboardColumnKey) => `${key}:partial`;

export function measureCell(column: ScoreboardColumn, tag: string): TableCellModel {
  return { valueKey: liveKey(tag, column.key), format: { decimals: column.decimals, percent: column.unit === 'percent', zero: column.noneAtZero ? 'None' : undefined },
    prefix: { key: liveKey(tag, partialSource(column.key)), text: '\u2020 ' }, ...(!column.family && { missing: column.reason ?? 'Unavailable' }) };
}

export function drawnCell(drawn: Pick<TableReading, 'values' | 'missing'>, tag: string, key: ScoreboardColumnKey): Cell {
  const amount = drawn.values[liveKey(tag, key)], mark = drawn.values[liveKey(tag, partialSource(key))];
  if (amount === undefined) return { amount: null, missing: drawn.missing[liveKey(tag, key)] ?? null, partial: false };
  return { amount: BigInt(amount), missing: null, partial: mark !== undefined && isPartial(BigInt(mark)) };
}

export function replyValues(rows: readonly BoardRow[], columns: readonly ScoreboardColumn[]): LiveValues<bigint> {
  const values = new Map<string, bigint>(), reasons = new Map<string, string>();
  for (const row of rows) for (const column of columns) {
    const cell = row.cells.get(column.key), key = liveKey(row.tag, column.key);
    if (cell?.amount == null) { reasons.set(key, cell?.missing ?? column.reason ?? 'Not available at this time'); continue; }
    values.set(key, cell.amount);
    values.set(liveKey(row.tag, partialSource(column.key)), cell.partial ? 1n : 0n);
  }
  return valuesOf(values, key => reasons.get(key) ?? 'Not in the reply');
}

export function rankedTags(answer: FollowedAnswer | undefined): string[] {
  if (!answer || answer.kind === 'notRecorded') return [];
  const out: string[] = [];
  for (let row = 0; row < answer.rows; row++) {
    const tag = rowName(answer, row);
    if (tag && cellState(answer, row * answer.columns) === CELL.value) out.push(tag);
  }
  return out;
}

export const isPartial = (mark: bigint | undefined) => mark !== undefined && mark !== 0n;

export function formatCell(column: Pick<ScoreboardColumn, 'unit' | 'decimals'> & { noneAtZero?: boolean }, amount: bigint | null, partial = false): string {
  if (amount === null) return '—';
  if (amount === 0n && column.noneAtZero) return `${partial ? '† ' : ''}None`;
  const percent = column.unit === 'percent';
  return `${partial ? '† ' : ''}${formatAmount(percent ? amount * 100n : amount, column.decimals, column.decimals)}${percent ? '%' : ''}`;
}

export function scoreboardColumnNote(column: ScoreboardColumn): string | undefined {
  if (!column.available) return column.reason ?? 'Unavailable';
  return column.note;
}

export function scoreboardTooltip(cell: (key: ScoreboardColumnKey) => Cell, columns: readonly ScoreboardColumn[], name: string, hour: number): TooltipContent {
  return {
    title: name, subtitle: formatDateTime(hour),
    rows: columns.map(column => {
      const value = cell(column.key);
      if (value.amount !== null) return { label: column.label, value: formatCell(column, value.amount, value.partial) };
      return { label: column.label, value: !column.available ? column.reason ?? 'Unavailable' : value.missing ?? 'Not recorded', tone: 'muted' as const };
    }),
    ...(columns.some(column => { const value = cell(column.key); return value.amount !== null && value.partial; }) && { note: '† Partial total' }),
  };
}
