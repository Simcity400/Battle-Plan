import type { LogRow } from './logData';

/** A page's filtered, date-ordered rows, indexed once by their supplying reads.
 * Exact dates can precede observation, so the display order and reveal order
 * deliberately remain separate. */
export interface LogPlaybackIndex<T> {
  rows: readonly LogRow<T>[];
  reads: readonly number[];
}
export interface LogPlaybackSlice<T> {
  index: LogPlaybackIndex<T>;
  count: number;
  rows: readonly LogRow<T>[];
}

export function logPlaybackIndex<T>(rows: readonly LogRow<T>[]): LogPlaybackIndex<T> {
  return { rows, reads: rows.map(row => Math.max(row.hour, row.revealedHour ?? row.hour)).sort((a, b) => a - b) };
}

/** A binary read cutoff preserves the last slice until a read is crossed. Only
 * a changed cutoff needs to select rows; it never changes their prepared order. */
export function logPlaybackAt<T>(index: LogPlaybackIndex<T>, hour: number | null,
  held?: LogPlaybackSlice<T>): LogPlaybackSlice<T> {
  let low = 0;
  let high = hour === null ? 0 : index.reads.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (index.reads[middle] <= hour!) low = middle + 1; else high = middle;
  }
  if (held?.index === index && held.count === low) return held;
  const rows = low === 0 ? [] : low === index.rows.length ? index.rows
    : index.rows.filter(row => Math.max(row.hour, row.revealedHour ?? row.hour) <= hour!);
  return { index, count: low, rows };
}
