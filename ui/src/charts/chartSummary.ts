import type { Run } from './series.ts';

export interface ChartBucket {
  firstHour: number;
  lastHour: number;
  firstValue: number;
  lastValue: number;
  lastValueHour: number;
  minHour: number;
  minValue: number;
  maxHour: number;
  maxValue: number;
  firstBreak: boolean;
  hasBreak: boolean;
}

export interface ChartSummary {
  from: number;
  to: number;
  width: number;
  buckets: readonly (ChartBucket | null)[];
  runs?: readonly Run[];
  approximate?: readonly { firstHour: number; lastHour: number }[];
  tiles?: TiledSummary;
}

export interface TiledSummary {
  binHours: number;
  valueAt(hour: number): number | null;
}

function placeBucketSample(
  pts: { hour: number; readonly value: number }[],
  desiredHour: number,
  value: number,
  lo: number,
  hi: number,
): boolean {
  if (pts.some(p => p.hour === desiredHour && p.value === value)) return true;
  const used = new Set(pts.map(p => p.hour));
  const clamped = Math.max(lo, Math.min(hi, desiredHour));
  if (!used.has(clamped)) {
    pts.push({ hour: clamped, value });
    return true;
  }
  for (let step = 1; clamped + step <= hi || clamped - step >= lo; step++) {
    if (clamped - step >= lo && !used.has(clamped - step)) {
      pts.push({ hour: clamped - step, value });
      return true;
    }
    if (clamped + step <= hi && !used.has(clamped + step)) {
      pts.push({ hour: clamped + step, value });
      return true;
    }
  }
  return false;
}

export function bucketSamples(
  bucket: ChartBucket,
  until: number,
): readonly { readonly hour: number; readonly value: number }[] {
  const end = Math.min(bucket.lastHour, until);
  const pts: { hour: number; readonly value: number }[] = [{ hour: bucket.firstHour, value: bucket.firstValue }];
  const hasClose = bucket.lastValueHour <= until && (
    bucket.lastValueHour > bucket.firstHour
    || (end > bucket.firstHour && (bucket.lastValue !== bucket.firstValue || bucket.minValue !== bucket.lastValue || bucket.maxValue !== bucket.lastValue))
  );
  const close = hasClose
    ? { hour: bucket.lastValueHour > bucket.firstHour ? bucket.lastValueHour : end, value: bucket.lastValue }
    : null;
  if (close) pts.push(close);
  const placeExtremum = (desiredHour: number, value: number) => {
    if (desiredHour < bucket.firstHour || desiredHour > end) return;
    if (!close) {
      placeBucketSample(pts, desiredHour, value, bucket.firstHour, end);
      return;
    }
    if (desiredHour === close.hour && value === close.value) return;
    const hi = close.hour - 1;
    if (hi >= bucket.firstHour && placeBucketSample(pts, Math.min(desiredHour, hi), value, bucket.firstHour, hi)) return;
    if (close.hour < end) {
      close.hour = end;
      placeBucketSample(pts, Math.min(desiredHour, end - 1), value, bucket.firstHour, end - 1);
    }
  };
  placeExtremum(bucket.minHour, bucket.minValue);
  placeExtremum(bucket.maxHour, bucket.maxValue);
  pts.sort((a, b) => a.hour - b.hour);
  return pts;
}
