import assert from 'node:assert/strict';
import test from 'node:test';
import { exactRuns, summaryStack, type ChartBucket, type ChartSummary } from './chartSummary.ts';
import { drawBands } from './draw.ts';
import { plotFrame } from './frame.ts';
import { heldStackSegment } from './race/countryRace.ts';
import type { Run } from './series.ts';
import { compactStackExtent, forEachCompactStackSegment } from './stack.ts';
import type { ChartTokens } from './tokens.ts';

const stackBucket: ChartBucket = { firstHour: 0, lastHour: 31, firstValue: 2, lastValue: 3, lastValueHour: 30,
  minHour: 9, minValue: -7, maxHour: 21, maxValue: 11, firstBreak: false, hasBreak: false };
const stackSummary = (bucket: ChartBucket | null): ChartSummary => ({ from: 0, to: 31, width: 1, buckets: [bucket] });
const stackTotal = stackSummary({ ...stackBucket, firstValue: 5, lastValue: 5, minValue: 5, maxValue: 5 });

test('exactRuns returns supplied runs and null for dense summaries', () => {
  const runs: Run[] = [{ firstHour: 0, lastHour: 5, value: 10 }];
  const withRuns: ChartSummary = { from: 0, to: 5, width: 10, buckets: [], runs };
  assert.deepEqual(exactRuns(withRuns), runs);
  assert.equal(exactRuns({ from: 0, to: 5, width: 10, buckets: [] }), null);
});

test('D-12 retains all 5000 series extrema on a shared bounded grid through the existing signed renderer', () => {
  const parts = Array.from({ length: 5000 }, (_, index) => stackSummary({ ...stackBucket, minHour: 1 + index % 14, maxHour: 16 + index % 14 }));
  const dense = summaryStack(parts, stackTotal);
  assert.ok(dense.model.hours.length <= 15, 'grid size is independent of the number of extrema dates');
  assert.equal(dense.model.bands.length, 0, 'edges are allocated only for emitted screen points');
  const values = parts.map(() => new Set<number>());
  forEachCompactStackSegment(dense.model, 0, 31, 0, segment => {
    assert.equal(segment.bands.length, 5001);
    segment.bands.slice(0, -1).forEach((band, index) => {
      band.low.forEach((low, point) => values[index].add(band.high[point] <= 0 && low < 0 ? low - band.high[point] : band.high[point] - low));
    });
  }, dense.sample);
  for (const kept of values) assert.deepEqual(kept, new Set([2, -7, 11, 3]));
  const one = summaryStack([stackSummary(stackBucket)], stackTotal);
  assert.deepEqual(compactStackExtent(one.model, 0, 31), [-7, 12]);
});

test('the landed stack renderer isolates broken buckets and never connects a missing column', () => {
  const frame = plotFrame(320, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 31], [-10, 20], false);
  const calls: unknown[][] = [];
  const ctx = new Proxy({}, { get: (_target, key) => (...args: unknown[]) => calls.push([key, ...args]), set: () => true }) as CanvasRenderingContext2D;
  const broken = summaryStack([stackSummary({ ...stackBucket, hasBreak: true })], stackTotal);
  let pieces = 0;
  forEachCompactStackSegment(broken.model, 0, 31, 0, piece => {
    pieces++;
    assert.ok(piece.hours.at(-1)! - piece.hours[0] <= 1);
    const segment = heldStackSegment(piece);
    drawBands(ctx, frame, { lineWidth: { series: 2 } } as ChartTokens, segment.hours, segment.bands, ['#123', '#456'], segment);
  }, broken.sample);
  assert.equal(pieces, 5);
  assert.ok(calls.some(call => call[0] === 'fill'));
  const shift = (value: ChartBucket, offset: number): ChartBucket => ({ ...value, firstHour: value.firstHour + offset, lastHour: value.lastHour + offset,
    minHour: value.minHour + offset, maxHour: value.maxHour + offset, lastValueHour: value.lastValueHour + offset });
  const view: ChartSummary = { from: 0, to: 95, width: 3, buckets: [stackBucket, null, shift(stackBucket, 64)] };
  const dense = summaryStack([view], { ...view, buckets: [stackTotal.buckets[0], shift(stackTotal.buckets[0]!, 32), shift(stackTotal.buckets[0]!, 64)] });
  forEachCompactStackSegment(dense.model, 0, 95, 0, piece => assert.ok(piece.hours.at(-1)! <= 32 || piece.hours[0] >= 64), dense.sample);
});

test('a stack without a total retains named percentages and signed extrema without a residual band', () => {
  const constant = stackSummary({ ...stackBucket, firstValue: 35, lastValue: 35, minValue: 35, maxValue: 35 });
  const dense = summaryStack([stackSummary(stackBucket), constant]);
  const kept = new Set<number>();
  forEachCompactStackSegment(dense.model, 0, 31, 0, piece => {
    assert.equal(piece.bands.length, 2, 'every band is a supplied series');
    piece.bands[0].low.forEach((low, index) => { const high = piece.bands[0].high[index]; kept.add(low < 0 ? low - high : high - low); });
  }, dense.sample);
  assert.deepEqual(kept, new Set([2, -7, 11, 3]));
  assert.deepEqual(compactStackExtent(dense.model, 0, 31), [-7, 46]);
  const percentages = summaryStack([stackSummary({ ...stackBucket, firstValue: 40, lastValue: 40, minValue: 40, maxValue: 40 }), constant]);
  for (const hour of percentages.model.hours) if (percentages.model.bounds[percentages.model.hours.indexOf(hour)]) {
    assert.equal(percentages.sample(hour).at(-1)!.high, 75, 'recorded percentages are never normalized');
  }
  const unknown = summaryStack([stackSummary(null), constant]);
  assert.deepEqual(unknown.model.hours, []);
});

test('a dense stack draws a column only over the hours every part was recorded', () => {
  const part = (firstHour: number, lastHour: number, value: number) => stackSummary({ firstHour, lastHour, firstValue: value, lastValue: value,
    lastValueHour: lastHour, minHour: firstHour, minValue: value, maxHour: firstHour, maxValue: value, firstBreak: false, hasBreak: false });
  assert.deepEqual(summaryStack([part(0, 9, 10), part(20, 29, 5)]).model.hours, []);
  const overlap = summaryStack([part(0, 19, 10), part(10, 29, 5)]);
  const drawn = overlap.model.hours.filter((_, index) => overlap.model.bounds[index]);
  assert.ok(drawn.length > 0);
  assert.ok(drawn.every(hour => hour >= 10 && hour <= 19), `${drawn}`);
  for (const bound of overlap.model.bounds) if (bound) assert.deepEqual(bound, [0, 15]);
});

test('stack summaries carry the union of part and total approximate spans without changing geometry', () => {
  const a = stackSummary(stackBucket), b = stackSummary({ ...stackBucket, firstValue: 3, lastValue: 3, minValue: 3, maxValue: 3 });
  const exact = summaryStack([a, b], stackTotal);
  const spans = [{ firstHour: 8, lastHour: 12 }, { firstHour: 2, lastHour: 5 }];
  const stacked = summaryStack([{ ...a, approximate: spans }, { ...b, approximate: [{ firstHour: 5, lastHour: 9 }, { firstHour: 20, lastHour: 21 }] }],
    { ...stackTotal, approximate: [{ firstHour: 13, lastHour: 15 }] });
  assert.deepEqual(stacked.approximate, [{ firstHour: 2, lastHour: 15 }, { firstHour: 20, lastHour: 21 }]);
  assert.deepEqual(stacked.model, exact.model);
  for (const hour of exact.model.hours) if (exact.model.bounds[exact.model.hours.indexOf(hour)]) assert.deepEqual(stacked.sample(hour), exact.sample(hour));
  assert.deepEqual(spans, [{ firstHour: 8, lastHour: 12 }, { firstHour: 2, lastHour: 5 }], 'a summary owned by another chart is not mutated');
  assert.deepEqual(exact.approximate, []);
});
