import test from 'node:test';
import assert from 'node:assert/strict';
import { laneRequests } from './laneRequests.ts';

test('names are read for every lane compared; flags only in the window and its margin', () => {
  const lanes = Array.from({ length: 5000 }, (_, index) => index);
  const requests = laneRequests(lanes, [200, 224]);
  assert.equal(requests.names, lanes);
  assert.deepEqual(requests.looks, lanes.slice(192, 233));
  assert.deepEqual(laneRequests(lanes, [0, 2]).looks, lanes.slice(0, 11));
  assert.deepEqual(laneRequests([], [0, -1]), { names: [], looks: [] });
});
