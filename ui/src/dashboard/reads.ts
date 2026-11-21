import { formatDateTime } from './calendar.ts';
import { runAt } from '../charts/series.ts';


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

export interface ReadRun { firstHour: number; lastHour: number; readHour: number; hourly: boolean }
export function readRunAt(reads: readonly ReadRun[], hour: number): number | null;
export function readRunAt(reads: readonly Omit<ReadRun, 'hourly'>[], hour: number, implicitHourly: true): number;
export function readRunAt(reads: readonly (Omit<ReadRun, 'hourly'> & { hourly?: boolean })[], hour: number, implicitHourly = false): number | null {
  const run = runAt(reads, hour);
  return run ? run.hourly ? hour : run.readHour : implicitHourly ? hour : null;
}

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

export function readNote(read: number | null, hour: number, cadence: string): string | undefined {
  return read != null && read < hour ? `Read ${formatDateTime(read)}; ${cadence}` : undefined;
}
