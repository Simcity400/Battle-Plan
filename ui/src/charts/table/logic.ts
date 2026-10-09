import type { MovingRow } from '../rowMotion';
import { compareTags } from '../../dashboard/tagOrder.ts';

/** Exact ordering: geometry may use numbers, comparisons never round integers. */
export type SortValue = string | number | bigint | null | undefined;
export type SortDirection = 'ascending' | 'descending';
export interface Sort { column: string; direction: SortDirection }

export const isMissing = (v: SortValue) => v == null || (typeof v === 'number' && !Number.isFinite(v));
const compareKey = compareTags;
const valueCollator = new Intl.Collator(undefined, { numeric: true });
/** Equal values always order by key, in either direction and across input updates. */
export function sortRows<T>(rows: readonly T[], value: (row: T) => SortValue, direction: SortDirection, rowKey: (row: T) => string,
  tieKey: (row: T) => string = rowKey): T[] {
  const present: { row: T; key: string; value: NonNullable<SortValue>; numeric: number }[] = [];
  const absent: { row: T; key: string }[] = [];
  let numerical = true;
  for (const row of rows) {
    const v = value(row);
    const key = tieKey(row);
    if (isMissing(v)) absent.push({ row, key }); else {
      numerical &&= typeof v !== 'string';
      present.push({ row, key, value: v!, numeric: typeof v === 'string' ? 0 : Number(v) });
    }
  }
  const sign = direction === 'ascending' ? 1 : -1;
  // Partition missing values once to keep the comparison path small.
  // Number conversion is monotone. Distinct approximate keys preserve exact
  // integer order; ties (including overflow) always compare the original value.
  present.sort(numerical
    ? (a, b) => a.numeric < b.numeric ? -sign : a.numeric > b.numeric ? sign : a.value < b.value ? -sign : a.value > b.value ? sign : compareKey(a.key, b.key)
    : (a, b) => sign * valueCollator.compare(String(a.value), String(b.value)) || compareKey(a.key, b.key));
  absent.sort((a, b) => compareKey(a.key, b.key));
  return present.map(item => item.row).concat(absent.map(item => item.row));
}

/** A dated group key is its identity; equal labels do not merge groups. */
export interface TableGroup { key: string; label: string; order?: number; orderKey?: bigint }

/** Groups stay in label order while each uses the selected exact column sort. */
export function groupedRows<T>(rows: readonly T[], groupBy: (row: T) => TableGroup,
  value: (row: T) => SortValue, direction: SortDirection, rowKey: (row: T) => string,
  tieKey: (row: T) => string = rowKey): T[] {
  const groups = new Map<string, { group: TableGroup; rows: T[] }>();
  for (const row of rows) {
    const group = groupBy(row);
    const present = groups.get(group.key);
    if (present) present.rows.push(row);
    else groups.set(group.key, { group, rows: [row] });
  }
  return [...groups.values()].sort(({ group: a }, { group: b }) => {
    const order = (a.order ?? 0) - (b.order ?? 0) || valueCollator.compare(a.label, b.label);
    if (order) return order;
    const aKey = a.orderKey ?? 0n;
    const bKey = b.orderKey ?? 0n;
    return aKey < bKey ? -1 : aKey > bKey ? 1 : compareKey(a.key, b.key);
  }).flatMap(group => sortRows(group.rows, value, direction, rowKey, tieKey));
}

export const nextSort = (sort: Sort, column: string, first: SortDirection = 'descending', fixed?: SortDirection): Sort => ({ column,
  direction: fixed ?? (column === sort.column ? (sort.direction === 'ascending' ? 'descending' : 'ascending') : first) });

/** A bar's fraction of a positive maximum. Only its width uses floating point. */
export function barFraction(value: number | bigint | null | undefined, maximum: number | bigint): number {
  if (value == null || (typeof value === 'number' && !Number.isFinite(value))
    || (typeof maximum === 'number' && !Number.isFinite(maximum)) || maximum <= 0 || value <= 0) return 0;
  return Math.min(1, Number(value) / Number(maximum));
}

export function barMaximum<T>(rows: readonly T[], value: (row: T) => SortValue): number | bigint {
  let maximum: number | bigint = 0;
  for (const row of rows) {
    const v = value(row);
    if ((typeof v === 'bigint' || (typeof v === 'number' && Number.isFinite(v))) && v > maximum) maximum = v;
  }
  return maximum;
}

export interface RankChange { rank: number; delta: number; until: number }
export function rankChanges(previous: ReadonlyMap<string, RankChange>, keys: readonly string[], now: number, hold = 2500, reset = false): Map<string, RankChange> {
  const changes = new Map<string, RankChange>();
  for (let rank = 0; rank < keys.length; rank++) {
    const key = keys[rank];
    const before = reset ? undefined : previous.get(key);
    const delta = before == null ? 0 : before.rank - rank;
    changes.set(key, delta ? { rank, delta, until: now + hold }
      : { rank, delta: before && before.until > now ? before.delta : 0, until: before?.until ?? 0 });
  }
  return changes;
}

/** Scale a table's layout-pixel height by the density ratio measured from its CSS. */
export function densityRowHeight(height: number, density: 'comfortable' | 'compact', compactRatio: number): number {
  return height * (density === 'compact' ? compactRatio : 1);
}

/** Absolute item offsets, including the sticky header before the first row. */
export function rowOffsets(count: number, rowHeight: number, headerHeight = 0): number[] {
  return Array.from({ length: count + 1 }, (_, slot) => headerHeight + slot * rowHeight);
}

// Same contracts as the log kit's logData.ts; kept local until both kits land.
/** [start, end) covering top..top+height, widened by overscan items each way. */
export function visibleRange(offsets: readonly number[], top: number, height: number, overscan = 6): [number, number] {
  const count = offsets.length - 1;
  if (count <= 0) return [0, 0];
  let low = 0;
  let high = count - 1;
  while (low < high) {
    const middle = (low + high + 1) >> 1;
    if (offsets[middle] <= top) low = middle; else high = middle - 1;
  }
  let end = low;
  while (end < count && offsets[end] < top + height) end++;
  return [Math.max(0, low - overscan), Math.min(count, end + overscan)];
}

/** Only viewport rows and keep(row); leaving rows vanish immediately without animation. */
export function shownMotion<T>(rows: readonly MovingRow<T>[], start: number, end: number, animate: boolean,
  keep: (row: MovingRow<T>) => boolean = () => false): MovingRow<T>[] {
  return rows.filter(row => (animate || row.phase !== 'leaving') && ((row.slot >= start && row.slot < end) || keep(row)));
}

/** Show the whole row, moving as little as possible and clearing the sticky header. */
export function scrollToShow(offsets: readonly number[], index: number, top: number, height: number, stickyTop = 0): number {
  const start = offsets[index];
  const end = offsets[index + 1];
  if (start - stickyTop < top) return Math.max(0, start - stickyTop);
  if (end > top + height) return end - height;
  return top;
}
