import assert from 'node:assert/strict';
import test from 'node:test';
import { AXIS_HEIGHT, ROW_HEIGHT, activateLane, barGlyphHours, barHours, barOpenEnd, laneAt, laneLabelY, laneRowAt, laneRows, lanesInView, laneStops, laneTop, spanAtRow } from './layout.ts';
import type { TimelineLane, TimelineSpan } from './layout.ts';

const span = (id: string, subrow = 0): TimelineSpan => ({ id, firstHour: 10, lastHour: 100, label: id, subrow });
const lane = (id: string, subrows = 1): TimelineLane => ({ id, label: id, spans: [], subrows });

test('ordinary lane geometry remains identical at every boundary and scroll position', () => {
  const lanes = Array.from({ length: 5000 }, (_, index) => lane(`${index}`));
  const rows = laneRows(lanes);
  assert.equal(rows.height, lanes.length * ROW_HEIGHT);
  for (const offset of [0, 1, 40, 41, 8000, 8000.5, rows.height - 1, rows.height]) {
    assert.deepEqual(lanesInView(offset, 360, lanes.length, rows), lanesInView(offset, 360, lanes.length));
    assert.equal(laneAt(AXIS_HEIGHT + offset, lanes.length, rows), laneAt(AXIS_HEIGHT + offset, lanes.length));
  }
  assert.equal(laneAt(AXIS_HEIGHT - 1, lanes.length, rows), null);
  assert.deepEqual(lanesInView(0, 24, 0, laneRows([])), [0, -1]);
});

test('tall lanes reserve distinct sub-rows and viewport lookup follows their prefixes', () => {
  const lanes = [lane('first'), lane('simultaneous', 3), lane('last')];
  const rows = laneRows(lanes);
  assert.deepEqual([...rows.offsets], [0, 40, 160, 200]);
  assert.equal(laneTop(1, rows), 64);
  assert.equal(laneTop(2, rows), 184);
  assert.deepEqual(lanesInView(40, 104, 3, rows), [1, 1]);
  assert.deepEqual(lanesInView(159, 26, 3, rows), [1, 2]);
  assert.equal(laneAt(183, 3, rows), 1);
  assert.equal(laneAt(184, 3, rows), 2);
  assert.equal(laneRowAt(63, 1, rows), null);
  assert.equal(laneRowAt(64, 1, rows), 0);
  assert.equal(laneRowAt(104, 1, rows), 1);
  assert.equal(laneRowAt(144, 1, rows), 2);
  assert.equal(laneRowAt(184, 1, rows), null);
});

test('compact tree rows keep their chosen heights without changing ordinary rows', () => {
  const lanes = [{ ...lane('war'), rowHeight: 30 }, { ...lane('side'), rowHeight: 18 }, { ...lane('participant', 2), rowHeight: 22 }, lane('ordinary')];
  const rows = laneRows(lanes);
  assert.deepEqual([...rows.rowHeights], [30, 18, 22, 40]);
  assert.deepEqual([...rows.offsets], [0, 30, 48, 92, 132]);
  assert.equal(laneRowAt(AXIS_HEIGHT + 70, 2, rows), 1);
  assert.equal(laneAt(AXIS_HEIGHT + 92, lanes.length, rows), 3);
});

test('a country with hundreds of simultaneous bars keeps its name beside the rows in view', () => {
  const rows = laneRows([lane('country', 500), lane('ordinary')]);
  assert.equal(laneLabelY(0, rows, 0, 360), 340);
  assert.equal(laneLabelY(0, rows, 15000, 360), 44);
  assert.equal(laneLabelY(1, rows, 19900, 360), 144);
});

test('simultaneous bars each have their own pointer and keyboard target', () => {
  const spans = [span('second', 1), span('first', 0), span('third', 2)];
  assert.equal(spanAtRow(spans, 50, 0)?.id, 'first');
  assert.equal(spanAtRow(spans, 50, 1)?.id, 'second');
  assert.equal(spanAtRow(spans, 50, 2)?.id, 'third');
  assert.equal(spanAtRow(spans, 50, 3), null);
  assert.equal(spanAtRow(spans, 101, 0), null);
  assert.deepEqual(laneStops(spans, 0, 200, null).map(stop => [stop.span.id, stop.hour]), [['first', 10], ['second', 10], ['third', 10]]);
  assert.deepEqual(laneStops(spans, 40, 70, 60).map(stop => [stop.span.id, stop.hour]), [['first', 40], ['second', 40], ['third', 40]]);
});

test('a positioned event mark never backfills unknown hours and is reachable by keyboard', () => {
  const event = { ...span('participant'), glyph: '✕', glyphHour: 60 };
  assert.deepEqual(barGlyphHours(event, null), [60]);
  assert.deepEqual(barGlyphHours(event, 59), []);
  assert.deepEqual(barGlyphHours(event, 60), [60]);
  assert.deepEqual(barGlyphHours({ ...event, glyphHour: 101 }, null), []);
  assert.deepEqual(barGlyphHours({ ...event, glyphHour: 9 }, null), []);
  const first = { ...event, glyphHour: event.firstHour };
  assert.equal(barHours(first, first.firstHour), null);
  assert.deepEqual(barGlyphHours(first, first.firstHour), [first.firstHour]);
  assert.deepEqual(laneStops([event], 0, 200, null).map(stop => stop.hour), [10, 60]);
  assert.deepEqual(laneStops([event], 20, 50, null).map(stop => stop.hour), [20]);
  assert.deepEqual(laneStops([event], 0, 200, 59).map(stop => stop.hour), [10]);
  // Existing start glyphs keep the same bar stop.
  assert.deepEqual(barGlyphHours({ ...span('leader'), glyph: '★' }, null), [10]);
});

test('an ongoing bar opens only where Live cuts observed hours and never extends its end', () => {
  const ongoing = { ...span('war'), openEnd: true };
  assert.equal(barOpenEnd(ongoing, null), false);
  assert.equal(barOpenEnd(ongoing, 10), false);
  assert.equal(barOpenEnd(ongoing, 60), true);
  assert.equal(barOpenEnd(ongoing, 101), false);
  assert.equal(barOpenEnd({ ...ongoing, openEnd: false }, 60), false);
  assert.deepEqual(barHours(ongoing, 60), [10, 60]);
  assert.deepEqual(barHours(ongoing, 150), [10, 101]);
});

test('several cross events keep one bar and independent keyboard stops, including its first Live hour', () => {
  const events = { ...span('run'), glyphShape: 'cross' as const, glyphHours: [10, 30, 60, 101] };
  assert.deepEqual(barGlyphHours(events, 10), [10]);
  assert.deepEqual(barGlyphHours(events, 40), [10, 30]);
  assert.deepEqual(laneStops([events], 0, 200, null).map(stop => stop.hour), [10, 30, 60]);
  assert.deepEqual(laneStops([events], 0, 200, 10).map(stop => stop.hour), [10]);
  assert.deepEqual(laneStops([events], 20, 50, null).map(stop => stop.hour), [20, 30]);
});

test('only expandable lanes activate, for click, Enter and Space', () => {
  const activated: string[] = [];
  const activate = (item: TimelineLane) => activated.push(item.id);
  assert.equal(activateLane(lane('ordinary'), 'Enter', activate), false);
  assert.equal(activateLane(undefined, 'Enter', activate), false);
  assert.equal(activateLane({ ...lane('war'), expanded: false }, 'ArrowRight', activate), false);
  for (const key of ['click', 'Enter', ' ']) assert.equal(activateLane({ ...lane(key), expanded: false }, key, activate), true);
  assert.equal(activateLane({ ...lane('open'), expanded: true }, 'Enter', activate), true);
  assert.deepEqual(activated, ['click', 'Enter', ' ', 'open']);
});

test('prefix geometry and coincident keyboard stops retain all 5000 and 20000 countries', () => {
  for (const count of [5000, 20000]) {
    const lanes = Array.from({ length: count }, (_, index) => lane(`${index}`, 1 + index % 4));
    const rows = laneRows(lanes);
    assert.equal(rows.offsets.length, lanes.length + 1);
    assert.equal(rows.height, ROW_HEIGHT * count / 4 * 10);
    assert.equal(laneAt(AXIS_HEIGHT + rows.height - 1, lanes.length, rows), lanes.length - 1);
    const spans = lanes.map((item, index) => span(item.id, index));
    const stops = laneStops(spans, 0, 200, null);
    assert.equal(stops.length, spans.length);
    assert.deepEqual(stops.map(stop => stop.span.id), spans.map(item => item.id));
  }
});
