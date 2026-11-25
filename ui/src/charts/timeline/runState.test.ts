import test from 'node:test';
import assert from 'node:assert/strict';
import { bridgeGaps, coverCounts, describeRun, runState, stretchesOf, withoutHours } from './runState.ts';

test('runs joined across unrecorded hours are one stretch; a recorded break is not', () => {
  const runs = [{ firstHour: 100, lastHour: 199 }, { firstHour: 210, lastHour: 300 }, { firstHour: 320, lastHour: 400 }];
  const stretches = bridgeGaps(runs, [{ firstHour: 200, lastHour: 209 }]);
  assert.deepEqual(stretches.map(item => [item.firstHour, item.lastHour, item.runs.length]), [[100, 300, 2], [320, 400, 1]]);
  assert.deepEqual(runState([stretches[0]!], 300), { kind: 'held', since: 100 });
  assert.deepEqual(runState([stretches[0]!], 310), { kind: 'ended', at: 301 });
  assert.equal(bridgeGaps(runs, [{ firstHour: 200, lastHour: 209 }], () => false).length, 3);
  assert.equal(bridgeGaps([{ firstHour: 0, lastHour: 9 }, { firstHour: 10, lastHour: 19 }]).length, 1);
});

test('fragmented, touching and overlapping unobserved spans bridge only fully covered gaps', () => {
  const runs = [0, 5, 10, 15, 20].map(hour => ({ firstHour: hour, lastHour: hour }));
  const spans = [[1, 2], [2, 3], [4, 4], [6, 7], [8, 9], [11, 12], [13, 13], [16, 30], [17, 18]]
    .map(([firstHour, lastHour]) => ({ firstHour, lastHour }));
  assert.deepEqual(bridgeGaps(runs, spans).map(stretch => [stretch.firstHour, stretch.lastHour, stretch.runs.length]), [[0, 10, 3], [15, 20, 2]]);
  assert.deepEqual(bridgeGaps(runs, [...spans].reverse()), bridgeGaps(runs, spans));
  assert.equal(bridgeGaps(runs, [{ firstHour: 1, lastHour: 30 }]).length, 1);
});


const words = { since: (h: number) => `since ${h}`, before: (h: number) => `before ${h}`, ended: (h: number) => `ended ${h}`,
  endedBy: (h: number) => `ended by ${h}`, lastSeen: (h: number) => `last ${h}` };

test('a run is held, ended, ended by a day or unknown, honestly about unrecorded hours', () => {
  const runs = [{ firstHour: 10, lastHour: 19 }, { firstHour: 40, lastHour: 49 }];
  assert.deepEqual(runState(runs, 5), { kind: 'none' });
  assert.deepEqual(runState(runs, 15), { kind: 'held', since: 10 });
  assert.deepEqual(runState(runs, 25), { kind: 'ended', at: 20 });
  const gap = [{ firstHour: 50, lastHour: 59 }];
  assert.deepEqual(runState(runs, 55, gap), { kind: 'unknown', lastSeen: 49 });
  assert.deepEqual(runState(runs, 65, gap), { kind: 'ended', at: 60, lastSeen: 49 });
  assert.equal(describeRun(runs, 5, words), null);
  assert.equal(describeRun(runs, 15, words, [], 0), 'since 10');
  assert.equal(describeRun(runs, 15, words, [], 10), 'before 10');
  assert.equal(describeRun(runs, 45, words, [{ firstHour: 30, lastHour: 39 }], 0), 'before 40');
  assert.equal(describeRun(runs, 65, words, gap), 'ended by 60');
  assert.equal(describeRun(runs, 55, words, gap), 'last 49');
});

test('a recorded gap touching hours still being prepared is one unknown stretch, never an end', () => {
  const runs = [{ firstHour: 10, lastHour: 19 }];
  const spans = [{ firstHour: 20, lastHour: 29 }, { firstHour: 30, lastHour: 100 }];
  assert.deepEqual(runState(runs, 50, spans), { kind: 'unknown', lastSeen: 19 });
  assert.deepEqual(runState(runs, 50, [...spans].reverse()), { kind: 'unknown', lastSeen: 19 });
  assert.deepEqual(runState(runs, 101, spans), { kind: 'ended', at: 101, lastSeen: 19 });
  assert.deepEqual(runState(runs, 50, [{ firstHour: 20, lastHour: 29 }, { firstHour: 31, lastHour: 100 }]), { kind: 'ended', at: 30, lastSeen: 19 });
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
