/** Server-paged logs keep the whole scrollbar without holding the whole product.
 * Unloaded windows are one item each, not a synthetic row for every event. */
import { useRef } from 'react';
import { logEntryItems, newestFirst, ROW_HEIGHT, visibleRange } from './logData.ts';
import type { LogDensity, LogEntry, LogItem, LogPage, LogRow } from './logData';
export type { LogPage } from './logData';

/** Count-cache bookkeeping can recreate page descriptors without changing the
 * list. Keep their array stable so row motion and visible-row requests continue. */
export function stableLogPages<T>(previous: readonly LogPage<T>[], next: readonly LogPage<T>[]): readonly LogPage<T>[] {
  if (previous.length !== next.length) return next;
  for (let index = 0; index < previous.length; index++) {
    const before = previous[index];
    const after = next[index];
    if (before.key !== after.key || before.start !== after.start || before.end !== after.end || before.count !== after.count
      || before.rows !== after.rows || before.error !== after.error) return next;
  }
  return previous;
}

/** The pages described, with the continuation the last loaded one names described at once: until a
 * chain stores it, a list whose loaded rows all fall after the hour shown holds no rows and nothing
 * loading, which reads as none. A repeated cursor is the chain's own error, never described. */
export function withContinuation<D extends { cursor: string | null }, P extends { nextCursor: string | null }>(
  described: readonly D[], loaded: ReadonlyMap<number, P>, next: (page: P, index: number) => D): readonly D[] {
  const last = described.length - 1;
  const page = loaded.get(last);
  if (!page || page.nextCursor === null || described.some(info => info.cursor === page.nextCursor)) return described;
  return [...described, next(page, last)];
}

/** `stored` with each page whose measurement no longer `holds` replaced by its `estimate`: the same
 * array while which pages hold is unchanged, so moving within every page's span redoes nothing. */
export function useEstimatedPages<D>(stored: readonly D[] | null, holds: (info: D) => boolean, estimate: (info: D) => D): readonly D[] | null {
  const kept = useRef<{ stored: readonly D[] | null; mask: string; pages: readonly D[] | null }>({ stored: null, mask: '', pages: null });
  if (!stored) return null;
  const mask = stored.map(info => holds(info) ? '1' : '0').join('');
  if (kept.current.stored !== stored || kept.current.mask !== mask) {
    kept.current = { stored, mask, pages: mask.includes('0') ? stored.map(info => holds(info) ? info : estimate(info)) : stored };
  }
  return kept.current.pages;
}

/** Pages newest first, each an unloaded window or its rows newest first. */
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

/** Logical row ordinals, independent of month headings and unloaded windows'
 * compact item representation. The final entry is the whole row count. */
export function logRowOrdinals(items: readonly LogItem<unknown>[]): number[] {
  const ordinals = [0];
  for (const item of items) ordinals.push(ordinals[ordinals.length - 1] + (item.kind === 'month' ? 0 : item.kind === 'page' ? item.page.count : 1));
  return ordinals;
}

export interface LogRowTarget<T> {
  index: number;
  /** The logical row's place inside its window, even before its payload loads. */
  offset: number;
  ordinal: number;
  pageKey?: string;
  row?: LogRow<T>;
}

/** Reaches a logical row in logarithmic time, including one still unloaded. */
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

/** Resolves a pending keyboard row once its own page has loaded. Filtering may
 * remove rows, so it reaches the last remaining row if that offset went away.
 * Rows `shown` refuses (held ahead of the clock) are neither counted nor reached. */
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

/** Overscan in pixels for paged logs: an enormous pending page never causes six
 * other enormous pages to load merely because each is represented by one item. */
export function logPageRange(offsets: readonly number[], top: number, height: number, density: LogDensity, overscan = 6): [number, number] {
  const margin = ROW_HEIGHT[density] * overscan;
  const start = Math.max(0, top - margin);
  return visibleRange(offsets, start, Math.max(height, ROW_HEIGHT[density]) + top - start + margin, 0);
}

/** Holds only pages in view or within its row-height margin, plus a keyboard row
 * until its scroll update is measured. Zero-count checked pages never reappear. */
export function wantedLogPages(items: readonly LogItem<unknown>[], start: number, end: number, keep: readonly string[] = []): string[] {
  const wanted = new Set(keep);
  for (let at = start; at < end; at++) {
    const item = items[at];
    const key = item.kind === 'page' ? item.page.key : item.pageKey;
    if (key != null) wanted.add(key);
  }
  return [...wanted];
}


/** Page by layout pixels through measured rows and compact unloaded windows.
 * Month headings do not occupy a row ordinal; every key advances at least one. */
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

/** The rows of the newest pages held, whole pages from the newest back until at least `count` rows: following the head,
 * the rows a block draws and those beneath it a view holds, which a list names with the block rather than as each
 * comes into view. */
export function newestPageRows<T>(pages: readonly LogPage<T>[], count: number): LogRow<T>[] {
  const rows: LogRow<T>[] = [];
  for (let index = pages.length - 1; index >= 0 && rows.length < count; index--) rows.push(...pages[index].rows ?? []);
  return rows;
}
