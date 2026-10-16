import assert from 'node:assert/strict';
import test from 'node:test';
import { extentOf, heldSegments, rangeBounds, sameStretch, segments } from './series.ts';
import type { Run } from './series';

test('range drawing and fits bound the visible runs by binary search', () => {
  let reads = 0, boundsReads = 0;
  const runs: Run[] = Array.from({ length: 5000 }, (_, i) => ({
    get firstHour() { boundsReads++; return i * 2; }, get lastHour() { boundsReads++; return i * 2; },
    get value() { reads++; return i % 37; }, breakBefore: i === 4000 }));
  assert.deepEqual(rangeBounds(runs, 8000, 8004), [4000, 4003]);
  assert.ok(boundsReads < 40, 'range bounds exclude the offscreen history');
  boundsReads = 0;
  assert.deepEqual(extentOf(runs, 8000, 8004), [4, 6]);
  assert.ok(reads <= 6 && boundsReads < 40, 'fits scan only intersecting values');
  reads = 0; boundsReads = 0;
  assert.equal(segments(runs, 8000, 8004).length, 3);
  assert.ok(reads <= 3 && boundsReads < 50, 'drawing excludes history outside the viewport');
  reads = 0; boundsReads = 0;
  assert.equal(heldSegments(runs, 8000, 8004, x => x, y => y).length, 3);
  assert.ok(reads < 20 && boundsReads < 50);
  assert.equal(extentOf(runs, 1, 1), null, 'gaps remain absent');
  assert.equal(extentOf(runs, 10, 9), null);
  assert.deepEqual(extentOf(runs, 103, 9853), [0, 36]);
});

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

test('playback continuity looks up the supplying run and walks only the intervening stretch', () => {
  let reads = 0;
  const runs = Array.from({ length: 5000 }, (_, i) => ({
    get firstHour() { reads++; return i * 2; }, lastHour: i * 2 + 1,
    startsSegment: i === 3000,
  }));
  assert.equal(sameStretch(runs, 9998, 9999), true);
  assert.ok(reads < 40, 'late playback must not scan from the first run');
  reads = 0;
  assert.equal(sameStretch(runs, 9997, 9998), true);
  assert.ok(reads < 40, 'adjacent playback needs no history index');
  assert.equal(sameStretch(runs, 100, 9000), false);
  assert.equal(sameStretch(runs, 9999, 6000), true);
  assert.equal(sameStretch(runs, 9999, 10000), false);
  assert.equal(sameStretch(runs, NaN, 10), false);
  assert.equal(sameStretch([{ firstHour: 0, lastHour: 1 }, { firstHour: 3, lastHour: 4 }], 1, 3), false);
});
