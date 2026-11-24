import assert from 'node:assert/strict';
import test from 'node:test';
import { factoryItems, factoryMonth } from './factoryData.ts';
import type { ProductionSpendingLeaf, ProductionSpendingReply, ProductionSpendingRun } from '../../generated/dashboard';

const known = (firstHour: number, lastHour: number, totalRaw: string): ProductionSpendingRun => ({ firstHour, lastHour, totalRaw, reason: null });
const purposeSum = (items: readonly { path: readonly { id: string }[]; exact?: bigint | null }[], id: string) =>
  items.filter(item => item.path[0].id === id).reduce((sum, item) => sum + (item.exact ?? 0n), 0n);
const leaf = (extra: Partial<ProductionSpendingLeaf>): ProductionSpendingLeaf => ({
  variant: '21', typeKey: 'mod_fighter', name: 'Fighters', domain: 'air', rocket: false, purpose: 'new', unproven: [], names: [],
  runs: [known(100, 123, '0'), known(124, 171, '2000')], ...extra,
});

const reply = (): ProductionSpendingReply => ({
  branchId: '1', tag: 'opaque/tag', decimals: 5, reads: [100, 124], observations: [100], warnings: [],
  equipmentReads: [100, 124], rocketReads: [], equipmentObservations: [100], rocketObservations: [],
  rocketKey: null, rocketName: null, rocketTotal: [],
  production: [{ firstHour: 100, lastHour: 171, present: true, emptyCensusHour: null }],
  leaves: [
    leaf({}),
    leaf({ purpose: 'converted', runs: [known(100, 147, '0'), known(148, 171, '300')] }),
    leaf({ variant: '22', typeKey: 'mod_bomber', name: 'Bombers', purpose: 'unknown', runs: [known(100, 147, '0'), known(148, 171, '40')],
      unproven: [known(100, 147, '0'), known(148, 171, '40')] }),
    leaf({ variant: '23', typeKey: 'mod_rifle', name: 'Rifles', domain: 'land', runs: [known(100, 171, '9')] }),
  ],
  total: [known(100, 171, '2349')], totalWithoutRefits: [known(100, 171, '2349')],
  equipmentTotal: [known(100, 123, '9'), known(124, 147, '2009'), known(148, 171, '2349')],
  equipmentTotalWithoutRefits: [known(100, 171, '2349')],
  repairs: null, window: 0, firstHour: 100, lastHour: 171,
});

test('new and converted input of the chosen equipment group by type and design; an unrecorded purpose is its own part', () => {
  const items = factoryItems(reply(), 'air', { from: null, hour: 160, end: 171 });
  assert.equal(purposeSum(items, 'new'), 2000n);
  assert.equal(purposeSum(items, 'converted'), 300n);
  assert.equal(purposeSum(items, 'purpose-unrecorded'), 40n);
  assert.ok(!items.some(item => item.path.some(key => key.id === 'mod_rifle')));
  assert.equal(items.find(item => item.path[0].id === 'new')?.localUnknownLevels?.[0], true);
  assert.equal(factoryItems(reply(), 'air', { from: null, hour: 130, end: 171 }).find(item => item.path[0].id === 'new')?.localUnknownLevels?.[0], false);
  assert.equal(purposeSum(factoryItems(reply(), 'land', { from: null, hour: 160, end: 171 }), 'new'), 9n);
});

test('a month holds new, converted, the unrecorded purpose and the others, and says when its split is not known', () => {
  const month = factoryMonth(reply(), 'air', 148, 171, 100);
  assert.deepEqual(month.bands.map(band => band.exact), [0n, 300n, 40n, 0n]);
  assert.deepEqual(month.total, { exact: 340n, reason: null });
  assert.equal(month.note, 'Whether some of it was converted was not recorded.');
  assert.deepEqual(month.uncertain, [true, true, true, false]);
  assert.deepEqual(factoryMonth(reply(), 'air', 124, 147, 100).bands.map(band => band.exact), [2000n, 0n, 0n, 0n]);
});

