/** Logs as the signed-off style shows them (mock-up style-5): HTML rows, newest
 * first, optional month dividers, a comfortable or compact density, virtualised so
 * a campaign's thousands of rows stay cheap. Pure: the list component (`LogList`)
 * draws what these compute, and a widget keeps only what its rows mean. */
import { gameDate, monthIndex, monthNames } from '../../dashboard/calendar.ts';
import type { MovingRow } from '../rowMotion';

/** The moving rows to draw: those at a slot in [start, end), and any `keep` says
 * must stay (the keyboard's row). A row that is gone fades out only while rows
 * animate; without animation it goes at once, never left over its replacement. */
export function shownMotion<T>(rows: readonly MovingRow<T>[], start: number, end: number, animate: boolean,
  keep: (row: MovingRow<T>) => boolean = () => false): MovingRow<T>[] {
  return rows.filter(row => (animate || row.phase !== 'leaving') && ((row.slot >= start && row.slot < end) || keep(row)));
}

/** One dated row of a log. `hour` places it (newest first) and is the date it
 * shows; `by` marks that date as an upper bound (first seen then, happened
 * earlier). `key` identifies it across updates: a new key flashes in. `id` is what
 * the row is about when that outlives its key (a project whose row re-enters as it
 * changes): the keyboard's row and a tooltip follow it; the key when absent. */
export interface LogRow<T> { key: string; id?: string; hour: number; by?: boolean; item: T }

/** What a row is about, for following it across keys. */
export const rowId = (row: LogRow<unknown>) => row.id ?? row.key;

/** Who shows a log's tooltip: the pointer over a row, or the keyboard's row. The one
 * that acted last owns it, as a chart's hover passes between them (`reduceHover`). */
export interface LogTip { id: string; by: 'pointer' | 'keyboard'; x: number; y: number }

/** The row a tooltip is about now, in rows that may have changed since it was shown
 * (a project that re-entered under a new key): found by what it is about, or null
 * once it is gone, so the tooltip is refreshed or hidden, never left stale. */
export function tipRow<T>(tip: Pick<LogTip, 'id'> | null, rows: ReadonlyMap<string, LogRow<T>>): LogRow<T> | null {
  return tip ? rows.get(tip.id) ?? null : null;
}

export type LogDensity = 'comfortable' | 'compact';

/** Row and divider heights in layout pixels (log.css draws them at these). */
export const ROW_HEIGHT: Readonly<Record<LogDensity, number>> = { comfortable: 38, compact: 28 };
export const MONTH_HEIGHT = 24;

/** Rows newest first: by hour, then by the order given (stable). */
export function newestFirst<T>(rows: readonly LogRow<T>[]): LogRow<T>[] {
  return rows.map((row, index) => ({ row, index })).sort((a, b) => b.row.hour - a.row.hour || a.index - b.index).map(entry => entry.row);
}

/** A month divider's label, such as "Jun 1941", from the recording's own calendar. */
export function monthLabel(hour: number): string {
  const date = gameDate(Math.floor(hour));
  return date ? `${monthNames[date.month - 1].slice(0, 3)} ${date.year}` : 'Undated';
}

/** The month an hour falls in, as a comparable index; null before the calendar. */
export function monthOf(hour: number): number | null {
  const date = gameDate(Math.floor(hour));
  return date ? monthIndex(date) : null;
}

/** What a log draws in order: its rows, with a divider before each month's first
 * row when `months` is on. Dividers are keyed by their month, so they hold still. */
export type LogItem<T> = { kind: 'month'; key: string; label: string } | { kind: 'row'; key: string; row: LogRow<T> };

export function logItems<T>(rows: readonly LogRow<T>[], months: boolean): LogItem<T>[] {
  const items: LogItem<T>[] = [];
  let month: number | null | undefined;
  for (const row of rows) {
    if (months) {
      const next = monthOf(row.hour);
      if (next !== month) {
        month = next;
        items.push({ kind: 'month', key: `month:${next ?? 'undated'}`, label: monthLabel(row.hour) });
      }
    }
    items.push({ kind: 'row', key: row.key, row });
  }
  return items;
}

/** Where each item starts, and the whole height: `offsets[i]` is item i's top and
 * `offsets[items.length]` the list's height. */
export function logOffsets(items: readonly LogItem<unknown>[], density: LogDensity): number[] {
  const offsets = [0];
  for (const item of items) offsets.push(offsets[offsets.length - 1] + (item.kind === 'month' ? MONTH_HEIGHT : ROW_HEIGHT[density]));
  return offsets;
}

/** The items to draw for a viewport: [start, end) covering `top`..`top + height`,
 * widened by `overscan` items each way. */
export function visibleRange(offsets: readonly number[], top: number, height: number, overscan = 6): [number, number] {
  const count = offsets.length - 1;
  if (count <= 0) return [0, 0];
  // The last item starting at or above `top`.
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

/** The scroll position that shows item `index` whole, moving as little as possible. */
export function scrollToShow(offsets: readonly number[], index: number, top: number, height: number, stickyTop = 0): number {
  const start = offsets[index];
  const end = offsets[index + 1];
  if (start - stickyTop < top) return Math.max(0, start - stickyTop);
  if (end > top + height) return end - height;
  return top;
}

/** Where a key moves the keyboard's row among `count` rows (null: the key is not
 * the list's). `page` is how many rows a page holds. */
export function keyTarget(key: string, index: number, count: number, page: number): number | null {
  if (count <= 0) return null;
  const clamp = (value: number) => Math.max(0, Math.min(count - 1, value));
  switch (key) {
    case 'ArrowDown': return clamp(index + 1);
    case 'ArrowUp': return clamp(index - 1);
    case 'PageDown': return clamp(index + Math.max(1, page));
    case 'PageUp': return clamp(index - Math.max(1, page));
    case 'Home': return 0;
    case 'End': return count - 1;
    default: return null;
  }
}

/** The date column of a row: day and month, and the year under it. */
export function logDate(hour: number): { day: string; year: string } {
  const date = gameDate(Math.floor(hour));
  if (!date) return { day: `Hour ${Math.floor(hour)}`, year: '' };
  return { day: `${date.day} ${monthNames[date.month - 1].slice(0, 3)}`, year: String(date.year) };
}
