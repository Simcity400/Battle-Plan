import assert from 'node:assert/strict';
import test from 'node:test';
import { extentOf, sameStretch } from './series.ts';
import type { Run } from './series';


test('range extrema retain inclusive edge comparisons and exceptional numeric values', () => {
  const lists: Run[][] = [[], [{ firstHour: 0, lastHour: 0, value: -0 }],
    Array.from({ length: 193 }, (_, i) => ({ firstHour: i * 2, lastHour: i * 2 + 1, value: [0, -0, 7, -7][i % 4] })),
  ];
  for (const special of [NaN, Infinity, -Infinity]) lists.push(lists[2].map((run, i) => i === 64 ? { ...run, value: special } : run));
  for (const runs of lists) for (const from of [NaN, -Infinity, -0, 0.5, 1.75, 127.5, 129, 383, Infinity]) {
    for (const to of [NaN, -Infinity, 0, 1.5, 128, 255.5, 384, Infinity]) {
      let low = Infinity, high = -Infinity;
      for (const run of runs) if (!(run.lastHour < Math.floor(from) || run.firstHour > to)) {
        low = Math.min(low, run.value); high = Math.max(high, run.value);
      }
      assert.deepEqual(extentOf(runs, from, to), low <= high ? [low, high] : null, `${from}..${to}`);
    }
  }
});

test('continuity retains fractional ending bounds across adjacent reads', () => {
  const runs = [{ firstHour: 0, lastHour: 1 }, { firstHour: 2, lastHour: 3 }, { firstHour: 4, lastHour: 5 }];
  assert.equal(sameStretch(runs, 0, 1.5), true);
  assert.equal(sameStretch(runs, 0, 3.5), true);
  assert.equal(sameStretch(runs, 1.5, 3), false);
  assert.equal(sameStretch(runs, 0, 5.5), false);
  assert.equal(sameStretch([{ firstHour: 0, lastHour: 1 }, { firstHour: 2, lastHour: 3, startsSegment: true }], 0, 1.5), false);
});

