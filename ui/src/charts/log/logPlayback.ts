import type { LogRow } from './logData';

/** A page's filtered, date-ordered rows, indexed once by their supplying reads.
 * Exact dates can precede observation, so the display order and reveal order
 * deliberately remain separate. */
export interface LogPlaybackIndex<T> {
  readonly rows: ReadonlyArray<LogRow<T>>;
  readonly reads: ReadonlyArray<number>;
  readonly reveal?: (row: LogRow<T>) => number;
}
export interface LogPlaybackSlice<T> {
  readonly index: LogPlaybackIndex<T>;
  readonly count: number;
  readonly rows: ReadonlyArray<LogRow<T>>;
}

const revealHour = <T>(row: LogRow<T>) => Math.max(row.hour, row.revealedHour ?? row.hour);

export function logPlaybackIndex<T>(rows: readonly LogRow<T>[], reveal?: (row: LogRow<T>) => number): LogPlaybackIndex<T> {
  return { rows, reads: rows.map(reveal ?? revealHour).sort((a, b) => a - b), ...(reveal ? { reveal } : {}) };
}

/** A binary read cutoff preserves the last slice until a read is crossed. Only
 * a changed cutoff needs to select rows; it never changes their prepared order. */
export function logPlaybackAt<T>(index: LogPlaybackIndex<T>, hour: number | null,
  held?: LogPlaybackSlice<T>): LogPlaybackSlice<T> {
  if (held?.index === index && (hour === null ? held.count === 0
    : (held.count === 0 || index.reads[held.count - 1] <= hour) && (held.count === index.reads.length || hour < index.reads[held.count]))) return held;
  let low = 0;
  let high = hour === null ? 0 : index.reads.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (index.reads[middle] <= hour!) low = middle + 1; else high = middle;
  }
  if (held?.index === index && held.count === low) return held;
  const rows = low === 0 ? [] : low === index.rows.length ? index.rows
    : index.rows.filter(row => (index.reveal ?? revealHour)(row) <= hour!);
  return { index, count: low, rows };
}

export class LogPlayback<T> {
  private readonly index: LogPlaybackIndex<T>;
  private held?: LogPlaybackSlice<T>;
  constructor(rows: ReadonlyArray<LogRow<T>>, reveal?: (row: LogRow<T>) => number) { this.index = logPlaybackIndex(rows, reveal); }
  at(hour: number | null): ReadonlyArray<LogRow<T>> {
    return (this.held = logPlaybackAt(this.index, hour, this.held)).rows;
  }
}
