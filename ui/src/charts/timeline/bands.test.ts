import assert from 'node:assert/strict';
import test from 'node:test';
import { barExtent, barHitAt, bandHitAt, laneBands, laneStops } from './layout.ts';

const toX = (hour: number) => hour;
const same = (value: number) => value;

test('the keyboard stops at every band step in view, so each value a pointer reaches is reachable', () => {
  const span = { id: 'line', firstHour: 0, lastHour: 47, label: 'Line', band: [{ firstHour: 0, lastHour: 23, value: 1 }, { firstHour: 24, lastHour: 47, value: 8 }] };
  assert.deepEqual(laneStops([span], 0, 100, null).map(stop => [stop.id, stop.hour]), [['line@0', 0], ['line@24', 24]]);
  assert.deepEqual(laneStops([span], 10.5, 100, null).map(stop => [stop.id, stop.hour]), [['line@0', 10.5], ['line@24', 24]]);
  assert.deepEqual(laneStops([span], 23.2, 100, null).map(stop => [stop.id, stop.hour]), [['line@0', 23.2], ['line@24', 24]]);
  assert.deepEqual(laneStops([span], 0, 100, 20).map(stop => stop.hour), [0]);
  const bar = { id: 'bar', firstHour: 50, lastHour: 60, label: 'Bar' };
  assert.deepEqual(laneStops([span, bar], 0, 100, null).map(stop => stop.id), ['line@0', 'line@24', 'bar@50']);
  assert.deepEqual(laneStops([bar], 55, 100, null).map(stop => [stop.id, stop.hour]), [['bar@50', 55]]);
  const holed = { ...span, band: [{ firstHour: 0, lastHour: 9, value: 1 }, { firstHour: 30, lastHour: 47, value: 8 }] };
  assert.deepEqual(laneStops([holed], 20, 100, null).map(stop => stop.hour), [30]);
});

test('a span also stops at its own hours: readings the bar does not show, a count published at its end', () => {
  const reads = [100, 124, 148, 172, 196];
  const line = { id: 'line', firstHour: 100, lastHour: 171, label: 'Line', band: [{ firstHour: 100, lastHour: 171, value: 3 }], stops: reads };
  assert.deepEqual(laneStops([line], 0, 500, null).map(stop => [stop.id, stop.hour]), [['line@100', 100], ['line@124', 124], ['line@148', 148]]);
  assert.deepEqual(laneStops([line], 130, 500, null).map(stop => [stop.id, stop.hour]), [['line@124', 130], ['line@148', 148]]);
  const between = { id: 'between', firstHour: 101, lastHour: 124, label: 'Made between', stops: [124] };
  assert.deepEqual(laneStops([between], 0, 500, null).map(stop => [stop.id, stop.hour]), [['between@101', 101], ['between@124', 124]]);
  const holed = { ...line, band: [{ firstHour: 100, lastHour: 110, value: 3 }, { firstHour: 124, lastHour: 171, value: 5 }], stops: [115, 124] };
  assert.deepEqual(laneStops([holed], 0, 500, null).map(stop => stop.hour), [100, 124]);
});

test('a lane places its band spans together, so a gap between two spans stays visible', () => {
  const spans = [
    { id: 'a', firstHour: 8, lastHour: 8, label: 'A', band: [{ firstHour: 8, lastHour: 8, value: 3 }] },
    { id: 'b', firstHour: 28, lastHour: 28, label: 'B', band: [{ firstHour: 28, lastHour: 28, value: 3 }] },
  ];
  const bands = laneBands(spans, null, hour => 120 + hour / 16, same, 120, 620);
  assert.deepEqual(bands.outlines.get('a'), [[{ x0: 120, x1: 121, half: 3 }]]);
  assert.deepEqual(bands.outlines.get('b'), [[{ x0: 122, x1: 123, half: 3 }]]);
  const touching = [
    { id: 'a', firstHour: 0, lastHour: 9, label: 'A', band: [{ firstHour: 0, lastHour: 9, value: 2 }] },
    { id: 'b', firstHour: 10, lastHour: 19, label: 'B', band: [{ firstHour: 10, lastHour: 19, value: 2 }] },
  ];
  const joined = laneBands(touching, null, toX, same, 0, 1000);
  assert.deepEqual(joined.outlines.get('a'), [[{ x0: 0, x1: 10, half: 2 }]]);
  assert.deepEqual(joined.outlines.get('b'), [[{ x0: 10, x1: 20, half: 2 }]]);
  const cut = laneBands([...touching, { id: 'bar', firstHour: 30, lastHour: 40, label: 'Bar' }], 15, toX, same, 0, 1000);
  assert.deepEqual(cut.outlines.get('b'), [[{ x0: 10, x1: 15, half: 2 }]]);
  assert.equal(cut.outlines.has('bar'), false);
});

test('short runs keep a visible gap at either plot edge: whichever neighbour can move does', () => {
  const toEdgeX = (hour: number) => 120 + hour / 16;
  const right = [{ firstHour: 7976, lastHour: 7976, value: 3 }, { firstHour: 7996, lastHour: 7996, value: 3 }];
  const spans = right.map((step, index) => ({ id: `s${index}`, firstHour: step.firstHour, lastHour: step.lastHour, label: 'S', band: [step] }));
  const bands = laneBands(spans, null, toEdgeX, same, 120, 620);
  assert.deepEqual([bands.outlines.get('s0'), bands.outlines.get('s1')], [[[{ x0: 617, x1: 618, half: 3 }]], [[{ x0: 619, x1: 620, half: 3 }]]]);
});

test('a peak never moves across a span boundary', () => {
  const spans = [
    { id: 'A', firstHour: 1, lastHour: 4, label: 'A', band: [{ firstHour: 1, lastHour: 4, value: 9 }] },
    { id: 'B', firstHour: 5, lastHour: 20, label: 'B', band: [{ firstHour: 5, lastHour: 20, value: 1 }] },
  ];
  const bands = laneBands(spans, null, hour => 120 + hour / 10, same, 120, 620);
  assert.deepEqual(bands.outlines.get('A'), [[{ x0: 120, x1: 121, half: 9 }]]);
  assert.deepEqual(bands.outlines.get('B'), [[{ x0: 121, x1: 122, half: 1 }]]);
  const shared = [
    { id: 'A', firstHour: 0, lastHour: 14, label: 'A', band: [{ firstHour: 0, lastHour: 14, value: 1 }] },
    { id: 'B', firstHour: 15, lastHour: 29, label: 'B', band: [{ firstHour: 15, lastHour: 29, value: 9 }] },
  ];
  const split = laneBands(shared, null, hour => hour / 10, same, 0, 100);
  assert.deepEqual(split.outlines.get('A'), [[{ x0: 0, x1: 1, half: 1 }]]);
  assert.deepEqual(split.outlines.get('B'), [[{ x0: 1, x1: 3, half: 9 }]]);
});

test('a short band tries every column its data touches before giving up', () => {
  const spans = [
    { id: 'A', firstHour: 1, lastHour: 1, label: 'A', band: [{ firstHour: 1, lastHour: 1, value: 9 }] },
    { id: 'B', firstHour: 6, lastHour: 10, label: 'B', band: [{ firstHour: 6, lastHour: 10, value: 3 }] },
  ];
  const bands = laneBands(spans, null, hour => 120 + hour / 10, same, 120, 620);
  assert.deepEqual(bands.outlines.get('A'), [[{ x0: 120, x1: 121, half: 9 }]]);
  assert.deepEqual(bands.outlines.get('B'), [[{ x0: 121, x1: 122, half: 3 }]]);
  const displaced = [
    { id: 'B', firstHour: 5, lastHour: 14, label: 'B', band: [{ firstHour: 5, lastHour: 14, value: 3 }] },
    { id: 'C', firstHour: 16, lastHour: 16, label: 'C', band: [{ firstHour: 16, lastHour: 16, value: 9 }] },
  ];
  const moved = laneBands(displaced, null, hour => 120 + hour / 10, same, 120, 620);
  assert.deepEqual([moved.outlines.get('B'), moved.outlines.get('C')], [[[{ x0: 120, x1: 121, half: 3 }]], [[{ x0: 121, x1: 122, half: 9 }]]]);
});

test('hover and Jump to describe the band drawn under the pointer, even one moved off its hours', () => {
  const toEdgeX = (hour: number) => 120 + hour / 16;
  const hourAt = (x: number) => (x - 120) * 16;
  const spans = [
    { id: 'a', firstHour: 8, lastHour: 8, label: 'A', band: [{ firstHour: 8, lastHour: 8, value: 3 }] },
    { id: 'b', firstHour: 28, lastHour: 28, label: 'B', band: [{ firstHour: 28, lastHour: 28, value: 3 }] },
  ];
  const bands = laneBands(spans, null, toEdgeX, same, 120, 620);
  assert.deepEqual(bandHitAt(bands, 122.5, hourAt(122.5)), { spanId: 'b', hour: 28 });
  assert.deepEqual(bandHitAt(bands, 120.5, hourAt(120.5)), { spanId: 'a', hour: 8 });
  assert.equal(bandHitAt(bands, 121.5, hourAt(121.5)), null);
  const wide = laneBands([{ id: 'w', firstHour: 0, lastHour: 99, label: 'W', band: [{ firstHour: 0, lastHour: 99, value: 2 }] }], null, toX, same, 0, 1000);
  assert.deepEqual(bandHitAt(wide, 42.7, 42.7), { spanId: 'w', hour: 42 });
});

test('a fragmented band is laid out in at most a column per pixel', () => {
  const fragmented = (steps: number) => {
    const band = Array.from({ length: steps }, (_, index) => ({ firstHour: index * 2, lastHour: index * 2, value: 1 + (index % 5) }));
    const toX = (hour: number) => 120 + hour / (steps / 250);
    return { band, span: { id: 'f', firstHour: 0, lastHour: steps * 2 - 2, label: 'F', band }, toX, to: steps * 2 };
  };
  const big = fragmented(20_000);
  const bands = laneBands([big.span], null, big.toX, same, 120, 620);
  assert.ok(bands.columns.size > 0 && bands.columns.size <= 500);
});


test('a band step is cut to its span before it becomes a keyboard stop', () => {
  const span = { id: 'cut', firstHour: 10, lastHour: 20, label: 'Cut', band: [{ firstHour: 0, lastHour: 30, value: 5 }] };
  assert.deepEqual(laneStops([span], 0, 100, null).map(stop => [stop.id, stop.hour]), [['cut@10', 10]]);
  assert.deepEqual(laneBands([span], null, toX, same, 0, 1000).outlines.get('cut'), [[{ x0: 10, x1: 21, half: 5 }]]);
});

test('a span whose last hour is the view’s first, partial hour is reachable there', () => {
  const first = { id: 'first', firstHour: 0, lastHour: 23, label: 'First' };
  const second = { id: 'second', firstHour: 24, lastHour: 400, label: 'Second' };
  assert.deepEqual(laneStops([first, second], 23.2, 500, null).map(stop => [stop.id, stop.hour]), [['first@0', 23.2], ['second@24', 24]]);
  const banded = [{ ...first, band: [{ firstHour: 0, lastHour: 23, value: 1 }] }, { ...second, band: [{ firstHour: 24, lastHour: 400, value: 8 }] }];
  assert.deepEqual(laneStops(banded, 23.2, 500, null).map(stop => [stop.id, stop.hour]), [['first@0', 23.2], ['second@24', 24]]);
  assert.deepEqual(laneStops([first, second], 23.2, 500, 23.1), []);
});

test('keyboard stops are found for every step of a long band', () => {
  const long = (steps: number) => {
    const band = Array.from({ length: steps }, (_, index) => ({ firstHour: index * 2, lastHour: index * 2 + 1, value: index % 7 }));
    return { span: { id: 'long', firstHour: 0, lastHour: steps * 2 - 1, label: 'Long', band }, to: steps * 2 };
  };
  const { span, to } = long(20_000);
  const stops = laneStops([span], 0.5, to, null);
  assert.equal(stops.length, 20_000);
  assert.deepEqual(stops.slice(0, 2).map(stop => [stop.id, stop.hour]), [['long@0', 0.5], ['long@2', 2]]);
  assert.equal(stops.at(-1)!.id, 'long@39998');
});

test('a plain bar keeps a pixel column however short it is at this zoom', () => {
  const perHour = 700 / (365 * 24);
  const x0 = 100 + 101 * perHour, x1 = 100 + 125 * perHour;
  assert.ok(x1 - x0 < 2);
  const kept = barExtent(x0, x1, 100, 800)!;
  assert.equal(kept[1] - kept[0], 1, 'one whole pixel column, as a band keeps');
  assert.ok(kept[0] >= Math.floor(x0) && kept[1] <= Math.ceil(x1), 'within its hours');
  assert.deepEqual(barExtent(200, 300, 100, 800), [201, 299]);
  assert.deepEqual(barExtent(50, 150, 100, 800), [100, 149]);
  assert.equal(barExtent(10, 20, 100, 800), null);
  assert.equal(barExtent(900, 910, 100, 800), null);
  assert.equal(barExtent(90, 100, 100, 800), null);
  assert.equal(barExtent(800, 810, 100, 800), null);
  assert.deepEqual(barExtent(799.6, 800.4, 100, 800), [799, 800]);
});

test('a widened bar is hit where it is drawn, and describes an hour it covers', () => {
  const toX = (hour: number) => hour / 1000;
  const hourAt = (px: number) => px * 1000;
  const bar = { id: 'between', firstHour: 100, lastHour: 123, label: 'Made between' };
  assert.deepEqual(barExtent(toX(100), toX(124), 0, 700), [0, 1]);
  const hit = barHitAt([bar], 0.5, null, toX, hourAt, 0, 700)!;
  assert.equal(hit.span.id, 'between');
  assert.ok(hit.hour >= 100 && hit.hour <= 123, `hour ${hit.hour}`);
  assert.equal(barHitAt([bar], 1.5, null, toX, hourAt, 0, 700), null);
  assert.equal(barHitAt([{ ...bar, band: [{ firstHour: 100, lastHour: 123, value: 1 }] }], 0.5, null, toX, hourAt, 0, 700), null);
  const wide = { id: 'wide', firstHour: 0, lastHour: 99, label: 'Wide' };
  assert.deepEqual(barHitAt([wide], 50.5, null, h => h, px => px, 0, 700), { span: wide, hour: 50 });
  assert.equal(barHitAt([wide], 50.5, 40, h => h, px => px, 0, 700), null);
});

test('where widened bars share a column, the one drawn on top answers', () => {
  const toX = (hour: number) => hour / 100;
  const hourAt = (px: number) => px * 100;
  const first = { id: 'first', firstHour: 100, lastHour: 123, label: 'First' };
  const second = { id: 'second', firstHour: 124, lastHour: 147, label: 'Second' };
  assert.deepEqual(barExtent(toX(100), toX(124), 0, 700), [1, 2]);
  assert.deepEqual(barExtent(toX(124), toX(148), 0, 700), [1, 2]);
  const hit = barHitAt([first, second], 1.5, null, toX, hourAt, 0, 700)!;
  assert.equal(hit.span.id, 'second');
  assert.ok(hit.hour >= 124 && hit.hour <= 147, `hour ${hit.hour}`);
});
