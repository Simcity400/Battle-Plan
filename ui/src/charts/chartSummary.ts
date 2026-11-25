import type { PlotFrame } from './frame.ts';
import { runAt } from './series.ts';
import type { Run } from './series.ts';
import { addStackAmount, stackEdges } from './stack.ts';
import type { CompactStackModel } from './stack';

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

export const exactRuns = (summary: ChartSummary): readonly Run[] | null => summary.runs ?? null;

function approximateUnion(spans: ChartSummary['approximate']) {
  const union: NonNullable<ChartSummary['approximate']>[number][] = [];
  for (const span of [...(spans ?? [])].sort((a, b) => a.firstHour - b.firstHour)) {
    const last = union.at(-1);
    if (last && span.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, span.lastHour);
    else union.push({ ...span });
  }
  return union;
}

export const summaryFits = (frame: Pick<PlotFrame, 'x' | 'plotWidth'>, summary: { from: number; to: number; width: number; tiles?: unknown }, until: number | null) => summary.width === Math.ceil(frame.plotWidth)
  && (summary.tiles !== undefined || summary.to <= Math.min(frame.x[1], until ?? frame.x[1])) && summary.to >= frame.x[0] && summary.from <= frame.x[1];

export function summaryStack(parts: readonly ChartSummary[], total?: ChartSummary) {
  const summaries = total ? [...parts, total] : parts;
  const approximate = approximateUnion(summaries.flatMap(summary => summary.approximate ?? []));
  const samples = (bucket: ChartBucket) => [...new Map([
    [bucket.firstHour, bucket.firstValue], [bucket.minHour, bucket.minValue],
    [bucket.maxHour, bucket.maxValue], [bucket.lastValueHour, bucket.lastValue],
    [bucket.lastHour, bucket.lastValue],
  ]).entries()].sort(([a], [b]) => a - b).map(([, value]) => value);
  const sampled = summaries.map(summary => summary.buckets.map(bucket => bucket ? samples(bucket) : null));
  const points: { firstHour: number; lastHour: number; column: number; stage: number; count: number; breakBefore: boolean }[] = [];
  for (let column = 0; column < (summaries[0]?.buckets.length ?? 0); column++) {
    const buckets = summaries.map(summary => summary.buckets[column]);
    if (buckets.some(bucket => !bucket)) continue;
    let firstHour = -Infinity, lastHour = Infinity;
    for (const bucket of buckets) { firstHour = Math.max(firstHour, bucket!.firstHour); lastHour = Math.min(lastHour, bucket!.lastHour); }
    if (firstHour > lastHour) continue;
    const count = sampled.reduce((count, columns) => Math.max(count, columns[column]!.length), 1);
    const range = { firstHour, lastHour };
    const broken = buckets.some(bucket => bucket!.hasBreak);
    const firstBreak = buckets.some(bucket => bucket!.firstBreak);
    const hours = Array.from({ length: count }, (_, index) => count === 1 ? range.firstHour
      : range.firstHour + Math.floor((range.lastHour - range.firstHour) * index / (count - 1)));
    for (let stage = 0; stage < count; stage++) points.push({
      firstHour: hours[stage], lastHour: broken ? hours[stage] : (hours[stage + 1] ?? range.lastHour + 1) - 1,
      column, stage, count, breakBefore: broken || stage === 0 && firstBreak,
    });
  }
  const valuesAt = (point: typeof points[number]) => {
    const values = sampled.map(columns => {
      const values = columns[point.column]!;
      return values[point.count === 1 ? 0 : Math.floor(point.stage * (values.length - 1) / (point.count - 1))];
    });
    if (total) {
      const sum = { sum: 0, error: 0 };
      for (let index = 0; index < parts.length; index++) addStackAmount(sum, values[index]);
      values[parts.length] -= sum.sum + sum.error;
    }
    return values;
  };
  const hours = [...new Set(points.flatMap(point => [point.firstHour, point.lastHour, point.lastHour + 1]))].sort((a, b) => a - b);
  const model: CompactStackModel = { bands: [], hours, bounds: [], breakBefore: [] };
  for (const hour of hours) {
    const point = runAt(points, hour);
    if (!point) { model.bounds.push(null); model.breakBefore.push(false); continue; }
    const up = { sum: 0, error: 0 }, down = { sum: 0, error: 0 };
    for (const value of valuesAt(point)) addStackAmount(value >= 0 ? up : down, value);
    model.bounds.push([down.sum + down.error, up.sum + up.error]);
    model.breakBefore.push(point.firstHour === hour && point.breakBefore);
  }
  const sample = (hour: number) => {
    let low = 0, high = points.length;
    while (low < high) { const middle = (low + high) >> 1; if (points[middle].firstHour <= hour) low = middle + 1; else high = middle; }
    return stackEdges(valuesAt(points[Math.max(0, low - 1)]));
  };
  return { model, sample, approximate };
}
