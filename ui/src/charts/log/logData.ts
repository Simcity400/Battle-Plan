/** Logs as the signed-off style shows them (mock-up style-5): HTML rows, newest
 * first, optional month dividers, a comfortable or compact density, virtualised so
 * a campaign's thousands of rows stay cheap. Pure: the list component (`LogList`)
 * draws what these compute, and a widget keeps only what its rows mean. */
import { gameDate, monthIndex, monthNames } from '../../dashboard/calendar.ts';
import type { MovingRow } from '../rowMotion';

/** The moving rows to draw: those at a slot in [start, end), and any `keep` says
 * must stay (the keyboard's row). Rows may fade out while motion is on. Cards
 * leave immediately, so a backward seek never retains a future conference. */
export function shownMotion<T>(rows: readonly MovingRow<T>[], start: number, end: number, animate: boolean,
  keep: (row: MovingRow<T>) => boolean = () => false, layout: 'row' | 'card' = 'row'): MovingRow<T>[] {
  return rows.filter(row => (animate && layout === 'row' || row.phase !== 'leaving') && ((row.slot >= start && row.slot < end) || keep(row)));
}

/** Cards enter only when their key is new to the committed log. Keeping every
 * committed key stops a card replaying its entry after scrolling or seeking. */
export function newCardKeys<T>(rows: readonly MovingRow<T>[], known: ReadonlySet<string>, enabled = true): string[] {
  return enabled ? rows.filter(row => row.phase === 'entering' && !known.has(row.key)).map(row => row.key) : [];
}

/** One dated row of a log. `hour` places it (newest first) and is the date it
 * shows; `by` marks that date as an upper bound (first seen then, happened
 * earlier). `key` identifies it across updates: a new key flashes in. `id` is what
 * the row is about when that outlives its key (a project whose row re-enters as it
 * changes): the keyboard's row and a tooltip follow it; the key when absent. */
export interface LogRow<T> {
  key: string;
  id?: string;
  hour: number;
  /** The read that first revealed this row, when its displayed date predates
   * that read. Live entry motion uses this hour; defaults to `hour`. */
  revealedHour?: number;
  by?: boolean;
  item: T;
}

/** A stable server window. Absent rows occupy `count` virtual rows until wanted;
 * a caller can retain a filtered count after releasing the window's payload. */
export interface LogPage<T> {
  key: string;
  start: number;
  end: number;
  count: number;
  rows?: readonly LogRow<T>[];
  error?: string;
}

/** Tracks after the date, shared by every row or chosen from that row's content. */
export type LogColumns<T> = string | ((row: LogRow<T>) => string);
export const logRowColumns = <T>(columns: LogColumns<T>, row: LogRow<T>): string => typeof columns === 'string' ? columns : columns(row);

/** Only countries on drawn rows, at each row's own observation hour. Opaque
 * tags never share a delimiter key, and later names cannot label older rows. */
export function logCountryRequests<T>(rows: readonly LogRow<T>[], countries: (row: LogRow<T>) => { hour: number; tags: readonly (string | null)[] }): { hour: number; tag: string }[] {
  const wanted = new Map<number, Set<string>>();
  for (const row of rows) {
    const { hour, tags } = countries(row);
    const set = wanted.get(hour) ?? new Set<string>();
    for (const tag of tags) if (tag !== null) set.add(tag);
    wanted.set(hour, set);
  }
  return [...wanted].flatMap(([hour, tags]) => [...tags].map(tag => ({ hour, tag })));
}

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

/** Intrinsic card heights apply only to the layout width and density measured. */
export interface CardMeasurements { width: number; density: LogDensity; font?: string; heights: ReadonlyMap<string, number> }
const NO_HEIGHTS: ReadonlyMap<string, number> = new Map();

export const cardHeightsAt = (measured: CardMeasurements, width: number, density: LogDensity, font = ''): ReadonlyMap<string, number> =>
  measured.width === width && measured.density === density && (measured.font ?? '') === font ? measured.heights : NO_HEIGHTS;

/** Merge one observed batch, discarding stale wrapping after layout changes and
 * released page keys when supplied. Unchanged measurements keep their identity. */
export function updateCardHeights(measured: CardMeasurements, width: number, density: LogDensity,
  sizes: readonly { key: string; height: number }[], font = '', held?: ReadonlySet<string>): CardMeasurements {
  const sameLayout = measured.width === width && measured.density === density && (measured.font ?? '') === font;
  const previous = cardHeightsAt(measured, width, density, font);
  let next: Map<string, number> | undefined;
  if (held) for (const key of previous.keys()) if (!held.has(key)) {
    next ??= new Map(previous);
    next.delete(key);
  }
  for (const { key, height } of sizes) {
    if (held && !held.has(key)) continue;
    if (height <= 0 || !Number.isFinite(height) || previous.get(key) === height) continue;
    next ??= new Map(previous);
    next.set(key, height);
  }
  return sameLayout && !next ? measured : { width, density, font, heights: next ?? previous };
}

interface TipElement { getAttribute(name: string): string | null; parentElement: TipElement | null }

/** A chip or other row part with its own tooltip takes hover from the row. */
export function hasLogPartTip(target: TipElement | null, row: TipElement): boolean {
  for (let at = target; at && at !== row; at = at.parentElement) {
    const text = at.getAttribute('data-tooltip');
    if (text != null) return text.length > 0;
  }
  return false;
}

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
export type LogItem<T> = { kind: 'month'; key: string; label: string; pageKey?: string }
  | { kind: 'row'; key: string; row: LogRow<T>; pageKey?: string; pageOffset?: number }
  | { kind: 'page'; key: string; page: LogPage<T> };

export type LogEntry<T> = { kind: 'row'; row: LogRow<T>; pageKey?: string; pageOffset?: number } | { kind: 'page'; page: LogPage<T> };

export function logItems<T>(rows: readonly LogRow<T>[], months: boolean): LogItem<T>[] {
  return logEntryItems(rows.map(row => ({ kind: 'row', row })), months);
}

/** Ordinary and paged logs share month sections. An unloaded window interrupts
 * a section: no month is assigned to rows whose dates have not been loaded. */
export function logEntryItems<T>(entries: readonly LogEntry<T>[], months: boolean): LogItem<T>[] {
  const items: LogItem<T>[] = [];
  const sections = new Map<number | null, number>();
  let month: number | null | undefined;
  for (const entry of entries) {
    if (entry.kind === 'page') {
      if (entry.page.count > 0) {
        items.push({ kind: 'page', key: JSON.stringify(['page', entry.page.key]), page: entry.page });
        month = undefined;
      }
      continue;
    }
    const { row, pageKey, pageOffset } = entry;
    if (months) {
      const next = monthOf(row.hour);
      if (next !== month) {
        month = next;
        // Pinned running rows can precede a newer finished section of the same month.
        const section = sections.get(next) ?? 0;
        sections.set(next, section + 1);
        items.push({ kind: 'month', key: `month:${next ?? 'undated'}${section ? `:${section}` : ''}`, label: monthLabel(row.hour), ...(pageKey == null ? {} : { pageKey }) });
      }
    }
    items.push({ kind: 'row', key: row.key, row, ...(pageKey == null ? {} : { pageKey, pageOffset }) });
  }
  return items;
}

/** Where each item starts, and the whole height: `offsets[i]` is item i's top and
 * `offsets[items.length]` the list's height. */
export function logOffsets<T>(items: readonly LogItem<T>[], density: LogDensity, rowHeight?: (row: LogRow<T>) => number): number[] {
  const offsets = [0];
  for (const item of items) {
    const requested = item.kind === 'row' ? rowHeight?.(item.row) : item.kind === 'page' ? ROW_HEIGHT[density] * item.page.count : MONTH_HEIGHT;
    const height = requested != null && Number.isFinite(requested) && requested > 0 ? requested : ROW_HEIGHT[density];
    offsets.push(offsets[offsets.length - 1] + height);
  }
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
  // A card taller than the viewport stays at its heading on successive keys.
  if (end - start > height - stickyTop) return Math.max(0, start - stickyTop);
  if (start - stickyTop < top) return Math.max(0, start - stickyTop);
  if (end > top + height) return end - height;
  return top;
}

/** Page through variable-height cards by a viewport of layout pixels, with at
 * least one row of progress. `slots` maps keyboard rows to their log items. */
export function cardPageTarget(offsets: readonly number[], slots: readonly number[], index: number, height: number, direction: 1 | -1): number {
  if (!slots.length) return 0;
  const at = Math.max(0, Math.min(slots.length - 1, index));
  const goal = offsets[slots[at]] + direction * Math.max(0, height);
  let low = 0;
  let high = slots.length - 1;
  while (low < high) {
    const middle = direction === 1 ? (low + high + 1) >> 1 : (low + high) >> 1;
    if (direction === 1) {
      if (offsets[slots[middle]] <= goal) low = middle; else high = middle - 1;
    } else if (offsets[slots[middle]] >= goal) high = middle; else low = middle + 1;
  }
  return direction === 1 ? Math.min(slots.length - 1, Math.max(at + 1, low)) : Math.max(0, Math.min(at - 1, low));
}

/** Where a key moves the keyboard's row among `count` rows (null: the key is not
 * the list's). `page` is how many rows a page holds. */
export { keyTarget } from '../rowNavigation.ts';

/** The date column of a row: day and month, and the year under it. */
export function logDate(hour: number): { day: string; year: string } {
  const date = gameDate(Math.floor(hour));
  if (!date) return { day: `Hour ${Math.floor(hour)}`, year: '' };
  return { day: `${date.day} ${monthNames[date.month - 1].slice(0, 3)}`, year: String(date.year) };
}

/** Retained-window logs keep the first visible row in place as off-screen rows
 * arrive or are released. At the top, new entries remain immediately visible. */
export function anchoredScrollTop<T>(previous: readonly LogItem<T>[], before: readonly number[],
  next: readonly LogItem<T>[], after: readonly number[], top: number): number {
  if (top <= 0) return 0;
  let at = before.length - 1;
  while (at > 0 && before[at] > top) at--;
  at = Math.max(0, at);
  const item = previous[at];
  if (!item) return top;
  const clampWithin = (index: number, within: number) => Math.min(within, Math.max(0, after[index + 1] - after[index] - 1));
  const index = next.findIndex(candidate => candidate.key === item.key
    && (item.kind !== 'month' || candidate.kind === 'month' && candidate.pageKey === item.pageKey));
  if (index >= 0) return Math.max(0, after[index] + clampWithin(index, top - before[at]));
  // Page load/release changes item keys. Keep its logical row offset, including
  // loaded month headings, without retaining an offset beyond a shrinking card.
  const pageKey = item.kind === 'page' ? item.page.key : item.pageKey;
  if (pageKey == null) return top;
  const delta = top - before[at];
  const step = item.kind === 'page' ? (before[at + 1] - before[at]) / item.page.count : before[at + 1] - before[at];
  const offset = item.kind === 'page' ? Math.floor(delta / step) : item.kind === 'row' ? item.pageOffset ?? 0 : 0;
  const within = item.kind === 'month' ? 0 : item.kind === 'page' ? delta - offset * step : delta;
  let target = -1;
  for (let i = 0; i < next.length; i++) {
    const candidate = next[i];
    if (candidate.kind === 'page' && candidate.page.key === pageKey) {
      const height = (after[i + 1] - after[i]) / candidate.page.count;
      return Math.max(0, after[i] + Math.min(candidate.page.count - 1, offset) * height + Math.min(within, Math.max(0, height - 1)));
    }
    if (candidate.kind !== 'row' || candidate.pageKey !== pageKey) continue;
    target = i;
    if ((candidate.pageOffset ?? 0) >= offset) break;
  }
  return target < 0 ? top : Math.max(0, after[target] + clampWithin(target, within));
}
