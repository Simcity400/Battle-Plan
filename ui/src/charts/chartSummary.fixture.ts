import type { ChartBucket } from './chartSummary.ts';

type Bucket = ChartBucket & { absoluteTotal?: string };

/** A summary as the dashboard sends it: its buckets as the columns `chartSummaryFrom` reads. */
export function wireSummary<S extends { buckets: readonly (Bucket | null)[] }>(summary: S) {
  const skip: number[] = [], hours: number[] = [], values: number[] = [], flags: number[] = [], totals: string[] = [];
  let gap = 0, previous: number | null = null;
  for (const bucket of summary.buckets) {
    if (!bucket) { gap++; continue; }
    skip.push(gap); gap = 0;
    hours.push(previous === null ? bucket.firstHour : bucket.firstHour - previous, bucket.lastHour - bucket.firstHour,
      bucket.lastValueHour - bucket.firstHour, bucket.minHour - bucket.firstHour, bucket.maxHour - bucket.firstHour);
    values.push(bucket.firstValue, bucket.lastValue, bucket.minValue, bucket.maxValue);
    flags.push(Number(bucket.firstBreak) | Number(bucket.hasBreak) << 1);
    totals.push(bucket.absoluteTotal ?? '0');
    previous = bucket.lastHour;
  }
  return { ...summary, buckets: { slots: summary.buckets.length, skip, hours, values, flags, totals } };
}

/** `value` with every summary in it (an object with a `width` and an array of `buckets`) as the dashboard sends it. */
export function wireSummaries<T>(value: T): T {
  if (Array.isArray(value)) return value.map(wireSummaries) as T;
  if (!value || typeof value !== 'object') return value;
  const object = value as Record<string, unknown>;
  if (typeof object.width === 'number' && Array.isArray(object.buckets)) return wireSummary(object as { buckets: (Bucket | null)[] }) as T;
  return Object.fromEntries(Object.entries(object).map(([key, item]) => [key, wireSummaries(item)])) as T;
}
