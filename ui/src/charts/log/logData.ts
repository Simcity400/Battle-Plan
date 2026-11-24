import { type Failure } from '../../dashboard/queryError.ts';
import { gameDate, monthIndex, monthNames } from '../../dashboard/calendar.ts';
import type { MovingRow } from '../rowMotion';

export function shownMotion<T>(rows: readonly MovingRow<T>[], start: number, end: number, animate: boolean,
  keep: (row: MovingRow<T>) => boolean = () => false, layout: 'row' | 'card' = 'row'): MovingRow<T>[] {
  return rows.filter(row => (animate && layout === 'row' || row.phase !== 'leaving') && ((row.slot >= start && row.slot < end) || keep(row)));
}

export function newCardKeys<T>(rows: readonly MovingRow<T>[], known: ReadonlySet<string>, enabled = true): string[] {
  return enabled ? rows.filter(row => row.phase === 'entering' && !known.has(row.key)).map(row => row.key) : [];
}

export interface LogRow<T> {
  readonly key: string;
  readonly id?: string;
  readonly hour: number;
  readonly revealedHour?: number;
  readonly leftHour?: number;
  readonly by?: boolean;
  readonly item: T;
}

export interface LogPage<T> {
  readonly key: string;
  readonly start: number;
  readonly end: number;
  readonly count: number;
  readonly rows?: ReadonlyArray<LogRow<T>>;
  readonly error?: Failure;
}

export type LogColumns<T> = string | ((row: LogRow<T>) => string);
export const logRowColumns = <T>(columns: LogColumns<T>, row: LogRow<T>): string => typeof columns === 'string' ? columns : columns(row);

export interface CountryRequest { readonly hour: number; readonly tag: string }

export function logCountryRequests<T>(rows: readonly LogRow<T>[], countries: (row: LogRow<T>) => { hour: number; tags: readonly (string | null)[] }): CountryRequest[] {
  const wanted = new Map<number, Set<string>>();
  for (const row of rows) {
    const { hour, tags } = countries(row);
    const set = wanted.get(hour) ?? new Set<string>();
    for (const tag of tags) if (tag !== null) set.add(tag);
    wanted.set(hour, set);
  }
  return [...wanted].flatMap(([hour, tags]) => [...tags].map(tag => ({ hour, tag })));
}

export const rowId = (row: LogRow<unknown>) => row.id ?? row.key;

export interface LogTip { id: string; by: 'pointer' | 'keyboard'; x: number; y: number }

export type LogDensity = 'comfortable' | 'compact';

export interface CardMeasurements { width: number; density: LogDensity; font?: string; heights: ReadonlyMap<string, number> }
const NO_HEIGHTS: ReadonlyMap<string, number> = new Map();

export const cardHeightsAt = (measured: CardMeasurements, width: number, density: LogDensity, font = ''): ReadonlyMap<string, number> =>
  measured.width === width && measured.density === density && (measured.font ?? '') === font ? measured.heights : NO_HEIGHTS;

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

export function hasLogPartTip(target: TipElement | null, row: TipElement): boolean {
  for (let at = target; at && at !== row; at = at.parentElement) {
    const text = at.getAttribute('data-tooltip');
    if (text != null) return text.length > 0;
  }
  return false;
}

export const ROW_HEIGHT: Readonly<Record<LogDensity, number>> = { comfortable: 38, compact: 28 };
export const MONTH_HEIGHT = 24;

export function newestFirst<T>(rows: readonly LogRow<T>[]): ReadonlyArray<LogRow<T>> {
  return rows.map((row, index) => ({ row, index })).sort((a, b) => b.row.hour - a.row.hour || a.index - b.index).map(entry => entry.row);
}

export function monthLabel(hour: number): string {
  const date = gameDate(Math.floor(hour));
  return date ? `${monthNames[date.month - 1].slice(0, 3)} ${date.year}` : 'Undated';
}

export function monthOf(hour: number): number | null {
  const date = gameDate(Math.floor(hour));
  return date ? monthIndex(date) : null;
}

export type LogItem<T> = Readonly<{ kind: 'month'; key: string; label: string; pageKey?: string; part?: LogSectionPart; section?: string }>
  | Readonly<{ kind: 'row'; key: string; row: LogRow<T>; pageKey?: string; pageOffset?: number }>
  | Readonly<{ kind: 'page'; key: string; page: LogPage<T> }>;

export type LogEntry<T> = Readonly<{ kind: 'row'; row: LogRow<T>; pageKey?: string; pageOffset?: number }> | Readonly<{ kind: 'page'; page: LogPage<T> }>;

export function logItems<T>(rows: readonly LogRow<T>[], months: boolean): ReadonlyArray<LogItem<T>> {
  return logEntryItems(rows.map(row => ({ kind: 'row', row })), months);
}

export type LogSectionPart = 'heading' | 'note';

export interface LogSection<T> {
  readonly key: string;
  readonly label: string;
  readonly rows?: readonly LogRow<T>[];
  readonly pages?: readonly LogPage<T>[];
  readonly months?: boolean;
}

export function stableLogSections<S extends LogSection<unknown>>(previous: readonly S[] | undefined, next: readonly S[]): readonly S[] {
  return previous && previous.length === next.length && previous.every((section, index) => {
    const other = next[index];
    return section.key === other.key && section.label === other.label && section.rows === other.rows && section.pages === other.pages && section.months === other.months;
  }) ? previous : next;
}

export function logSectionItems<T>(sections: readonly LogSection<T>[], months: boolean, held: boolean,
  pageEntries: (pages: readonly LogPage<T>[]) => LogEntry<T>[]): ReadonlyArray<LogItem<T>> {
  const items: LogItem<T>[] = [];
  for (const section of sections) {
    const entries = section.pages ? pageEntries(section.pages) : (section.rows ?? []).map<LogEntry<T>>(row => ({ kind: 'row', row }));
    const inner = logEntryItems(entries, section.months ?? months);
    items.push({ kind: 'month', key: JSON.stringify(['section', section.key]), label: section.label, part: 'heading', section: section.key });
    if (held || !inner.length) items.push({ kind: 'month', key: JSON.stringify(['section-note', section.key]), label: '', part: 'note', section: section.key });
    for (const item of inner) items.push(item.kind === 'month' ? { ...item, key: JSON.stringify([section.key, item.key]) } : item);
  }
  return items;
}

export function logEntryItems<T>(entries: readonly LogEntry<T>[], months: boolean): ReadonlyArray<LogItem<T>> {
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
        const section = sections.get(next) ?? 0;
        sections.set(next, section + 1);
        items.push({ kind: 'month', key: `month:${next ?? 'undated'}${section ? `:${section}` : ''}`, label: monthLabel(row.hour), ...(pageKey == null ? {} : { pageKey }) });
      }
    }
    items.push({ kind: 'row', key: row.key, row, ...(pageKey == null ? {} : { pageKey, pageOffset }) });
  }
  return items;
}

export function logOffsets<T>(items: readonly LogItem<T>[], density: LogDensity, rowHeight?: (row: LogRow<T>) => number): number[] {
  const offsets = [0];
  for (const item of items) {
    const requested = item.kind === 'row' ? rowHeight?.(item.row) : item.kind === 'page' ? ROW_HEIGHT[density] * item.page.count
      : item.part === 'note' ? ROW_HEIGHT[density] : MONTH_HEIGHT;
    const height = requested != null && Number.isFinite(requested) && requested > 0 ? requested : ROW_HEIGHT[density];
    offsets.push(offsets[offsets.length - 1] + height);
  }
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

export function scrollToShow(offsets: readonly number[], index: number, top: number, height: number, stickyTop = 0): number {
  const start = offsets[index];
  const end = offsets[index + 1];
  if (end - start > height - stickyTop) return Math.max(0, start - stickyTop);
  if (start - stickyTop < top) return Math.max(0, start - stickyTop);
  if (end > top + height) return end - height;
  return top;
}

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

export { keyTarget } from '../rowNavigation.ts';

export function logDate(hour: number): { day: string; year: string } {
  const date = gameDate(Math.floor(hour));
  if (!date) return { day: `Hour ${Math.floor(hour)}`, year: '' };
  return { day: `${date.day} ${monthNames[date.month - 1].slice(0, 3)}`, year: String(date.year) };
}

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
