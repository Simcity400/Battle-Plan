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

test('an unread month draws full plot height even when measured values are negative', () => {
  const negativeAndUnread: ColumnPeriod[] = [
    { firstHour: 0, lastHour: 9, values: [-10] },
    { firstHour: 10, lastHour: 19, values: [], unknown: true },
  ];
  const frame = plotFrame(200, 120, { left: 40, right: 10, top: 10, bottom: 20 }, [0, 20], [-10, 0], false);
  let clippedHeight = 0;
  const fakeCtx = {
    fillStyle: '', strokeStyle: '', lineWidth: 0,
    save() {}, restore() {}, beginPath() {}, moveTo() {}, lineTo() {}, stroke() {}, fillRect() {},
    rect(_x: number, _y: number, _w: number, h: number) { clippedHeight = Math.max(clippedHeight, h); },
    clip() {},
  } as unknown as CanvasRenderingContext2D;
  columnsLayer(negativeAndUnread, ['#888']).draw(fakeCtx, frame, TEST_TOKENS, { playhead: null });
  assert.equal(clippedHeight, frame.plotHeight, 'unread hatch spans full plot height rather than collapsing to zero');
});




