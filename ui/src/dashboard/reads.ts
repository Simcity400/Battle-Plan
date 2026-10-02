import { formatDateTime } from './calendar.ts';

/** When recorded values were true. A table read on a cadence (daily, say) carries
 * each read's values to the hours until the next read; products list the hours of
 * the reads, ascending. */

/** The latest read at or before an hour: when a value shown then was read. */
export function readAt(reads: readonly number[], hour: number): number | null {
  let low = 0;
  let high = reads.length - 1;
  let found: number | null = null;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (reads[middle] <= hour) { found = reads[middle]; low = middle + 1; } else high = middle - 1;
  }
  return found;
}

/** The recorded entry that applies at an hour (such as a variant's name as a read
 * recorded it): the latest whose `firstHour` is at or before the hour, in entries
 * ascending by `firstHour`. None before the first: nothing recorded applies yet, so
 * a later entry is never backfilled into earlier hours. */
export function entryAt<T extends { firstHour: number }>(entries: readonly T[], hour: number): T | null {
  let low = 0;
  let high = entries.length - 1;
  let found: T | null = null;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (entries[middle].firstHour <= hour) { found = entries[middle]; low = middle + 1; } else high = middle - 1;
  }
  return found;
}

/** The note under a value shown at `hour` that was read earlier, such as "Read
 * 2 Dec 1943 · 20:00; resources are read once a day."; none when it was read then.
 * `cadence` says how often it is read. */
export function readNote(read: number | null, hour: number, cadence: string): string | undefined {
  return read != null && read < hour ? `Read ${formatDateTime(read)}; ${cadence}` : undefined;
}
