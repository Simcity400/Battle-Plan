import type { MovingRow } from '../rowMotion';
import { compareTags } from '../../dashboard/tagOrder.ts';

export type SortValue = string | number | bigint | null | undefined;
export type SortDirection = 'ascending' | 'descending';
export interface Sort { column: string; direction: SortDirection }

export const isMissing = (v: SortValue) => v == null || (typeof v === 'number' && !Number.isFinite(v));
const compareKey = compareTags;
const valueCollator = new Intl.Collator(undefined, { numeric: true });
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
  present.sort(numerical
    ? (a, b) => a.numeric < b.numeric ? -sign : a.numeric > b.numeric ? sign : a.value < b.value ? -sign : a.value > b.value ? sign : compareKey(a.key, b.key)
    : (a, b) => sign * valueCollator.compare(String(a.value), String(b.value)) || compareKey(a.key, b.key));
  absent.sort((a, b) => compareKey(a.key, b.key));
  return present.map(item => item.row).concat(absent.map(item => item.row));
}

export interface TableGroup { key: string; label: string; order?: number; orderKey?: bigint }

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

export function densityRowHeight(height: number, density: 'comfortable' | 'compact', compactRatio: number): number {
  return height * (density === 'compact' ? compactRatio : 1);
}

export function rowOffsets(count: number, rowHeight: number, headerHeight = 0): number[] {
  return Array.from({ length: count + 1 }, (_, slot) => headerHeight + slot * rowHeight);
}

export interface RowMeasurements { scope: string; heights: ReadonlyMap<string, number>; floor: number }

export function withRowHeights(before: RowMeasurements, readings: readonly (readonly [string, number])[], rowHeight: number, scope: string): RowMeasurements {
  if (!readings.length) return before;
  const same = before.scope === scope;
  const lowest = Math.min(...readings.map(([, height]) => height));
  const held = same ? before.floor : 0;
  const floor = !held || lowest < held - 0.5 ? lowest : held;
  const base = Math.max(rowHeight, floor) + 0.5;
  const taller = (height: number | undefined) => height !== undefined && height > base ? height : 0;
  let heights: Map<string, number> | undefined;
  for (const [key, height] of readings) {
    const next = taller(height);
    if (taller(before.heights.get(key)) === next) continue;
    heights ??= new Map(before.heights);
    if (next) heights.set(key, next); else heights.delete(key);
  }
  return heights || !same || floor !== before.floor ? { scope, heights: heights ?? before.heights, floor } : before;
}

export function measuredRowOffsets(keys: readonly string[], heights: ReadonlyMap<string, number>, minimum: number, headerHeight = 0): number[] {
  const offsets = [headerHeight];
  for (const key of keys) offsets.push(offsets[offsets.length - 1] + Math.max(minimum, heights.get(key) ?? minimum));
  return offsets;
}

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

export function shownMotion<T>(rows: readonly MovingRow<T>[], start: number, end: number, animate: boolean,
  keep: (row: MovingRow<T>) => boolean = () => false): MovingRow<T>[] {
  return rows.filter(row => (animate || row.phase !== 'leaving') && ((row.slot >= start && row.slot < end) || keep(row)));
}

export function scrollToShow(offsets: readonly number[], index: number, top: number, height: number, stickyTop = 0): number {
  const start = offsets[index];
  const end = offsets[index + 1];
  if (end - start > height - stickyTop) return Math.max(0, start - stickyTop);
  if (start - stickyTop < top) return Math.max(0, start - stickyTop);
  if (end > top + height) return end - height;
  return top;
}

export interface ColumnFit { readonly hidden: readonly string[]; readonly needs: readonly number[] }
export const FULL_FIT: ColumnFit = { hidden: [], needs: [] };
export function fitColumns(fit: ColumnFit, order: readonly string[], available: number, required: number): ColumnFit {
  let kept = 0;
  while (kept < fit.hidden.length && fit.hidden[kept] === order[kept]) kept++;
  const held = kept === fit.hidden.length ? fit : { hidden: fit.hidden.slice(0, kept), needs: fit.needs.slice(0, kept) };
  if (!(available > 0)) return held;
  if (required > available + 1) return held.hidden.length < order.length
    ? { hidden: [...held.hidden, order[held.hidden.length]], needs: [...held.needs, required] } : held;
  if (held.hidden.length && available >= held.needs[held.needs.length - 1])
    return { hidden: held.hidden.slice(0, -1), needs: held.needs.slice(0, -1) };
  return held;
}

export interface TableTree<T> {
  children: (row: T) => readonly T[] | undefined;
  initiallyOpen?: (row: T) => boolean;
  open?: (row: T) => boolean;
  onToggle?: (row: T, open: boolean) => void;
}

export interface TreeRow { level: number; parent: string | null; group: boolean; open: boolean; position: number; size: number; count: number }

export function treeRows<T>(top: readonly T[], children: TableTree<T>['children'], open: (row: T) => boolean,
  arrange: (rows: readonly T[], level: number) => T[], rowKey: (row: T) => string, everyGroup = false): { rows: T[]; nesting: Map<string, TreeRow> } {
  const rows: T[] = [];
  const nesting = new Map<string, TreeRow>();
  const visit = (list: readonly T[], level: number, parent: string | null) => {
    const arranged = arrange(list, level);
    for (let index = 0; index < arranged.length; index++) {
      const row = arranged[index];
      const key = rowKey(row);
      if (nesting.has(key)) continue;
      const nested = children(row);
      const isOpen = nested !== undefined && open(row);
      nesting.set(key, { level, parent, group: nested !== undefined, open: isOpen, position: index + 1, size: arranged.length, count: nested?.length ?? 0 });
      rows.push(row);
      if (nested?.length && (isOpen || everyGroup)) visit(nested, level + 1, key);
    }
  };
  visit(top, 1, null);
  return { rows, nesting };
}
