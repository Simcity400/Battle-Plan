import type { GatedHour } from '../../dashboard/regionTime';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { Sort } from '../../charts/table/logic';
import { CELL, cellState, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { OptionValue } from '../../dashboard/options';
import { int, object } from '../../dashboard/validation.ts';
import type { CountryRef, ScoreboardColumn, ScoreboardColumnKey, ScoreboardColumnsReply, ScoreboardParams, ScoreboardReply } from '../../generated/dashboard';
import { DEFAULT_SORT } from './definition.ts';

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
  if (!columns.length || (top != null && !rank)) return null;
  const keys = columns.map(column => column.key);
  return top == null ? { branchId, hour, columns: keys, countries: tags.map(countryFromKey) }
    : { branchId, hour, columns: keys, countries: [], top: { count: top, sort: rank!.key, descending } };
}

export const partialSource = (key: ScoreboardColumnKey) => `${key}:partial`;

export function rankedTags(answer: FollowedAnswer | undefined): string[] {
  if (!answer || answer.kind === 'notRecorded') return [];
  const out: string[] = [];
  for (let row = 0; row < answer.rows; row++) {
    const tag = rowName(answer, row);
    if (tag && cellState(answer, row * answer.columns) === CELL.value) out.push(tag);
  }
  return out;
}

export function scoreboardColumnNote(column: ScoreboardColumn): string | undefined {
  if (!column.available) return column.reason ?? 'Unavailable';
  return column.note;
}
