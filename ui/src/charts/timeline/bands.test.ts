import assert from 'node:assert/strict';
import test from 'node:test';
import { assertLinear } from '../linearTime.ts';
import { bandHalf, bandHitAt, bandOutlines, laneBandLayouts, laneBands, laneStops } from './layout.ts';

const toX = (hour: number) => hour;
const same = (value: number) => value;

test('a band has one flat stretch per step, cut to the hours shown', () => {
  const steps = [{ firstHour: 0, lastHour: 9, value: 2 }, { firstHour: 10, lastHour: 29, value: 5 }];
  assert.deepEqual(bandOutlines(steps, 0, 30, toX, same, 0, 1000), [[{ x0: 0, x1: 10, half: 2 }, { x0: 10, x1: 30, half: 5 }]]);
  // A playhead at hour 15 stops the band there; a band starting at 5 starts there.
  assert.deepEqual(bandOutlines(steps, 5, 15, toX, same, 0, 1000), [[{ x0: 5, x1: 10, half: 2 }, { x0: 10, x1: 15, half: 5 }]]);
  assert.deepEqual(bandOutlines(steps, 40, 50, toX, same, 0, 1000), []);
});

test('a missing hour between steps breaks the band', () => {
  const steps = [{ firstHour: 0, lastHour: 9, value: 2 }, { firstHour: 12, lastHour: 20, value: 2 }];
  assert.deepEqual(bandOutlines(steps, 0, 30, toX, same, 0, 1000), [[{ x0: 0, x1: 10, half: 2 }], [{ x0: 12, x1: 21, half: 2 }]]);
});

test('each column is drawn at the thickest step touching it, so no peak is lost', () => {
  // A day is a tenth of a pixel: a one-day spike still shows.
  const steps = [{ firstHour: 0, lastHour: 23, value: 1 }, { firstHour: 24, lastHour: 47, value: 9 }, { firstHour: 48, lastHour: 71, value: 1 },
    { firstHour: 72, lastHour: 2399, value: 3 }];
  assert.deepEqual(bandOutlines(steps, 0, 2400, hour => hour / 240, same, 0, 1000), [[{ x0: 0, x1: 1, half: 9 }, { x0: 1, x1: 10, half: 3 }]]);
});

test('two runs more than a pixel apart never touch, even when each is narrower than a pixel', () => {
  // Finding: steps only at hours 8 and 28, 19 missing hours (1.19 px) between them.
  const steps = [{ firstHour: 8, lastHour: 8, value: 3 }, { firstHour: 28, lastHour: 28, value: 3 }];
  const outline = bandOutlines(steps, 0, 48, hour => 120 + hour / 16, same, 120, 620);
  assert.deepEqual(outline, [[{ x0: 120, x1: 121, half: 3 }], [{ x0: 122, x1: 123, half: 3 }]]);
  // Less than a pixel apart, the two columns may touch: the data is closer than a pixel.
  const close = [{ firstHour: 8, lastHour: 8, value: 3 }, { firstHour: 18, lastHour: 18, value: 3 }];
  assert.deepEqual(bandOutlines(close, 0, 48, hour => 120 + hour / 16, same, 120, 620).map(run => run[0].x0), [120, 121]);
});

test('a column a run only partly covers passes its thickest to the column drawn', () => {
  // Finding: steps [1, 4] = 9 and [5, 20] = 1 at a tenth of a pixel an hour: only column
  // 121 is fully covered, and it keeps the 9 of the partly covered column before it.
  const steps = [{ firstHour: 1, lastHour: 4, value: 9 }, { firstHour: 5, lastHour: 20, value: 1 }];
  assert.deepEqual(bandOutlines(steps, 0, 30, hour => 120 + hour / 10, same, 120, 620), [[{ x0: 121, x1: 122, half: 9 }]]);
});

test('a band value maps from a least thickness up to the most at the maximum', () => {
  assert.equal(bandHalf(0, 20, 1, 12), 1);
  assert.equal(bandHalf(20, 20, 1, 12), 12);
  assert.equal(bandHalf(10, 20, 1, 12), 6.5);
  assert.equal(bandHalf(40, 20, 1, 12), 12);
  // No scale: every band is drawn thin.
  assert.equal(bandHalf(5, 0, 1, 12), 1);
});

test('a short band stays drawn at least a pixel wide however far the view is zoomed out', () => {
  // A year across 500 pixels after a 120-pixel column: a day is 1.37 pixels, an hour 0.06.
  const year = 365 * 24;
  const toYearX = (hour: number) => 120 + hour / year * 500;
  const day = bandOutlines([{ firstHour: 0, lastHour: 23, value: 3 }], 0, year, toYearX, value => value, 120, 620);
  assert.equal(day.length, 1);
  assert.ok(day[0][0].x1 - day[0][0].x0 >= 1, 'a day is drawn');
  assert.equal(day[0][0].half, 3);
  const hour = bandOutlines([{ firstHour: 100, lastHour: 100, value: 2 }], 0, year, toYearX, value => value, 120, 620);
  assert.equal(hour.length, 1);
  assert.deepEqual(hour, [[{ x0: 125, x1: 126, half: 2 }]], 'an hour fills the pixel column it falls in');
  // At the plot's edge the pixel stays inside the plot.
  const edge = bandOutlines([{ firstHour: 0, lastHour: 0, value: 1 }], 0, year, toYearX, value => value, 120, 620);
  assert.equal(edge[0][0].x0, 120);
});

test('a band is cut to the plot and one wholly outside it is not drawn', () => {
  const steps = [{ firstHour: 0, lastHour: 99, value: 4 }];
  assert.deepEqual(bandOutlines(steps, 0, 100, hour => hour, value => value, 20, 60), [[{ x0: 20, x1: 60, half: 4 }]]);
  assert.deepEqual(bandOutlines(steps, 0, 100, hour => hour + 500, value => value, 20, 60), []);
});

test('a sub-pixel peak between wide steps keeps its full thickness in its pixel column', () => {
  // The reviewer's case: 1 over hours 0–2399 (120–130 px), 20 over 2400–2423 (130–130.1 px),
  // 1 over 2424–4799 (130.1–140 px). Every column is the thickest step touching it.
  const steps = [{ firstHour: 0, lastHour: 2399, value: 1 }, { firstHour: 2400, lastHour: 2423, value: 20 }, { firstHour: 2424, lastHour: 4799, value: 1 }];
  const outline = bandOutlines(steps, 0, 4800, hour => 120 + hour / 240, value => value, 120, 640);
  assert.deepEqual(outline, [[{ x0: 120, x1: 130, half: 1 }, { x0: 130, x1: 131, half: 20 }, { x0: 131, x1: 140, half: 1 }]]);
});

test('widening never moves a band into hours it does not cover', () => {
  // A day wholly left of the plot (119.6–119.7 px, the plot starting at 120) draws nothing.
  const day = [{ firstHour: 0, lastHour: 23, value: 5 }];
  assert.deepEqual(bandOutlines(day, 0, 24, hour => 119.6 + hour / 240, value => value, 120, 620), []);
  // Nor one wholly right of it.
  assert.deepEqual(bandOutlines(day, 0, 24, hour => 620 + hour / 240, value => value, 120, 620), []);
  // One whose hours reach into the view shows there.
  assert.deepEqual(bandOutlines(day, 0, 24, hour => 119.95 + hour / 240, value => value, 120, 620), [[{ x0: 120, x1: 121, half: 5 }]]);
});

test('rounding never fills over a gap in the recording', () => {
  // The reviewer's case: equal steps over hours 0–201 and 226–447 at 0.075 px an hour. The
  // 24 missing hours lie at 135.15–136.95 px; both columns they touch stay empty.
  const steps = [{ firstHour: 0, lastHour: 201, value: 3 }, { firstHour: 226, lastHour: 447, value: 3 }];
  const outline = bandOutlines(steps, 0, 448, hour => 120 + hour * 0.075, value => value, 120, 640);
  assert.equal(outline.length, 2);
  assert.equal(outline[0].at(-1)!.x1, 135);
  assert.equal(outline[1][0].x0, 137);
  // A gap narrower than a pixel still leaves its pixel empty between two wider runs.
  const narrow = [{ firstHour: 0, lastHour: 201, value: 3 }, { firstHour: 204, lastHour: 447, value: 3 }];
  const split = bandOutlines(narrow, 0, 448, hour => 120 + hour * 0.075, value => value, 120, 640);
  assert.equal(split.length, 2);
  assert.ok(split[1][0].x0 - split[0].at(-1)!.x1 >= 1, 'the gap shows');
  // A thicker step in a trimmed edge column passes its thickness to the run's next column.
  const edge = [{ firstHour: 0, lastHour: 199, value: 1 }, { firstHour: 200, lastHour: 201, value: 9 }];
  assert.equal(bandOutlines(edge, 0, 202, hour => 120 + hour * 0.075, value => value, 120, 640)[0].at(-1)!.half, 9);
});

test('the keyboard stops at every band step in view, so each value a pointer reaches is reachable', () => {
  // The reviewer's case: a span of hours 0–47 whose band is 1 for 0–23 and 8 for 24–47.
  const span = { id: 'line', firstHour: 0, lastHour: 47, label: 'Line', band: [{ firstHour: 0, lastHour: 23, value: 1 }, { firstHour: 24, lastHour: 47, value: 8 }] };
  assert.deepEqual(laneStops([span], 0, 100, null).map(stop => [stop.id, stop.hour]), [['line@0', 0], ['line@24', 24]]);
  // A view starting inside the first step reaches the value carried into it (at the view's
  // start, described at its whole hour as hover floors it), then the next step.
  assert.deepEqual(laneStops([span], 10.5, 100, null).map(stop => [stop.id, stop.hour]), [['line@0', 10.5], ['line@24', 24]]);
  // Finding: a view starting at 23.2 still reaches hour 23's value, 1, before the step at 24.
  assert.deepEqual(laneStops([span], 23.2, 100, null).map(stop => [stop.id, stop.hour]), [['line@0', 23.2], ['line@24', 24]]);
  // Not past the playhead.
  assert.deepEqual(laneStops([span], 0, 100, 20).map(stop => stop.hour), [0]);
  // A bar without a band keeps one stop, at its first hour shown.
  const bar = { id: 'bar', firstHour: 50, lastHour: 60, label: 'Bar' };
  assert.deepEqual(laneStops([span, bar], 0, 100, null).map(stop => stop.id), ['line@0', 'line@24', 'bar@50']);
  assert.deepEqual(laneStops([bar], 55, 100, null).map(stop => [stop.id, stop.hour]), [['bar@50', 55]]);
  // An hour between a band's steps that no step covers is not a stop.
  const holed = { ...span, band: [{ firstHour: 0, lastHour: 9, value: 1 }, { firstHour: 30, lastHour: 47, value: 8 }] };
  assert.deepEqual(laneStops([holed], 20, 100, null).map(stop => stop.hour), [30]);
});

test('a lane places its band spans together, so a gap between two spans stays visible', () => {
  // Finding: two separate equal-valued band spans covering only hours 8 and 28 (1.19 px apart).
  const spans = [
    { id: 'a', firstHour: 8, lastHour: 8, label: 'A', band: [{ firstHour: 8, lastHour: 8, value: 3 }] },
    { id: 'b', firstHour: 28, lastHour: 28, label: 'B', band: [{ firstHour: 28, lastHour: 28, value: 3 }] },
  ];
  const bands = laneBands(spans, null, hour => 120 + hour / 16, same, 120, 620);
  assert.deepEqual(bands.outlines.get('a'), [[{ x0: 120, x1: 121, half: 3 }]]);
  assert.deepEqual(bands.outlines.get('b'), [[{ x0: 122, x1: 123, half: 3 }]]);
  // Spans that touch (a variant switch) make one run with no gap, each span keeping its columns.
  const touching = [
    { id: 'a', firstHour: 0, lastHour: 9, label: 'A', band: [{ firstHour: 0, lastHour: 9, value: 2 }] },
    { id: 'b', firstHour: 10, lastHour: 19, label: 'B', band: [{ firstHour: 10, lastHour: 19, value: 2 }] },
  ];
  const joined = laneBands(touching, null, toX, same, 0, 1000);
  assert.deepEqual(joined.outlines.get('a'), [[{ x0: 0, x1: 10, half: 2 }]]);
  assert.deepEqual(joined.outlines.get('b'), [[{ x0: 10, x1: 20, half: 2 }]]);
  // A span is cut at the playhead, and bars without a band are not bands.
  const cut = laneBands([...touching, { id: 'bar', firstHour: 30, lastHour: 40, label: 'Bar' }], 15, toX, same, 0, 1000);
  assert.deepEqual(cut.outlines.get('b'), [[{ x0: 10, x1: 15, half: 2 }]]);
  assert.equal(cut.outlines.has('bar'), false);
});

test('short runs keep a visible gap at either plot edge: whichever neighbour can move does', () => {
  // Finding: at the right edge, one-hour steps at 7976 and 7996 (618.5 and 619.75 px, a 1.19 px
  // gap); the second cannot move right, so the first moves left.
  const toEdgeX = (hour: number) => 120 + hour / 16;
  const right = [{ firstHour: 7976, lastHour: 7976, value: 3 }, { firstHour: 7996, lastHour: 7996, value: 3 }];
  assert.deepEqual(bandOutlines(right, 0, 8000, toEdgeX, same, 120, 620), [[{ x0: 617, x1: 618, half: 3 }], [{ x0: 619, x1: 620, half: 3 }]]);
  // The left-edge mirror: the first cannot move left, so the second moves right.
  const leftEdge = [{ firstHour: 4, lastHour: 4, value: 3 }, { firstHour: 24, lastHour: 24, value: 3 }];
  assert.deepEqual(bandOutlines(leftEdge, 0, 8000, toEdgeX, same, 120, 620), [[{ x0: 120, x1: 121, half: 3 }], [{ x0: 122, x1: 123, half: 3 }]]);
  // The same across two spans of a lane.
  const spans = right.map((step, index) => ({ id: `s${index}`, firstHour: step.firstHour, lastHour: step.lastHour, label: 'S', band: [step] }));
  const bands = laneBands(spans, null, toEdgeX, same, 120, 620);
  assert.deepEqual([bands.outlines.get('s0'), bands.outlines.get('s1')], [[[{ x0: 617, x1: 618, half: 3 }]], [[{ x0: 619, x1: 620, half: 3 }]]]);
});

test('a peak never moves across a span boundary', () => {
  // Finding: span A (hours 1–4, value 9) and span B (5–20, value 1) at a tenth of a pixel an
  // hour. A's only column is partly a gap, so A keeps it (its middle) at 9; B keeps its own.
  const spans = [
    { id: 'A', firstHour: 1, lastHour: 4, label: 'A', band: [{ firstHour: 1, lastHour: 4, value: 9 }] },
    { id: 'B', firstHour: 5, lastHour: 20, label: 'B', band: [{ firstHour: 5, lastHour: 20, value: 1 }] },
  ];
  const bands = laneBands(spans, null, hour => 120 + hour / 10, same, 120, 620);
  assert.deepEqual(bands.outlines.get('A'), [[{ x0: 120, x1: 121, half: 9 }]]);
  assert.deepEqual(bands.outlines.get('B'), [[{ x0: 121, x1: 122, half: 1 }]]);
  // A fully recorded column two spans share is drawn by the thicker, in its colour.
  const shared = [
    { id: 'A', firstHour: 0, lastHour: 14, label: 'A', band: [{ firstHour: 0, lastHour: 14, value: 1 }] },
    { id: 'B', firstHour: 15, lastHour: 29, label: 'B', band: [{ firstHour: 15, lastHour: 29, value: 9 }] },
  ];
  const split = laneBands(shared, null, hour => hour / 10, same, 0, 100);
  assert.deepEqual(split.outlines.get('A'), [[{ x0: 0, x1: 1, half: 1 }]]);
  assert.deepEqual(split.outlines.get('B'), [[{ x0: 1, x1: 3, half: 9 }]]);
});

test('a short band tries every column its data touches before giving up', () => {
  // Finding: span A at hour 1 (value 9) and span B over hours 6–10 (value 3) at a tenth of a
  // pixel an hour: B's middle column 120 holds A's thicker mark, so B takes its free column 121.
  const spans = [
    { id: 'A', firstHour: 1, lastHour: 1, label: 'A', band: [{ firstHour: 1, lastHour: 1, value: 9 }] },
    { id: 'B', firstHour: 6, lastHour: 10, label: 'B', band: [{ firstHour: 6, lastHour: 10, value: 3 }] },
  ];
  const bands = laneBands(spans, null, hour => 120 + hour / 10, same, 120, 620);
  assert.deepEqual(bands.outlines.get('A'), [[{ x0: 120, x1: 121, half: 9 }]]);
  assert.deepEqual(bands.outlines.get('B'), [[{ x0: 121, x1: 122, half: 3 }]]);
  // A thicker mark displaces a thinner one, which then takes another of its own columns:
  // B (hours 5–14, 120.5–121.5 px) first takes its middle, 121, the only column C (hour 16)
  // touches; C takes it, and B moves to its other column, 120.
  const displaced = [
    { id: 'B', firstHour: 5, lastHour: 14, label: 'B', band: [{ firstHour: 5, lastHour: 14, value: 3 }] },
    { id: 'C', firstHour: 16, lastHour: 16, label: 'C', band: [{ firstHour: 16, lastHour: 16, value: 9 }] },
  ];
  const moved = laneBands(displaced, null, hour => 120 + hour / 10, same, 120, 620);
  assert.deepEqual([moved.outlines.get('B'), moved.outlines.get('C')], [[[{ x0: 120, x1: 121, half: 3 }]], [[{ x0: 121, x1: 122, half: 9 }]]]);
});

test('hover and Jump to describe the band drawn under the pointer, even one moved off its hours', () => {
  // Finding: plot 120–620, band spans at hours 8 and 28 drawn at columns 120 and 122; column 122
  // lies over hours 32–47, where no span is, yet it draws the span at hour 28.
  const toEdgeX = (hour: number) => 120 + hour / 16;
  const hourAt = (x: number) => (x - 120) * 16;
  const spans = [
    { id: 'a', firstHour: 8, lastHour: 8, label: 'A', band: [{ firstHour: 8, lastHour: 8, value: 3 }] },
    { id: 'b', firstHour: 28, lastHour: 28, label: 'B', band: [{ firstHour: 28, lastHour: 28, value: 3 }] },
  ];
  const bands = laneBands(spans, null, toEdgeX, same, 120, 620);
  assert.deepEqual(bandHitAt(bands, 122.5, hourAt(122.5)), { spanId: 'b', hour: 28 });
  assert.deepEqual(bandHitAt(bands, 120.5, hourAt(120.5)), { spanId: 'a', hour: 8 });
  // Over the empty column between them nothing is drawn, so nothing is described from the bands.
  assert.equal(bandHitAt(bands, 121.5, hourAt(121.5)), null);
  // Over a wide band, the pointer's own hour within the step.
  const wide = laneBands([{ id: 'w', firstHour: 0, lastHour: 99, label: 'W', band: [{ firstHour: 0, lastHour: 99, value: 2 }] }], null, toX, same, 0, 1000);
  assert.deepEqual(bandHitAt(wide, 42.7, 42.7), { spanId: 'w', hour: 42 });
});

test('a fragmented band is laid out in time linear in its steps', () => {
  // Finding: 20,000 one-hour steps separated by missing hours across a 500 px plot, so the
  // runs, narrower than a pixel, compete for 500 columns; a quadratic pass took 41-82 ms.
  const fragmented = (steps: number) => {
    const band = Array.from({ length: steps }, (_, index) => ({ firstHour: index * 2, lastHour: index * 2, value: 1 + (index % 5) }));
    const toX = (hour: number) => 120 + hour / (steps / 250);
    return { band, span: { id: 'f', firstHour: 0, lastHour: steps * 2 - 2, label: 'F', band }, toX, to: steps * 2 };
  };
  const big = fragmented(20_000);
  const bands = laneBands([big.span], null, big.toX, same, 120, 620);
  assert.ok(bands.columns.size > 0 && bands.columns.size <= 500);
  assert.ok(bandOutlines(big.band, 0, big.to, big.toX, same, 120, 620).length > 0);
  // Linear, not quadratic.
  for (const layout of [
    (f: ReturnType<typeof fragmented>) => laneBands([f.span], null, f.toX, same, 120, 620),
    (f: ReturnType<typeof fragmented>) => bandOutlines(f.band, 0, f.to, f.toX, same, 120, 620),
  ]) assertLinear('steps', steps => { const f = fragmented(steps); return () => layout(f); });
});

test('lane layouts are rebuilt from the lanes shown and reused while nothing changes', () => {
  const lane = (id: string) => ({ id, label: id, spans: [{ id: `${id}-span`, firstHour: 0, lastHour: 9, label: id, band: [{ firstHour: 0, lastHour: 9, value: 2 }] }] });
  const [a, b] = [lane('a'), lane('b')];
  const bar = { id: 'bar', label: 'Bar', spans: [{ id: 'x', firstHour: 0, lastHour: 9, label: 'X' }] };
  let computed = 0;
  const layout = (item: { spans: readonly { id: string; firstHour: number; lastHour: number; label: string }[] }) => {
    computed++;
    return laneBands(item.spans, null, toX, same, 0, 1000);
  };
  const first = laneBandLayouts([a, b, bar], 'view', new Map(), layout);
  assert.deepEqual([...first.keys()], ['a', 'b'], 'a lane without bands has no layout');
  assert.equal(computed, 2);
  // Same lanes and view (the pointer moved): nothing is laid out again.
  const again = laneBandLayouts([a, b, bar], 'view', first, layout);
  assert.equal(computed, 2);
  assert.equal(again.get('a'), first.get('a'));
  // A lane gone is dropped; a changed view lays the rest out again.
  const pruned = laneBandLayouts([b], 'view', again, layout);
  assert.deepEqual([...pruned.keys()], ['b']);
  assert.equal(computed, 2);
  laneBandLayouts([b], 'zoomed', pruned, layout);
  assert.equal(computed, 3);
});

test('a band step is cut to its span before it becomes a keyboard stop', () => {
  // Finding: a span of hours 10–20 whose band step covers 0–30 is drawn from 10; its stop is 10.
  const span = { id: 'cut', firstHour: 10, lastHour: 20, label: 'Cut', band: [{ firstHour: 0, lastHour: 30, value: 5 }] };
  assert.deepEqual(laneStops([span], 0, 100, null).map(stop => [stop.id, stop.hour]), [['cut@10', 10]]);
  assert.deepEqual(laneBands([span], null, toX, same, 0, 1000).outlines.get('cut'), [[{ x0: 10, x1: 21, half: 5 }]]);
});

test('a span whose last hour is the view’s first, partial hour is reachable there', () => {
  // Finding: spans 0–23 and 24–400, a view starting at 23.2 with no playhead. Hover at the
  // view's start shows hour 23, the first span's; the keyboard reaches it too.
  const first = { id: 'first', firstHour: 0, lastHour: 23, label: 'First' };
  const second = { id: 'second', firstHour: 24, lastHour: 400, label: 'Second' };
  assert.deepEqual(laneStops([first, second], 23.2, 500, null).map(stop => [stop.id, stop.hour]), [['first@0', 23.2], ['second@24', 24]]);
  // The same with bands.
  const banded = [{ ...first, band: [{ firstHour: 0, lastHour: 23, value: 1 }] }, { ...second, band: [{ firstHour: 24, lastHour: 400, value: 8 }] }];
  assert.deepEqual(laneStops(banded, 23.2, 500, null).map(stop => [stop.id, stop.hour]), [['first@0', 23.2], ['second@24', 24]]);
  // Not when the playhead is before the view.
  assert.deepEqual(laneStops([first, second], 23.2, 500, 23.1), []);
});

test('keyboard stops are found in one pass over a long band', () => {
  // Finding: 20,000 steps; a scan per stop took about 200 million comparisons.
  const long = (steps: number) => {
    const band = Array.from({ length: steps }, (_, index) => ({ firstHour: index * 2, lastHour: index * 2 + 1, value: index % 7 }));
    return { span: { id: 'long', firstHour: 0, lastHour: steps * 2 - 1, label: 'Long', band }, to: steps * 2 };
  };
  const { span, to } = long(20_000);
  const stops = laneStops([span], 0.5, to, null);
  assert.equal(stops.length, 20_000);
  assert.deepEqual(stops.slice(0, 2).map(stop => [stop.id, stop.hour]), [['long@0', 0.5], ['long@2', 2]]);
  assert.equal(stops.at(-1)!.id, 'long@39998');
  assertLinear('steps', steps => { const l = long(steps); return () => laneStops([l.span], 0.5, l.to, null); });
});
