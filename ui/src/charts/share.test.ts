import assert from 'node:assert/strict';
import test from 'node:test';
import { SHARE_TICKS, sharePixels, shareSegments, shareStack, summaryShareRuns } from './share.ts';
import type { ChartSummary } from './chartSummary';
import { valueTicks } from './draw.ts';
import { plotFrame } from './frame.ts';
import { tokensFromValues } from './tokens.ts';

test('prepared share geometry fills dense complete buckets, preserves gaps and leaves exact counts unchanged', () => {
  const runs = [{ firstHour: 0, lastHour: 0, counts: [1, 0, 0], startsSegment: true },
    { firstHour: 4, lastHour: 4, counts: [0, 1, 0] }, { firstHour: 9, lastHour: 9, counts: [0, 0, 1] }];
  const before = structuredClone(runs);
  const summary: ChartSummary = { from: 0, to: 9, width: 1, buckets: [{ firstHour: 0, lastHour: 9,
    firstValue: 1, lastValue: 0, lastValueHour: 9, minHour: 4, minValue: 0, maxHour: 0, maxValue: 1, firstBreak: true, hasBreak: false }] };
  assert.deepEqual(summaryShareRuns(runs, [summary]).map(run => [run.firstHour, run.lastHour]), [[0, 3], [4, 8], [9, 9]]);
  const broken = { ...summary, buckets: summary.buckets.map(bucket => bucket && { ...bucket, hasBreak: true }) };
  assert.deepEqual(summaryShareRuns(runs, [broken]).map(run => [run.firstHour, run.lastHour]), [[0, 0], [4, 4], [9, 9]]);
  assert.equal(summaryShareRuns(runs, [{ ...summary, runs: [] }]), runs);
  assert.deepEqual(runs, before);
});

test('stacked shares retain internal changes, held values, identity breaks and missing hours', () => {
  const stack = shareStack([
    { firstHour: 1, lastHour: 2, counts: [1, 2, 1], startsSegment: true },
    { firstHour: 3, lastHour: 4, counts: [3, 0, 1] },
    { firstHour: 6, lastHour: 7, counts: [0, 0, 4], startsSegment: true },
    { firstHour: 8, lastHour: 9, counts: [0, 4, 0], startsSegment: true },
  ], 3);
  assert.deepEqual(stack.edges[0], [{ low: 0, high: 25 }, { low: 25, high: 75 }, { low: 75, high: 100 }]);
  const segments = shareSegments(stack, 1, 9);
  assert.equal(segments.length, 3);
  const at = segments[0].hours.indexOf(3);
  assert.equal(segments[0].bands[0].high[at], 25);
  assert.equal(segments[0].bands[0].high[at + 1], 75);
  assert.equal(stack.edges[stack.hours.indexOf(5)], null);
  assert.deepEqual(shareSegments(stack, 6, 5), []);
});

test('zero denominators are gaps and malformed counts are refused', () => {
  assert.deepEqual(shareStack([{ firstHour: 1, lastHour: 2, counts: [0, 0, 0] }], 3).hours, []);
  assert.throws(() => shareStack([{ firstHour: 1, lastHour: 2, counts: [0, 1] }], 3));
  assert.throws(() => shareStack([{ firstHour: 1, lastHour: 2, counts: [-1, 1, 2] }], 3));
});

test('pixel thinning retains internal share peaks, gaps, identities and exact model values', () => {
  const runs = Array.from({ length: 2_000 }, (_, hour) => ({ firstHour: hour, lastHour: hour,
    counts: hour === 71 ? [100, 0, 0] : [0, 0, 100], startsSegment: hour === 1_001 }));
  runs.splice(1_000, 1);
  const model = shareStack(runs, 3);
  const exact = shareSegments(model, 0, 1_999);
  const thinned = shareSegments(model, 0, 1_999, 20);
  assert.equal(thinned.length, exact.length);
  assert.ok(thinned.flatMap(segment => segment.hours).length < exact.flatMap(segment => segment.hours).length / 2);
  assert.ok(thinned.some(segment => segment.bands[0].high.includes(100)));
  assert.equal(model.edges[model.hours.indexOf(1_000)], null);
  assert.equal(model.edges[model.hours.indexOf(71)]![0].high, 100);
});

test('share axes keep quarters at every chart height and when zoomed', () => {
  const tokens = tokensFromValues(() => 'theme', 'font');
  for (const height of [150, 300, 800]) {
    const frame = plotFrame(400, height, { left: 20, right: 20, top: 20, bottom: 20 }, [1, 8], [0, 100], false);
    frame.valueTicks = SHARE_TICKS;
    assert.deepEqual(valueTicks(frame, tokens), SHARE_TICKS);
    frame.y = [20, 80];
    assert.deepEqual(valueTicks(frame, tokens), [25, 50, 75]);
  }
});





const colors = new Uint8ClampedArray([255, 0, 0, 255, 255, 255, 0, 255, 0, 255, 0, 217]);
const pixels = { createImageData(width: number, height: number) {
  return { width, height, data: new Uint8ClampedArray(width * height * 4) };
} } as unknown as CanvasRenderingContext2D;


test('dense device columns retain each state contribution and the exact fractional gap coverage', () => {
  const frame = plotFrame(8, 8, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 16], [0, 100], false);
  const model = shareStack([
    { firstHour: 0, lastHour: 0, counts: [4, 0, 0] },
    { firstHour: 1, lastHour: 1, counts: [0, 0, 4] },
  ], 3);
  const mixed = sharePixels(pixels, model, frame, colors);
  for (let row = 0; row < 8; row++) {
    assert.deepEqual([...mixed.data.slice(row * 32, row * 32 + 4)], [138, 117, 0, 236]);
    assert.deepEqual([...mixed.data.slice(row * 32 + 4, (row + 1) * 32)], Array(28).fill(0));
  }
  const gaps = sharePixels(pixels, shareStack([
    { firstHour: 0, lastHour: 0, counts: [4, 0, 0] },
    { firstHour: 2, lastHour: 2, counts: [0, 0, 4] },
  ], 3), frame, colors);
  assert.deepEqual([...gaps.data.slice(0, 8)], [255, 0, 0, 128, 0, 255, 0, 108]);
  const zoomed = plotFrame(8, 8, frame.padding, frame.x, [25, 75], false);
  const middle = sharePixels(pixels, shareStack([{ firstHour: 0, lastHour: 0, counts: [1, 2, 1] }], 3), zoomed, colors);
  assert.deepEqual([...middle.data.slice(0, 4)], [255, 255, 0, 128]);
  assert.deepEqual([...middle.data.slice(-32, -28)], [255, 255, 0, 128]);
});

test('dense pixel preparation reads only the requested columns; zero-width bands never gain area', () => {
  const model = shareStack(Array.from({ length: 4_000 }, (_, hour) => ({ firstHour: hour, lastHour: hour, counts: [1, 0, 1] })), 3);
  const readHours: number[] = [];
  model.edges = new Proxy(model.edges, { get(target, key, receiver) {
    if (typeof key === 'string' && /^\d+$/.test(key)) readHours.push(model.hours[Number(key)]);
    return Reflect.get(target, key, receiver);
  } });
  const frame = plotFrame(80, 8, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 4_000], [0, 100], false);
  const image = sharePixels(pixels, model, frame, colors, 79);
  assert.equal(image.width, 1);
  assert.equal(readHours.length, 50);
  assert.ok(readHours.every(hour => hour >= 3_950));
  assert.deepEqual([...image.data.slice(0, 4)], [0, 255, 0, 217]);
  assert.deepEqual([...image.data.slice(-4)], [255, 0, 0, 255]);
});



