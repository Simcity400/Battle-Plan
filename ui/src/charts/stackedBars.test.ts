import assert from 'node:assert/strict';
import test from 'node:test';
import { periodAt, periodStops, stackedExtent } from './stackedBars.ts';
import type { StackedPeriod } from './stackedBars';

const periods: StackedPeriod[] = [
  { key: 'a', firstHour: 0, lastHour: 9, parts: [{ key: 'x', value: 3 }, { key: 'y', value: 4 }] },
  { key: 'b', firstHour: 10, lastHour: 19, parts: [], unknown: true },
  { key: 'c', firstHour: 20, lastHour: 29, parts: [{ key: 'x', value: 10 }, { key: 'y', value: -2 }] },
];

test('a period is found for every hour it holds and none between periods', () => {
  assert.equal(periodAt(periods, 0)?.key, 'a');
  assert.equal(periodAt(periods, 19)?.key, 'b');
  assert.equal(periodAt(periods, 29)?.key, 'c');
  assert.equal(periodAt(periods, 30), null);
  assert.equal(periodAt([], 5), null);
  assert.deepEqual(periodStops(periods), [0, 10, 20]);
});

test('extents stack each sign from zero and skip unknown and unreached periods', () => {
  assert.deepEqual(stackedExtent(periods, [0, 29]), [-2, 10]);
  assert.deepEqual(stackedExtent(periods, [0, 15]), [0, 7]);
  assert.equal(stackedExtent(periods, [10, 19]), null, 'an unknown period has no extent, not zero');
  assert.deepEqual(stackedExtent(periods, [0, 29], 15), [0, 7], 'periods after the playhead are not drawn yet');
});

test('five thousand periods resolve hours by binary search', () => {
  const many = Array.from({ length: 5000 }, (_, i) => ({ key: String(i), firstHour: i * 10, lastHour: i * 10 + 9, parts: [] }));
  assert.equal(periodAt(many, 49_995)?.key, '4999');
  assert.equal(periodAt(many, 12_345)?.key, '1234');
});
