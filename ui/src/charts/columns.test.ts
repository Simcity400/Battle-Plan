import assert from 'node:assert/strict';
import test from 'node:test';
import { CAP_HEADROOM, columnAt, columnEdges, columnsExtent, columnsLayer, columnStops } from './columns.ts';
import type { ColumnPeriod } from './columns';
import { plotFrame } from './frame.ts';
import { tokensFromValues } from './tokens.ts';

const TEST_TOKENS = tokensFromValues(() => '#888888', 'sans-serif');

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
  assert.deepEqual(columnEdges(periods[2]), [{ low: 0, high: 5 }, { low: -2, high: 0 }, { low: 5, high: 5 }]);
});

test('the value axis fits the columns in view, with zero', () => {
  assert.deepEqual(columnsExtent(periods, [0, 2159]), [-2, 6 + 8 * CAP_HEADROOM]);
  assert.deepEqual(columnsExtent(periods, [744, 1000]), [0, 5 + 5 * CAP_HEADROOM]);
  assert.deepEqual(columnsExtent([{ firstHour: 0, lastHour: 9, values: [-1_000_000, null, 0] }], [0, 9]), [-1_000_000, 1_000_000 * CAP_HEADROOM]);
  assert.deepEqual(columnsExtent([{ firstHour: 0, lastHour: 9, values: [3, -7, null] }], [0, 9]), [-7, 3 + 10 * CAP_HEADROOM]);
  assert.deepEqual(columnsExtent(periods, [1416, 2159]), [-2, 5 + 7 * CAP_HEADROOM]);
  assert.deepEqual(columnsExtent([{ firstHour: 0, lastHour: 9, values: [4, 0, 0] }], [0, 9]), [0, 4]);
  assert.equal(columnsExtent([{ firstHour: 0, lastHour: 9, values: [null] }], [0, 9]), null, 'an all-unknown period has no value extent');
  assert.equal(columnsExtent(periods, [5000, 6000]), null);
});

const months: ColumnPeriod[] = [
  { firstHour: 0, lastHour: 9, values: [3, 4] },
  { firstHour: 10, lastHour: 19, values: [], unknown: true },
  { firstHour: 20, lastHour: 29, values: [10, -2] },
];

test('a period not known at all has no extent, never zero, and stops stay at every period', () => {
  assert.deepEqual(columnsExtent(months, [0, 29]), [-2, 10]);
  assert.deepEqual(columnsExtent(months, [0, 15]), [0, 7]);
  assert.equal(columnsExtent(months, [10, 19]), null, 'an unknown period has no extent, not zero');
  assert.deepEqual(columnStops(months), [0, 10, 20]);
  assert.equal(columnAt([], 5), -1);
  const many = Array.from({ length: 5000 }, (_, i) => ({ firstHour: i * 10, lastHour: i * 10 + 9, values: [] }));
  assert.equal(columnAt(many, 49_995), 4999);
  assert.equal(columnAt(many, 12_345), 1234);
});

test('an unread month keeps main hatch heights by default and draws full plot height only with fullHeightUnknown', () => {
  const measure = (y: [number, number], style: { fullHeightUnknown?: boolean } = {}) => {
    const periodsWithUnknown: ColumnPeriod[] = [
      { firstHour: 0, lastHour: 9, values: [y[0]] },
      { firstHour: 10, lastHour: 19, values: [], unknown: true },
    ];
    const frame = plotFrame(200, 120, { left: 40, right: 10, top: 10, bottom: 20 }, [0, 20], y, false);
    let clippedHeight = 0;
    const fakeCtx = {
      fillStyle: '', strokeStyle: '', lineWidth: 0,
      save() {}, restore() {}, beginPath() {}, moveTo() {}, lineTo() {}, stroke() {}, fillRect() {},
      rect(_x: number, _y: number, _w: number, h: number) { clippedHeight = Math.max(clippedHeight, h); },
      clip() {},
    } as unknown as CanvasRenderingContext2D;
    columnsLayer(periodsWithUnknown, ['#888'], style).draw(fakeCtx, frame, TEST_TOKENS, { playhead: null });
    return { clippedHeight, plotHeight: frame.plotHeight };
  };
  const positive = measure([0, 10]);
  assert.equal(positive.clippedHeight, positive.plotHeight);
  const signed = measure([-10, 10]);
  assert.equal(signed.clippedHeight, signed.plotHeight / 2);
  const negative = measure([-10, 0]);
  assert.equal(negative.clippedHeight, 0);
  const fullNegative = measure([-10, 0], { fullHeightUnknown: true });
  assert.equal(fullNegative.clippedHeight, fullNegative.plotHeight);
  const fullSigned = measure([-10, 10], { fullHeightUnknown: true });
  assert.equal(fullSigned.clippedHeight, fullSigned.plotHeight);
});

test('a fresh columnsLayer per call reads zero offscreen column values during both extent and draw', () => {
  let offscreenReads = 0;
  const all: ColumnPeriod[] = Array.from({ length: 120 }, (_, index) => {
    const base: ColumnPeriod = { firstHour: index * 720, lastHour: (index + 1) * 720 - 1, values: [1, 2, 3] };
    return index < 119 ? new Proxy(base, {
      get(target, prop, receiver) {
        if (prop === 'values') offscreenReads++;
        return Reflect.get(target, prop, receiver);
      },
    }) : base;
  });
  const visible: [number, number] = [119 * 720, 120 * 720 - 1];
  const ext = columnsLayer(all, ['#111', '#222', '#333']).extent!(visible)!;
  assert.deepEqual(ext, [0, 6]);
  assert.equal(offscreenReads, 0, 'extent on a fresh layer reads zero offscreen column values');
  const frame = plotFrame(200, 120, { left: 40, right: 10, top: 10, bottom: 20 }, visible, ext, false);
  const fakeCtx = {
    fillStyle: '', strokeStyle: '', lineWidth: 0,
    save() {}, restore() {}, beginPath() {}, moveTo() {}, lineTo() {}, stroke() {}, fillRect() {}, rect() {}, clip() {},
  } as unknown as CanvasRenderingContext2D;
  columnsLayer(all, ['#111', '#222', '#333']).draw(fakeCtx, frame, TEST_TOKENS, { playhead: null });
  assert.equal(offscreenReads, 0, 'draw on a fresh layer reads zero offscreen column values');
});
