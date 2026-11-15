import type { Sort } from '../../charts/table/logic';
import { noValues } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { TooltipContent } from '../../charts/tooltip';
import { formatAmount } from '../../charts/valueAxis.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { CELL, cellState, cellValue, rowName, rowReason } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { OptionValue } from '../../dashboard/options';
import { int, object } from '../../dashboard/validation.ts';
import { DEFAULT_SORT } from './definition.ts';
import type { ScoreboardColumn, ScoreboardColumnKey, ScoreboardColumnsReply, ScoreboardParams, ScoreboardReply } from '../../generated/dashboard';

const family = (value: unknown) => value === null || (object(value) && typeof value.kind === 'string' && typeof value.measure === 'string');

/** The registry: only the service names the columns, their labels and the measures they read. */
export function validateColumnsReply(value: unknown): ScoreboardColumnsReply {
  if (!object(value) || typeof value.branchId !== 'string' || !Array.isArray(value.columns) || !Array.isArray(value.warnings)
    || !value.warnings.every(warning => typeof warning === 'string')) throw new Error('Invalid Scoreboard columns.');
  const keys = new Set<string>();
  for (const column of value.columns) {
    if (!object(column) || typeof column.key !== 'string' || keys.has(column.key) || typeof column.label !== 'string'
      || typeof column.unit !== 'string' || !int(column.decimals) || column.decimals < 0 || column.decimals > 18
      || typeof column.available !== 'boolean' || !(column.reason === null || typeof column.reason === 'string')
      || !(column.note === undefined || typeof column.note === 'string') || !family(column.family) || !family(column.partial)
      || column.available !== (column.family !== null)) throw new Error('Invalid Scoreboard column.');
    keys.add(column.key);
  }
  if (!Array.isArray(value.defaultColumns) || !value.defaultColumns.every(key => typeof key === 'string' && keys.has(key))
    || typeof value.defaultSort !== 'string' || !keys.has(value.defaultSort)) throw new Error('Invalid Scoreboard defaults.');
  return value as unknown as ScoreboardColumnsReply;
}

/** A reply at an hour: every row holds one cell per column asked, each a value or why it has none. */
export function validateScoreboardReply(value: unknown, columns: number): ScoreboardReply {
  if (!object(value) || typeof value.branchId !== 'string' || !int(value.hour) || !(value.missing === null || typeof value.missing === 'string')
    || !Array.isArray(value.rows) || !Array.isArray(value.warnings)) throw new Error('Invalid Scoreboard reply.');
  const tags = new Set<string>();
  for (const row of value.rows) {
    if (!object(row) || typeof row.tag !== 'string' || tags.has(row.tag) || !Array.isArray(row.cells) || row.cells.length !== columns
      || !row.cells.every(cell => object(cell) && typeof cell.partial === 'boolean'
        && (typeof cell.valueRaw === 'string' ? /^-?\d{1,39}$/.test(cell.valueRaw) && cell.missing === null : cell.valueRaw === null && typeof cell.missing === 'string')))
      throw new Error('Invalid Scoreboard row.');
    tags.add(row.tag);
  }
  return value as unknown as ScoreboardReply;
}

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

/** The measure a top N ranks by: the sorted column when it is one, else the chosen measure, else the default. */
export function rankingColumn(registry: readonly ScoreboardColumn[], sort: Sort, stored: OptionValue, defaultColumn: ScoreboardColumnKey | ''): ScoreboardColumn | undefined {
  const sorted = registry.find(column => column.key === sort.column && column.available);
  if (sorted) return sorted;
  const key = typeof stored === 'string' && stored ? contentKey(stored) : defaultColumn;
  return registry.find(column => column.key === key && column.available)
    ?? registry.find(column => column.key === defaultColumn && column.available)
    ?? registry.find(column => column.available);
}

/** Whether a top is the greatest: the sorted column's direction when it ranks by it, else the greatest. */
export const rankingDescending = (sort: Sort, rank: ScoreboardColumn | undefined) => !rank || sort.column !== rank.key || sort.direction === 'descending';

/** The request at `hour`: the columns shown, of the chosen countries or of the top `top` by `rank`. */
export function scoreboardParams(branchId: string, hour: number, columns: readonly ScoreboardColumn[], tags: readonly string[],
  top: number | null, rank: ScoreboardColumn | undefined, descending = true): ScoreboardParams | null {
  if (!columns.length || (top == null ? !tags.length : !rank)) return null;
  const keys = columns.map(column => column.key);
  return top == null ? { branchId, hour, columns: keys, tags: [...tags] }
    : { branchId, hour, columns: keys, tags: [], top: { count: top, sort: rank!.key, descending } };
}

/** A country's value of a column: its raw amount, or why there is none. */
export interface Cell { amount: bigint | null; missing: string | null; partial: boolean }
/** A row is a country, keyed by its tag; its cells come from a reply, or are painted from followed values. */
export interface BoardRow { key: string; tag: string; hour: number; cells: ReadonlyMap<string, Cell> }

const NO_CELLS: ReadonlyMap<string, Cell> = new Map();

/** A reply's rows, their cells by the keys asked. */
export function replyRows(reply: ScoreboardReply, asked: readonly ScoreboardColumnKey[]): BoardRow[] {
  return reply.rows.map(row => ({ key: row.tag, tag: row.tag, hour: reply.hour,
    cells: new Map(asked.map((key, at) => {
      const cell = row.cells[at]!;
      return [key, { amount: cell.valueRaw === null ? null : BigInt(cell.valueRaw), missing: cell.missing, partial: cell.partial }];
    })) }));
}

/** A followed row: its values are painted, never held. */
export const followedRow = (tag: string, hour: number): BoardRow => ({ key: tag, tag, hour, cells: NO_CELLS });

/** A followed `series-state` answer's raw values by tag; a country it holds no value for is missing for its reason. */
export function followedValues(answer: FollowedAnswer | undefined): LiveValues<bigint> {
  if (!answer) return noValues('Not read yet');
  if (answer.kind === 'notRecorded') return noValues(answer.reason);
  const rows = new Map<string, number>();
  for (let row = 0; row < answer.rows; row++) {
    const tag = rowName(answer, row);
    if (tag !== null && !rows.has(tag)) rows.set(tag, row);
  }
  return {
    get: tag => {
      const row = rows.get(tag);
      return row !== undefined && cellState(answer, row * answer.columns) === CELL.value ? cellValue(answer, row * answer.columns) : undefined;
    },
    missing: tag => {
      const row = rows.get(tag);
      if (row === undefined) return 'Not in the answer';
      return rowReason(answer, row) ?? (cellState(answer, row * answer.columns) === CELL.missing ? 'No value' : 'Not recorded at this hour');
    },
  };
}

/** The countries a ranked answer names, greatest first. */
export function rankedTags(answer: FollowedAnswer | undefined): string[] {
  if (!answer || answer.kind === 'notRecorded') return [];
  const out: string[] = [];
  for (let row = 0; row < answer.rows; row++) {
    const tag = rowName(answer, row);
    if (tag && cellState(answer, row * answer.columns) === CELL.value) out.push(tag);
  }
  return out;
}

/** A partial mark's value: a subtotal wherever it is known and not zero. */
export const isPartial = (mark: bigint | undefined) => mark !== undefined && mark !== 0n;

export function formatCell(column: Pick<ScoreboardColumn, 'unit' | 'decimals'>, amount: bigint | null, partial = false): string {
  if (amount === null) return '—';
  const percent = column.unit === 'percent';
  return `${partial ? '† ' : ''}${formatAmount(percent ? amount * 100n : amount, column.decimals, column.decimals)}${percent ? '%' : ''}`;
}

export function scoreboardColumnNote(column: ScoreboardColumn): string | undefined {
  if (!column.available) return column.reason ?? 'Unavailable';
  return column.note;
}

/** A row's tooltip: every column's value at its hour, with why a value is missing and which are partial subtotals. */
export function scoreboardTooltip(cell: (key: ScoreboardColumnKey) => Cell, columns: readonly ScoreboardColumn[], name: string, hour: number): TooltipContent {
  return {
    title: name, subtitle: formatDateTime(hour),
    rows: columns.flatMap(column => {
      const value = cell(column.key);
      const note = !column.available ? 'Unavailable' : value.amount === null ? value.missing ?? 'Not available at this time' : undefined;
      const description = scoreboardColumnNote(column);
      return [{ label: column.label, value: formatCell(column, value.amount, value.partial) }, ...(note ? [{ label: note, tone: 'muted' as const }] : []),
        ...(value.amount !== null && value.partial ? [{ label: '† Partial total; some losses or corrections may be missing.', tone: 'muted' as const }] : []),
        ...(description && description !== note ? [{ label: description, tone: 'muted' as const }] : [])];
    }),
  };
}
