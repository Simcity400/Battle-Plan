import test from 'node:test';
import assert from 'node:assert/strict';
import { AXIS_HEIGHT, ROW_HEIGHT, lanesInView } from './layout.ts';
import { bandMaxOf, bridgeGaps, coverCounts, coveredBy, describeRun, runState, stretchAt, stretchesOf, withoutHours } from './runState.ts';

test('the band maximum takes any number of steps without a spread', () => {
  // 1,000 overlords of 140 single-hour runs: 140,000 steps, past what a spread into Math.max takes.
  const bands = Array.from({ length: 1000 }, (_, band) => Array.from({ length: 140 }, (_, step) => ({ firstHour: step * 2, lastHour: step * 2, value: (band * 7 + step) % 97 })));
  assert.equal(bandMaxOf(bands), 96);
  assert.equal(bandMaxOf([]), 1);
});

test('runs joined across unrecorded hours are one stretch; a recorded break is not', () => {
  const runs = [{ firstHour: 100, lastHour: 199 }, { firstHour: 210, lastHour: 300 }, { firstHour: 320, lastHour: 400 }];
  // 200..209 were not recorded; 301..319 were, and show the holding gone.
  const stretches = bridgeGaps(runs, [{ firstHour: 200, lastHour: 209 }]);
  assert.deepEqual(stretches.map(item => [item.firstHour, item.lastHour, item.runs.length]), [[100, 300, 2], [320, 400, 1]]);
  // So the first run is not said to have ended: at 300 its stretch is still held.
  assert.deepEqual(runState([stretchAt(stretches, 150)!], 300), { kind: 'held', since: 100 });
  assert.deepEqual(runState([stretchAt(stretches, 310)!], 310), { kind: 'ended', at: 301 });
  // A caller may refuse a join (another relation), and adjacent runs join with no gap at all.
  assert.equal(bridgeGaps(runs, [{ firstHour: 200, lastHour: 209 }], () => false).length, 3);
  assert.equal(bridgeGaps([{ firstHour: 0, lastHour: 9 }, { firstHour: 10, lastHour: 19 }]).length, 1);
  assert.equal(coveredBy(5, 4, []), true);
  assert.equal(coveredBy(200, 209, [{ firstHour: 195, lastHour: 204 }, { firstHour: 205, lastHour: 209 }]), true);
  assert.equal(coveredBy(200, 209, [{ firstHour: 195, lastHour: 204 }]), false);
});

test('fragmented, touching and overlapping unobserved spans bridge only fully covered gaps', () => {
  const runs = [0, 5, 10, 15, 20].map(hour => ({ firstHour: hour, lastHour: hour }));
  const spans = [[1, 2], [2, 3], [4, 4], [6, 7], [8, 9], [11, 12], [13, 13], [16, 30], [17, 18]]
    .map(([firstHour, lastHour]) => ({ firstHour, lastHour }));
  assert.deepEqual(bridgeGaps(runs, spans).map(stretch => [stretch.firstHour, stretch.lastHour, stretch.runs.length]), [[0, 10, 3], [15, 20, 2]]);
  assert.deepEqual(bridgeGaps(runs, [...spans].reverse()), bridgeGaps(runs, spans));
  // One long span covers several gaps without being lost when the cursor moves.
  assert.equal(bridgeGaps(runs, [{ firstHour: 1, lastHour: 30 }]).length, 1);
});

test('alternating runs and gaps visit spans a linear number of times', () => {
  for (const count of [5000, 10000]) {
    let visits = 0;
    const runs = Array.from({ length: count + 1 }, (_, index) => ({ firstHour: index * 2, lastHour: index * 2 }));
    const spans = Array.from({ length: count }, (_, index) => ({
      get firstHour() { visits++; return index * 2 + 1; },
      get lastHour() { visits++; return index * 2 + 1; },
    }));
    const stretches = bridgeGaps(runs, spans);
    assert.equal(stretches.length, 1);
    assert.equal(stretches[0].runs.length, count + 1);
    // Count the work instead of timing it, so machine load cannot hide a rescan.
    assert.ok(visits < count * 12, `${count} gaps took ${visits} span reads`);
  }
});

const words = { since: (h: number) => `since ${h}`, before: (h: number) => `before ${h}`, ended: (h: number) => `ended ${h}`,
  endedBy: (h: number) => `ended by ${h}`, lastSeen: (h: number) => `last ${h}` };

test('a run is held, ended, ended by a day or unknown, honestly about unrecorded hours', () => {
  const runs = [{ firstHour: 10, lastHour: 19 }, { firstHour: 40, lastHour: 49 }];
  assert.deepEqual(runState(runs, 5), { kind: 'none' });
  assert.deepEqual(runState(runs, 15), { kind: 'held', since: 10 });
  assert.deepEqual(runState(runs, 25), { kind: 'ended', at: 20 });
  // Hours 50..59 were not recorded: after the last run it is unknown, then ended by 60.
  const gap = [{ firstHour: 50, lastHour: 59 }];
  assert.deepEqual(runState(runs, 55, gap), { kind: 'unknown', lastSeen: 49 });
  assert.deepEqual(runState(runs, 65, gap), { kind: 'ended', at: 60, lastSeen: 49 });
  assert.equal(describeRun(runs, 5, words), null);
  assert.equal(describeRun(runs, 15, words, [], 0), 'since 10');
  // Held from the branch's first hour, or right after an unrecorded stretch: before, not since.
  assert.equal(describeRun(runs, 15, words, [], 10), 'before 10');
  assert.equal(describeRun(runs, 45, words, [{ firstHour: 30, lastHour: 39 }], 0), 'before 40');
  assert.equal(describeRun(runs, 65, words, gap), 'ended by 60');
  assert.equal(describeRun(runs, 55, words, gap), 'last 49');
});

test('cover counts step with the runs covering each hour, in any order and overlapping', () => {
  const runs = [{ firstHour: 5, lastHour: 9 }, { firstHour: 0, lastHour: 9 }, { firstHour: 12, lastHour: 14 }, { firstHour: 7, lastHour: 7 }];
  assert.deepEqual(coverCounts(runs), [
    { firstHour: 0, lastHour: 4, value: 1 }, { firstHour: 5, lastHour: 6, value: 2 }, { firstHour: 7, lastHour: 7, value: 3 },
    { firstHour: 8, lastHour: 9, value: 2 }, { firstHour: 12, lastHour: 14, value: 1 },
  ]);
  assert.deepEqual(coverCounts([]), []);
});

test('steps lose the hours spans cover, and split into unbroken stretches', () => {
  const steps = [{ firstHour: 0, lastHour: 9, value: 1 }, { firstHour: 10, lastHour: 19, value: 2 }];
  const cut = withoutHours(steps, [{ firstHour: 3, lastHour: 4 }, { firstHour: 8, lastHour: 12 }, { firstHour: 19, lastHour: 30 }]);
  assert.deepEqual(cut, [
    { firstHour: 0, lastHour: 2, value: 1 }, { firstHour: 5, lastHour: 7, value: 1 }, { firstHour: 13, lastHour: 18, value: 2 },
  ]);
  assert.deepEqual(stretchesOf(cut).map(stretch => [stretch.firstHour, stretch.lastHour, stretch.steps.length]), [[0, 2, 1], [5, 7, 1], [13, 18, 1]]);
  assert.deepEqual(stretchesOf(steps).map(stretch => [stretch.firstHour, stretch.lastHour, stretch.steps.length]), [[0, 19, 2]]);
});

test('only the lanes in a scrolled view are reported', () => {
  // A view two and a half lanes tall under the axis, at the top: lanes 0..2.
  const height = AXIS_HEIGHT + ROW_HEIGHT * 2.5;
  assert.deepEqual(lanesInView(0, height, 1500), [0, 2]);
  // Scrolled by one and a half lanes: lane 1 (half hidden under the axis) to lane 3.
  assert.deepEqual(lanesInView(ROW_HEIGHT * 1.5, height, 1500), [1, 3]);
  // Never past the last lane; none when there are none.
  assert.deepEqual(lanesInView(ROW_HEIGHT * 1498, height, 1500), [1498, 1499]);
  assert.deepEqual(lanesInView(0, height, 0), [0, -1]);
});
