import assert from 'node:assert/strict';
import test from 'node:test';
import { CAP_HEADROOM, columnAt, columnBox, columnEdges, columnsExtent, columnsLayer } from './columns.ts';
import type { ColumnPeriod } from './columns';
import { plotFrame } from './frame.ts';
import type { ChartTokens } from './tokens';

const periods: ColumnPeriod[] = [
  { firstHour: 0, lastHour: 743, values: [2, 3, 1] },
  { firstHour: 744, lastHour: 1415, values: [4, null, 1] },
  { firstHour: 1416, lastHour: 2159, values: [5, -2, 0], uncertain: [true, false, false] },
];

test('a period is found by its hours, and an hour between periods finds none', () => {
  assert.equal(columnAt(periods, 0), 0);
  assert.equal(columnAt(periods, 744), 1);
  assert.equal(columnAt(periods, 2159), 2);
  assert.equal(columnAt(periods, 2160), -1);
  assert.equal(columnAt([{ firstHour: 0, lastHour: 9, values: [] }, { firstHour: 20, lastHour: 29, values: [] }], 15), -1);
});

test('bands stack in order, signed, and an unknown band is never stacked as zero', () => {
  assert.deepEqual(columnEdges(periods[0]), [{ low: 0, high: 2 }, { low: 2, high: 5 }, { low: 5, high: 6 }]);
  assert.deepEqual(columnEdges(periods[1]), [{ low: 0, high: 4 }, null, { low: 4, high: 5 }]);
  // A correction stacks down from zero.
  assert.deepEqual(columnEdges(periods[2]), [{ low: 0, high: 5 }, { low: -2, high: 0 }, { low: 5, high: 5 }]);
});

test('the value axis fits the columns in view, with zero', () => {
  // An unknown band in view keeps room above the stacks for its cap.
  assert.deepEqual(columnsExtent(periods, [0, 2159]), [-2, 6 + 8 * CAP_HEADROOM]);
  assert.deepEqual(columnsExtent(periods, [744, 1000]), [0, 5 + 5 * CAP_HEADROOM]);
  // Negative-only and mixed-sign stacks keep room from their whole span.
  assert.deepEqual(columnsExtent([{ firstHour: 0, lastHour: 9, values: [-1_000_000, null, 0] }], [0, 9]), [-1_000_000, 1_000_000 * CAP_HEADROOM]);
  assert.deepEqual(columnsExtent([{ firstHour: 0, lastHour: 9, values: [3, -7, null] }], [0, 9]), [-7, 3 + 10 * CAP_HEADROOM]);
  // An uncertain band keeps the same room for its marker.
  assert.deepEqual(columnsExtent(periods, [1416, 2159]), [-2, 5 + 7 * CAP_HEADROOM]);
  assert.deepEqual(columnsExtent([{ firstHour: 0, lastHour: 9, values: [4, 0, 0] }], [0, 9]), [0, 4]);
  assert.deepEqual(columnsExtent([{ firstHour: 0, lastHour: 9, values: [null] }], [0, 9]), [0, 1]);
  assert.equal(columnsExtent(periods, [5000, 6000]), null);
});

test('columns span their hours and keep a gap only once wide enough', () => {
  const wide = plotFrame(400, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 2160], [0, 10], false);
  const box = columnBox(periods[0], wide);
  assert.equal(box.left, 1);
  assert.ok(Math.abs(box.left + box.width + 1 - wide.toX(744)) < 1e-9);
  const narrow = plotFrame(40, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 21600], [0, 10], false);
  assert.equal(columnBox(periods[0], narrow).left, 0);
  assert.ok(columnBox(periods[0], narrow).width >= 1);
});

function recorder() {
  const calls: string[] = [];
  const ctx = new Proxy({}, {
    get: (_target, name) => typeof name === 'string' && ['fillStyle', 'strokeStyle', 'lineWidth', 'globalAlpha'].includes(name)
      ? undefined : (...args: unknown[]) => { calls.push(`${String(name)}(${args.map(arg => typeof arg === 'number' ? Math.round(arg) : arg).join(',')})`); },
    set: () => true,
  }) as unknown as CanvasRenderingContext2D;
  return { ctx, calls };
}

test('drawing stops at the playhead and caps a column with an unknown band', () => {
  const tokens = { muted: '#888', hatchSpacing: 4, lineWidth: { grid: 1 } } as ChartTokens;
  const frame = plotFrame(300, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 2160], [0, 10], false);
  const { ctx, calls } = recorder();
  columnsLayer(periods, ['#a', '#b', '#c']).draw(ctx, frame, tokens, { playhead: 800 });
  const fills = calls.filter(call => call.startsWith('fillRect'));
  // The first column's three bands, the second's two known bands and the
  // hatched cap's wash above them; the third lies after the playhead.
  const capWash = fills.filter(call => call.endsWith(',12)'));
  assert.equal(capWash.length, 1);
  assert.equal(fills.length, 6);
});

test('the unknown cap stays visible on the tallest column at the fitted extent', () => {
  const tokens = { muted: '#888', hatchSpacing: 4, lineWidth: { grid: 1 } } as ChartTokens;
  const lone = [{ firstHour: 0, lastHour: 743, values: [5, null, 0] }];
  const frame = plotFrame(300, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 744], columnsExtent(lone, [0, 744])!, false);
  const { ctx, calls } = recorder();
  columnsLayer(lone, ['#a', '#b', '#c']).draw(ctx, frame, tokens, { playhead: null });
  const fills = calls.filter(call => call.startsWith('fillRect'));
  // The known band and a cap of nonzero height above it.
  assert.equal(fills.length, 2);
  const cap = fills[1].slice('fillRect('.length, -1).split(',').map(Number);
  assert.ok(cap[3] > 0, `the cap has height (${fills[1]})`);
});

test('the unknown cap stays visible over a negative-dominated column at the fitted extent', () => {
  const tokens = { muted: '#888', hatchSpacing: 4, lineWidth: { grid: 1 } } as ChartTokens;
  const negative = [{ firstHour: 0, lastHour: 743, values: [-1_000_000, null, 0] }];
  const frame = plotFrame(300, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 744], columnsExtent(negative, [0, 744])!, false);
  const { ctx, calls } = recorder();
  columnsLayer(negative, ['#a', '#b', '#c']).draw(ctx, frame, tokens, { playhead: null });
  const fills = calls.filter(call => call.startsWith('fillRect'));
  const cap = fills[1].slice('fillRect('.length, -1).split(',').map(Number);
  assert.ok(cap[3] >= 8, `the cap keeps visible height above zero (${fills[1]})`);
});

test('a known band that is uncertain but draws no height keeps a visible marker', () => {
  const tokens = { muted: '#888', hatchSpacing: 4, lineWidth: { grid: 1 } } as ChartTokens;
  const zero = [{ firstHour: 0, lastHour: 743, values: [0, 0, 0], uncertain: [true, true, false] }];
  const frame = plotFrame(300, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 744], columnsExtent(zero, [0, 744])!, false);
  const { ctx, calls } = recorder();
  columnsLayer(zero, ['#a', '#b', '#c']).draw(ctx, frame, tokens, { playhead: null });
  const fills = calls.filter(call => call.startsWith('fillRect'));
  assert.equal(fills.length, 1, 'only the marker draws');
  const mark = fills[0].slice('fillRect('.length, -1).split(',').map(Number);
  assert.ok(mark[3] > 0, `the marker has height (${fills[0]})`);
});
