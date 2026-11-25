import assert from 'node:assert/strict';
import test from 'node:test';
import { buildStack, stackedSegments, stackEdges, stackExtent } from './stack.ts';
import { clipToSpans, curveSegments, segments } from './series.ts';

const highs = (segment: { bands: { high: number[] }[] }) => segment.bands.map(band => band.high);

test('internal band extrema are opt-in when the outer total stays constant', () => {
  const model = { hours: [0, 1, 2, 3], breakBefore: [false, false, false, false],
    edges: [[20, 30, 50], [80, 10, 10], [5, 25, 70], [20, 30, 50]].map(stackEdges) };
  assert.deepEqual(stackedSegments(model, 0, 3, 10)[0].hours, [0, 3]);
  const [internal] = stackedSegments(model, 0, 3, 10, true);
  assert.deepEqual(internal.hours, [0, 1, 2, 3]);
  assert.deepEqual(internal.bands[0].high, [20, 80, 5, 20]);
  assert.deepEqual(internal.bands[2].low, [50, 90, 30, 50]);
});

test('bands stack in order, an unrecorded band breaks the whole stack, and a new line starts a new segment', () => {
  const first = [{ firstHour: 100, lastHour: 104, value: 2 }, { firstHour: 106, lastHour: 110, value: 3, startsSegment: true }];
  const second = [{ firstHour: 100, lastHour: 110, value: 5 }];
  const model = buildStack([first, second]);
  assert.deepEqual(model.edges[model.hours.indexOf(100)], [{ low: 0, high: 2 }, { low: 2, high: 7 }]);
  assert.equal(model.edges[model.hours.indexOf(105)], null);
  assert.deepEqual(stackedSegments(model, 100, 110, 0).map(segment => [segment.hours[0], segment.hours.at(-1)]), [[100, 104], [106, 110]]);
  assert.deepEqual(stackExtent(model, 100, 110), [0, 8]);
  assert.deepEqual(stackExtent(model, 200, 300), null);
  assert.deepEqual(stackExtent(model, 102, 103), [0, 7]);
});

test('a view or playhead ending inside a run still draws it, from the view’s start to its end', () => {
  const model = buildStack([[{ firstHour: 100, lastHour: 200, value: 5 }], [{ firstHour: 100, lastHour: 200, value: 1 }]]);
  assert.deepEqual(stackedSegments(model, 120, 180, 0).map(segment => [segment.hours, highs(segment)]), [[[120, 180], [[5, 5], [6, 6]]]]);
  assert.deepEqual(stackedSegments(model, 100, 150, 0).map(segment => segment.hours), [[100, 150]]);
  const changing = buildStack([[{ firstHour: 100, lastHour: 110, value: 1 }, { firstHour: 111, lastHour: 200, value: 2 }]]);
  assert.deepEqual(stackedSegments(changing, 100, 150, 30).map(segment => segment.hours), [[100, 111, 150]]);
  assert.deepEqual(stackedSegments(model, 150, 400, 0).map(segment => segment.hours), [[150, 200]]);
  const lonely = buildStack([[{ firstHour: 100, lastHour: 100, value: 1 }]]);
  assert.deepEqual(stackedSegments(lonely, 90, 120, 0).map(segment => segment.hours), [[100, 101]]);
});

test('a playhead before the view’s start draws nothing, in stacks and lines alike', () => {
  const model = buildStack([[{ firstHour: 100, lastHour: 1000, value: 5 }]]);
  assert.deepEqual(stackedSegments(model, 500, 200, 0), []);
  assert.deepEqual(stackedSegments(model, 500, Number.NaN, 0), []);
  assert.deepEqual(stackedSegments(model, 500, 500, 0).map(segment => segment.hours), [[500, 501]]);
  const runs = [{ firstHour: 100, lastHour: 1000, value: 5 }];
  assert.deepEqual(segments(runs, 500, 200), []);
  assert.deepEqual(curveSegments(runs, 500, 200, hour => hour, value => value), []);
  assert.deepEqual(segments(runs, 500, 600), [[{ firstHour: 500, lastHour: 600, value: 5 }]]);
});

test('a zoom starting inside the last recorded hour before a gap never paints into the gap', () => {
  const runs = [{ firstHour: 100, lastHour: 104, value: 2 }, { firstHour: 106, lastHour: 110, value: 3 }];
  const model = buildStack([runs]);
  assert.deepEqual(stackedSegments(model, 104.5, 110, 0).map(segment => segment.hours), [[104.5, 105], [106, 110]]);
  assert.deepEqual(stackedSegments(model, 104.2, 104.8, 0).map(segment => segment.hours), [[104.2, 104.8]]);
  assert.deepEqual(stackedSegments(model, 105, 110, 0).map(segment => segment.hours), [[106, 110]]);
  const lonely = buildStack([[{ firstHour: 100, lastHour: 100, value: 1 }, { firstHour: 102, lastHour: 102, value: 1 }]]);
  assert.deepEqual(stackedSegments(lonely, 100.5, 110, 0).map(segment => segment.hours), [[100.5, 101], [102, 103]]);
  assert.deepEqual(segments(runs, 104.5, 110).map(segment => segment.map(run => [run.firstHour, run.lastHour])), [[[104, 104]], [[106, 110]]]);
  assert.deepEqual(segments(runs, 99.5, 105.5).map(segment => segment.map(run => [run.firstHour, run.lastHour])), [[[100, 104]]]);
});

test('runs are cut to spans in one pass: pieces in order, overlapping spans merged, breaks kept only where a run starts', () => {
  const runs = [
    { firstHour: 0, lastHour: 9, value: 1, breakBefore: true },
    { firstHour: 10, lastHour: 19, value: 2 },
    { firstHour: 25, lastHour: 29, value: 3, breakBefore: true },
  ];
  assert.deepEqual(clipToSpans(runs, [{ firstHour: 5, lastHour: 12 }, { firstHour: 8, lastHour: 11 }, { firstHour: 15, lastHour: 27 }]), [
    { firstHour: 5, lastHour: 9, value: 1 },
    { firstHour: 10, lastHour: 12, value: 2 },
    { firstHour: 15, lastHour: 19, value: 2 },
    { firstHour: 25, lastHour: 27, value: 3, breakBefore: true },
  ]);
  assert.deepEqual(clipToSpans(runs, [{ firstHour: 12, lastHour: 14 }, { firstHour: 0, lastHour: 11 }]).map(run => [run.firstHour, run.lastHour]),
    [[0, 9], [10, 11], [12, 14]]);
  assert.deepEqual(clipToSpans(runs, []), []);
  assert.deepEqual(clipToSpans([], [{ firstHour: 0, lastHour: 5 }]), []);
});


test('negative values stack below zero, never replaced by zero', () => {
  assert.deepEqual(stackEdges([3, -2, 4, -1, 0]), [
    { low: 0, high: 3 }, { low: -2, high: 0 }, { low: 3, high: 7 }, { low: -3, high: -2 }, { low: 7, high: 7 },
  ]);
  const model = buildStack([[{ firstHour: 0, lastHour: 9, value: 4 }], [{ firstHour: 0, lastHour: 9, value: -3 }]]);
  const [segment] = stackedSegments(model, 0, 9, 0);
  assert.deepEqual(segment.bands[1], { low: [-3, -3], high: [0, 0] });
  assert.deepEqual(stackExtent(model, 0, 9), [-3, 4]);
});

test('thinning keeps a peak or dip narrower than the samples merged, and nothing that stays inside its neighbours', () => {
  const up = [{ firstHour: 100, lastHour: 123, value: 0 }, { firstHour: 124, lastHour: 147, value: 100 }, { firstHour: 148, lastHour: 171, value: 0 }];
  const down = [{ firstHour: 100, lastHour: 123, value: 0 }, { firstHour: 124, lastHour: 147, value: -40 }, { firstHour: 148, lastHour: 171, value: 0 }];
  const [spike] = stackedSegments(buildStack([up, down]), 100, 171, 50);
  assert.deepEqual(spike.hours, [100, 124, 171]);
  assert.deepEqual(highs(spike)[0], [0, 100, 0]);
  assert.deepEqual(spike.bands[1].low, [0, -40, 0]);
  const flat = [{ firstHour: 100, lastHour: 171, value: 5 }];
  assert.deepEqual(stackedSegments(buildStack([flat]), 100, 171, 50)[0].hours, [100, 171]);
});
