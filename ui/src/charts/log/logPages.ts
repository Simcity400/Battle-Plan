import { logEntryItems, newestFirst, ROW_HEIGHT, visibleRange } from './logData.ts';
import type { LogDensity, LogEntry, LogItem, LogPage, LogRow } from './logData';

export function logPageEntries<T>(pages: readonly LogPage<T>[]): LogEntry<T>[] {
  const entries: LogEntry<T>[] = [];
  const ordered = pages.map((page, index) => ({ page, index })).sort((a, b) => b.page.end - a.page.end || b.page.start - a.page.start || a.index - b.index);
  for (const { page } of ordered) {
    if (page.rows == null) entries.push({ kind: 'page', page });
    else newestFirst(page.rows).forEach((row, pageOffset) => entries.push({ kind: 'row', row, pageKey: page.key, pageOffset }));
  }
  return entries;
}

export function logPageItems<T>(pages: readonly LogPage<T>[], months: boolean): ReadonlyArray<LogItem<T>> {
  return logEntryItems(logPageEntries(pages), months);
}

export function logRowOrdinals(items: readonly LogItem<unknown>[]): number[] {
  const ordinals = [0];
  for (const item of items) ordinals.push(ordinals[ordinals.length - 1] + (item.kind === 'month' ? 0 : item.kind === 'page' ? item.page.count : 1));
  return ordinals;
}

export interface LogRowTarget<T> {
  index: number;
  offset: number;
  ordinal: number;
  pageKey?: string;
  row?: LogRow<T>;
}

export function logRowTarget<T>(items: readonly LogItem<T>[], ordinals: readonly number[], ordinal: number): LogRowTarget<T> | null {
  if (ordinal < 0 || ordinal >= ordinals[items.length]) return null;
  let low = 0;
  let high = items.length - 1;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (ordinals[middle + 1] <= ordinal) low = middle + 1; else high = middle;
  }
  const item = items[low];
  if (item.kind === 'month') return null;
  if (item.kind === 'page') return { index: low, offset: ordinal - ordinals[low], ordinal, pageKey: item.page.key };
  return { index: low, offset: item.pageOffset ?? ordinal, ordinal, pageKey: item.pageKey, row: item.row };
}

export function logPageTarget<T>(items: readonly LogItem<T>[], ordinals: readonly number[], pageKey: string, offset: number,
  shown: (index: number) => boolean = () => true): LogRowTarget<T> | null {
  let last: LogRowTarget<T> | null = null;
  let seen = 0;
  for (let index = 0; index < items.length; index++) {
    const item = items[index];
    if (item.kind === 'page' && item.page.key === pageKey) {
      if (item.page.count <= 0) return null;
      const within = Math.max(0, Math.min(item.page.count - 1, offset));
      return { index, offset: within, ordinal: ordinals[index] + within, pageKey };
    }
    if (item.kind !== 'row' || item.pageKey !== pageKey || !shown(index)) continue;
    last = { index, offset: seen, ordinal: ordinals[index], pageKey, row: item.row };
    if (seen++ >= offset) return last;
  }
  return last;
}

export function logPageRange(offsets: readonly number[], top: number, height: number, density: LogDensity, overscan = 6): [number, number] {
  const margin = ROW_HEIGHT[density] * overscan;
  const start = Math.max(0, top - margin);
  return visibleRange(offsets, start, Math.max(height, ROW_HEIGHT[density]) + top - start + margin, 0);
}

export function wantedLogPages(items: readonly LogItem<unknown>[], start: number, end: number, keep: readonly string[] = []): string[] {
  const wanted = new Set(keep);
  for (let at = start; at < end; at++) {
    const item = items[at];
    const key = item.kind === 'page' ? item.page.key : item.pageKey;
    if (key != null) wanted.add(key);
  }
  return [...wanted];
}

export function logPixelTarget<T>(items: readonly LogItem<T>[], offsets: readonly number[], ordinals: readonly number[],
  ordinal: number, height: number, direction: 1 | -1): LogRowTarget<T> | null {
  const current = logRowTarget(items, ordinals, ordinal);
  if (!current) return null;
  const item = items[current.index];
  const step = item.kind === 'page' ? (offsets[current.index + 1] - offsets[current.index]) / item.page.count : 0;
  const top = offsets[current.index] + (item.kind === 'page' ? current.offset * step : 0);
  const goal = Math.max(0, top + direction * Math.max(0, height));
  const [index] = visibleRange(offsets, goal, 0, 0);
  const destination = items[index];
  let target = ordinals[index];
  if (destination?.kind === 'page') {
    const rowHeight = (offsets[index + 1] - offsets[index]) / destination.page.count;
    const within = (goal - offsets[index]) / rowHeight;
    target += direction === 1 ? Math.floor(within) : Math.ceil(within);
  } else if (direction === -1 && offsets[index] < goal && destination?.kind === 'row') target++;
  else if (direction === 1 && destination?.kind === 'month') target--;
  target = direction === 1 ? Math.max(ordinal + 1, target) : Math.min(ordinal - 1, target);
  return logRowTarget(items, ordinals, Math.max(0, Math.min(ordinals[items.length] - 1, target)));
}
