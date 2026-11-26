import assert from 'node:assert/strict';
import test from 'node:test';
import { keyId } from '../../dashboard/keyedSeriesData.ts';
import { FAMILY_VIEW_UI } from '../manifest.ts';
import { parseBranchFamily, resolveBranchDrillOpen } from './fuelBranchesDrill.ts';

test('fuel-branches registers a view UI wrapper for both bars and over-time views', () => {
  const entry = FAMILY_VIEW_UI['fuel-branches'];
  assert.ok(entry, 'fuel-branches must be registered in FAMILY_VIEW_UI');
  assert.equal(entry.views, undefined, 'wrapper applies to both bars and time views');
});

test('parseBranchFamily resolves standard military branches from bars keys and time-chart keyIds while leaving plot and modded indices unfiltered', () => {
  assert.equal(parseBranchFamily('0'), 'family:0');
  assert.equal(parseBranchFamily('1'), 'family:1');
  assert.equal(parseBranchFamily('2'), 'family:2');
  assert.equal(parseBranchFamily(keyId({ tag: 'slot:1', part: '2' })), 'family:2');
  assert.equal(parseBranchFamily(keyId({ tag: 'GER', part: '1' })), 'family:1');
  assert.equal(parseBranchFamily('3'), null);
  assert.equal(parseBranchFamily(keyId({ tag: 'GER', part: '3' })), null);
  assert.equal(parseBranchFamily('plot:500'), null);
  assert.equal(parseBranchFamily(null), null);
});

test('resolveBranchDrillOpen validates extent, membership, and rejects gap clicks', () => {
  const membership = new Map([['GER', [{ firstHour: 100, lastHour: 500 }]]]);
  assert.deepEqual(
    resolveBranchDrillOpen({ active: true, tag: 'GER', extent: [0, 600], membership, hour: 250.7, subject: '2' }),
    { tag: 'GER', hour: 250, initialLevel: 'family:2' },
  );
  assert.deepEqual(
    resolveBranchDrillOpen({ active: true, tag: 'GER', extent: [0, 600], membership, hour: 300, subject: 'plot:300' }),
    { tag: 'GER', hour: 300, initialLevel: null },
  );
  assert.equal(
    resolveBranchDrillOpen({ active: true, tag: 'GER', extent: [0, 600], membership, hour: 300, subject: 'gap:0' }),
    null,
    'clicking an unrecorded gap does not open a consumption breakdown at the playhead',
  );
  assert.equal(
    resolveBranchDrillOpen({ active: true, tag: 'GER', extent: [0, 600], membership, hour: 50, subject: '0' }),
    null,
    'hours outside membership are rejected',
  );
  assert.equal(
    resolveBranchDrillOpen({ active: true, tag: 'GER', extent: [200, 400], membership, hour: 450, subject: '0' }),
    null,
    'hours outside the widget extent are rejected',
  );
  assert.equal(
    resolveBranchDrillOpen({ active: false, tag: 'GER', extent: [0, 600], membership, hour: 250, subject: '0' }),
    null,
  );
});
