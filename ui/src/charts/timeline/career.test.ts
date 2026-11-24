import assert from 'node:assert/strict';
import test from 'node:test';
import { AXIS_HEIGHT, barHitAt, escapeDismissesTooltip, joinedStep, laneAt, laneRows, laneStops, laneTop, markerAt, periodBarHeight, pointerBarSpans, spanAt, spanAtRow, spanY, stackedSpanAt, stackedSpans } from './layout.ts';
import type { TimelineLane, TimelineSpan } from './layout';

const span = (id: string, firstHour: number, lastHour: number): TimelineSpan => ({ id, firstHour, lastHour, label: id });

test('observed steps join consecutive hours only and missing period counts stay distinct from zero', () => {
  const first = { ...span('first', 1, 2), value: 2 };
  assert.equal(joinedStep(first, { ...span('next', 3, 4), value: 3 }), true);
  assert.equal(joinedStep(first, { ...span('gap', 4, 4), value: 3 }), false);
  assert.equal(joinedStep(first, span('missing', 3, 4)), false);
  assert.equal(periodBarHeight(2, 4), 12);
  assert.equal(periodBarHeight(0, 4), 0);
  for (const value of [null, -1, NaN, Infinity]) assert.equal(periodBarHeight(value, 4), null);
  assert.equal(periodBarHeight(2, NaN), null);
});

test('dated icons retain their actual date and row through campaign zoom and Live cutoff', () => {
  const marker = { ...span('event', 100, 100), marker: true, icons: [undefined, undefined], subrow: 1 };
  assert.equal(markerAt([marker], 21, h => h / 10, 0, 101, 1), marker);
  assert.equal(markerAt([marker], 21, h => h / 10, 0, 101, 0), null);
  assert.equal(markerAt([marker], 21, h => h / 10, 0, 100), null);
  assert.equal(markerAt([marker], 33, h => h / 10, 0, 101), null);
  assert.equal(laneStops([marker], 100.5, 200, null).length, 0);
  assert.equal(laneStops([marker], 0, 200, 100).length, 0);
  assert.equal(laneStops([marker], 0, 200, 101)[0].hour, 100);
  assert.equal(spanAt([marker], 100), null);
  const holder = { ...span('holder', 90, 110), glyphShape: 'cross' as const, glyphHours: [100] };
  assert.equal(spanAt([marker, holder], 100), holder);
  assert.equal(spanAtRow([marker], 100, 1), null);
  assert.equal(barHitAt([marker], 100, null, h => h, x => x, 0, 200, 1), null);
});

test('command stacking reuses a row only after its last hour and shares root prefix geometry', () => {
  const commands = stackedSpans([span('a', 1, 5), span('b', 2, 3), span('c', 5, 7), span('d', 8, 9)]);
  assert.deepEqual(commands.map(item => item.stack), [0, 1, 1, 0]);
  const simultaneous = stackedSpans(['a', 'b', 'c', 'd'].map(id => span(id, 1, 9)));
  const ordinary = { ...span('ordinary', 1, 9), subrow: 2 };
  const lanes: TimelineLane[] = [
    { id: 'commands', label: 'Commands', spans: simultaneous },
    { id: 'subrows', label: 'Subrows', spans: [ordinary], rowHeight: 30, subrows: 3 },
    { id: 'tiny', label: 'Tiny', spans: [], rowHeight: 1 },
  ];
  const rows = laneRows(lanes);
  assert.deepEqual([...rows.offsets], [0, 72, 162, 163]);
  assert.equal(laneTop(1, rows), AXIS_HEIGHT + 72);
  assert.equal(laneAt(AXIS_HEIGHT + 71, lanes.length, rows), 0);
  assert.equal(laneAt(AXIS_HEIGHT + 72, lanes.length, rows), 1);
  assert.equal(spanY(ordinary, 1, rows, lanes[1].spans), AXIS_HEIGHT + 72 + 75);
  for (const [index, item] of simultaneous.entries()) {
    const centre = spanY(item, 0, rows, simultaneous);
    assert.equal(centre, AXIS_HEIGHT + 12 + 16 * index);
    const offset = centre - AXIS_HEIGHT - 36;
    assert.equal(stackedSpanAt(simultaneous, 5, offset), item);
    assert.equal(barHitAt(pointerBarSpans(simultaneous, offset), 5, null, h => h, x => x, 0, 20, 1)?.span, item);
  }
  assert.equal(stackedSpanAt(simultaneous, 10, 24), null);
  assert.equal(spanAtRow(simultaneous, 5, 0), null);
  assert.deepEqual(laneStops(simultaneous, 0, 20, null).map(stop => stop.span.id), ['a', 'b', 'c', 'd']);
});


test('ordinary pointer arrays are reused, markers excluded, and pointer Escape is opt-in', () => {
  const spans = [span('plain', 1, 3)];
  assert.equal(pointerBarSpans(spans, 0), spans);
  assert.equal(pointerBarSpans(spans, 10), spans);
  const marked = [...spans, { ...span('marker', 2, 2), marker: true }];
  assert.deepEqual(pointerBarSpans(marked, 0), spans);
  assert.equal(pointerBarSpans(marked, 0), pointerBarSpans(marked, 10));
  assert.equal(escapeDismissesTooltip(false), false);
  assert.equal(escapeDismissesTooltip(false, true), true);
  assert.equal(escapeDismissesTooltip(true), true);
});
