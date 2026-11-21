import type { HourSpan } from '../generated/dashboard';

export type KeyedRun = readonly [number, number, string, boolean];

export interface KeyedWindow {
  firstHour: number;
  lastHour: number;
  known: readonly HourSpan[];
  reads: readonly (readonly [number, number, number | null])[];
  series: readonly { name: string; runs: readonly KeyedRun[] }[];
}

const isRun = (value: unknown): value is KeyedRun => { const run = value as unknown[]; return Array.isArray(run) && run.length === 4
  && Number.isSafeInteger(run[0]) && Number.isSafeInteger(run[1]) && (run[0] as number) <= (run[1] as number)
  && typeof run[2] === 'string' && /^-?\d+$/.test(run[2]) && typeof run[3] === 'boolean'; };

const ordered = <T>(items: readonly T[], first: (item: T) => number, last: (item: T) => number) =>
  items.every((item, index) => index === 0 || last(items[index - 1]) < first(item));

export function validateKeyedWindow(value: unknown): KeyedWindow {
  const window = value as KeyedWindow;
  if (!window || typeof window !== 'object' || !Number.isSafeInteger(window.firstHour) || !Number.isSafeInteger(window.lastHour)
    || !Array.isArray(window.known) || !ordered(window.known, span => span.firstHour, span => span.lastHour)
    || !Array.isArray(window.reads) || !window.reads.every(read => Array.isArray(read) && read.length === 3 && Number.isSafeInteger(read[0]) && Number.isSafeInteger(read[1]) && read[0] <= read[1]
      && (read[2] === null || Number.isSafeInteger(read[2])))
    || !ordered(window.reads, read => read[0], read => read[1])
    || !Array.isArray(window.series) || !window.series.every(series => series && typeof series.name === 'string'
      && Array.isArray(series.runs) && (series.runs as readonly unknown[]).every(isRun) && ordered(series.runs as readonly KeyedRun[], run => run[0], run => run[1])))
    throw new Error('The keyed history in this reply is malformed.');
  return window;
}

function find<T>(items: readonly T[], hour: number, first: (item: T) => number, last: (item: T) => number): T | undefined {
  let low = 0, high = items.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1, item = items[middle];
    if (hour < first(item)) high = middle - 1;
    else if (hour > last(item)) low = middle + 1;
    else return item;
  }
  return undefined;
}

export const keyedKnown = (window: KeyedWindow, hour: number) =>
  !!find(window.known, hour, span => span.firstHour, span => span.lastHour);

export function keyedRead(window: KeyedWindow, hour: number): number | undefined {
  const read = find(window.reads, hour, read => read[0], read => read[1]);
  return read && (read[2] ?? hour);
}

export const runAtHour = (runs: readonly KeyedRun[], hour: number) => find(runs, hour, run => run[0], run => run[1]);

export function keyedValuesAt(window: KeyedWindow, hour: number): Map<string, string> {
  const values = new Map<string, string>();
  if (!keyedKnown(window, hour)) return values;
  for (const series of window.series) {
    const run = runAtHour(series.runs, hour);
    if (run) values.set(series.name, run[2]);
  }
  return values;
}

export const nameParts = (name: string) => name.split('|');

export function countryPart(part: string): { tag: string; slot: number } | null {
  const at = part.lastIndexOf('@');
  const slot = Number(part.slice(at + 1));
  return at > 0 && Number.isSafeInteger(slot) ? { tag: part.slice(0, at), slot } : null;
}

export function keyedChanges(window: KeyedWindow): number[] {
  const hours = new Set<number>();
  for (const span of window.known) hours.add(span.firstHour);
  for (const series of window.series) for (const run of series.runs) { hours.add(run[0]); hours.add(run[1] + 1); }
  return [...hours].filter(hour => hour >= window.firstHour && hour <= window.lastHour).sort((a, b) => a - b);
}

export function chartRuns(runs: readonly KeyedRun[], divisor = 1): { firstHour: number; lastHour: number; value: number; breakBefore: boolean }[] {
  return runs.map(run => ({ firstHour: run[0], lastHour: run[1], value: Number(run[2]) / divisor, breakBefore: run[3] }));
}
