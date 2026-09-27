import assert from 'node:assert/strict';
import test from 'node:test';
import { mapPresentationMatches } from './presentation.ts';
import type { DashboardMapState } from '../../dashboard/client';
import type { MapTarget } from '../../dashboard/geometry';

const target: MapTarget = { sourceId: 's', path: 'p', revision: 'r', branchId: '1', hour: 20, fraction: 0, sequence: 10, playing: true };
const state: DashboardMapState = { id: 'map', sourceId: 's', revision: 'r', branchId: '1', hour: 15, fraction: 0,
  sequence: 8, requestedSequence: 8, loading: false, error: null };

test('playing, an earlier built hour answers the clock without waiting for its sequence', () => {
  assert.equal(mapPresentationMatches(state, target), true);
  assert.equal(mapPresentationMatches({ ...state, hour: 20 }, target), true);
  for (const change of [{ sourceId: 'other' }, { revision: 'old' }, { branchId: '2' }, { hour: 21 }, { hour: null },
    { loading: true }, { error: 'gap' }]) {
    assert.equal(mapPresentationMatches({ ...state, ...change }, target), false, JSON.stringify(change));
  }
});

test('paused and pinned targets stay exact', () => {
  const paused = { ...target, playing: false };
  assert.equal(mapPresentationMatches(state, paused), false);
  assert.equal(mapPresentationMatches({ ...state, hour: 20 }, paused), false, 'an older sequence');
  assert.equal(mapPresentationMatches({ ...state, hour: 20, sequence: 10 }, paused), true);
  assert.equal(mapPresentationMatches({ ...state, hour: 20, sequence: 10 }, { ...paused, playing: undefined }), true);
});
