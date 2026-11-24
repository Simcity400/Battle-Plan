import assert from 'node:assert/strict';
import test from 'node:test';
import type { SeriesState } from '../../generated/dashboard';
import { formatShare, supplyAt, totalDivisions } from './supplyHealthData.ts';

const state = (tag: string, hour: number, value: string | null, missing: string | null = null): SeriesState => ({
  country: tag,
  hour,
  drop: null,
  missing,
  run: value === null
    ? null
    : {
        firstHour: hour,
        lastHour: hour,
        valueRaw: value,
        slot: 1,
        countryId: 1,
        startsSegment: false,
        segmentFirstHour: hour,
        firstExact: true,
        lastExact: true,
      },
});

test('counts at an hour are the three states, the first reason any is missing, or not read yet', () => {
  const replies = [
    [state('A', 10, '1'), state('B', 10, null, 'Not recorded at this hour')],
    [state('A', 10, '2'), state('B', 10, '0')],
    [state('A', 10, '0'), state('B', 10, '3')],
  ];
  assert.deepEqual(supplyAt(replies, 'A', 10), { counts: { noSupply: 1, strained: 2, inSupply: 0 } });
  assert.deepEqual(supplyAt(replies, 'B', 10), { missing: 'Not recorded at this hour' });
  assert.equal(supplyAt(replies, 'A', 11), undefined);
  assert.equal(supplyAt(replies.slice(0, 2), 'A', 10), undefined);
  assert.equal(supplyAt(undefined, 'A', 10), undefined);
});

test('supply shares use shared share formatting and preserve zero denominators', () => {
  assert.equal(formatShare(1, 3), '33%');
  assert.equal(formatShare(2, 3), '67%');
  assert.equal(formatShare(7, 100), '7.0%');
  assert.equal(formatShare(1, 2000), '<0.1%');
  assert.equal(formatShare(1, 1), '100%');
  assert.equal(formatShare(0, 0), 'Not known');
  assert.equal(totalDivisions({ noSupply: 1, strained: 3, inSupply: 0 }), 4);
});
