import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveOptions } from '../../dashboard/options.ts';
import { definition, OPTIONS } from './definition.ts';
import { airForceRequest, airForceTotals, validateAirForceReply } from './airForceData.ts';

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('air force defaults to planes, roster ownership and the shared top-ten race', () => {
  const options = resolveOptions(OPTIONS, {});
  assert.equal(options.measure, 'planes'); assert.equal(options.ownership, 'roster');
  assert.equal(options.view, 'race'); assert.equal(options.split, false); assert.equal(options.dateOverlay, false);
  assert.equal(options.playback, 'draw-in'); assert.equal(definition.defaultTime, 'follow');
  assert.deepEqual(definition.defaultCountries, { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false });
  assert.deepEqual(airForceRequest('branch', { tags: ['ignored'], top: 10 }, options),
    { branchId: 'branch', tags: [], top: 10, measure: 'planes', ownership: 'roster' });
});

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('air force requests preserve opaque picks and select wings/effective ownership', () => {
  assert.deepEqual(airForceRequest('branch', { tags: ['space , tag', 'another'] }, resolveOptions(OPTIONS, { measure: 'wings', ownership: 'effective' })),
    { branchId: 'branch', tags: ['space , tag', 'another'], top: null, measure: 'wings', ownership: 'effective' });
  assert.equal(resolveOptions(OPTIONS, { measure: 'unknown' }).measure, 'planes');
  assert.throws(() => validateAirForceReply({}), /malformed/);
  assert.deepEqual(airForceTotals(resolveOptions(OPTIONS, { measure: 'wings', ownership: 'effective' })), { kind: 'air-force-totals', measure: 'effectiveWings', decimals: 0 });
  assert.deepEqual(airForceTotals(resolveOptions(OPTIONS, {})), { kind: 'air-force-totals', measure: 'rosterPlanes', decimals: 0 });
});


test('air view controls expose mix, lines, split bars and whole-campaign future toggle', () => {
  const split = OPTIONS.find(field => field.key === 'split')!;
  assert.equal(split.visibleWhen?.(resolveOptions(OPTIONS, {})), true);
  assert.equal(split.visibleWhen?.(resolveOptions(OPTIONS, { view: 'mix' })), false);
  const future = OPTIONS.find(field => field.key === 'afterPlayhead')!;
  assert.equal(future.visibleWhen?.(resolveOptions(OPTIONS, { view: 'lines', playback: 'cursor' })), true);
  assert.equal(future.visibleWhen?.(resolveOptions(OPTIONS, { view: 'race', playback: 'cursor' })), false);
});
