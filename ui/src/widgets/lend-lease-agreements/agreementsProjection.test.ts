import assert from 'node:assert/strict';
import test from 'node:test';
import { validateMarketReply } from '../../dashboard/market.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import { agreementsProjection, emptyAgreements, fuelDelivered, fuelUnits, lastDelivery } from './agreementsProjection.ts';

const item = (amountRaw: string) => ({ variant: '6', name: null, amountRaw });
const terms = (lastDelivery: number, delivered: string[], lastFuelDeliveredRaw = '0', sunk: string[] = []) => ({ lastDelivery, delivered: delivered.map(item), sunk: sunk.map(item), lastFuelDeliveredRaw });

test('a last delivery is a date when recorded, no delivery only when neither equipment nor fuel was delivered, and otherwise not recorded', () => {
  const none = zeroValue(0n);
  assert.equal(lastDelivery(terms(60_800_000, []), none, zeroValue(0n)).kind, 'known');
  assert.equal(lastDelivery(terms(0, ['0']), none, zeroValue(0n)).kind, 'notApplicable');
  assert.equal(lastDelivery(terms(0, ['500000']), none, zeroValue(0n)).kind, 'notRecorded');
  assert.equal(lastDelivery(terms(0, [], '0', ['100000']), none, zeroValue(0n)).kind, 'notRecorded', 'equipment lost on the way was sent');
  assert.equal(lastDelivery(terms(0, []), knownValue(32_768n), zeroValue(0n)).kind, 'notRecorded', 'fuel sent is a delivery');
  assert.equal(lastDelivery(terms(0, [], '32768'), none, zeroValue(0n)).kind, 'notRecorded', 'a last fuel delivery is a delivery');
  const unread = lastDelivery(terms(0, []), notRecorded('fuel unread'), zeroValue(0n));
  assert.equal(unread.kind, 'notRecorded', 'unread fuel is never no delivery');
  assert.match(unread.kind === 'notRecorded' ? unread.reason : '', /fuel unread/);
});

test('an agreement shows its fuel when fuel was delivered, scheduled or not', () => {
  assert.equal(fuelDelivered({ lastFuelDeliveredRaw: '0' }, zeroValue(0n), zeroValue(0n)), false);
  assert.equal(fuelDelivered({ lastFuelDeliveredRaw: '32768' }, zeroValue(0n), zeroValue(0n)), true);
  assert.equal(fuelDelivered({ lastFuelDeliveredRaw: '0' }, knownValue(32_768n), zeroValue(0n)), true);
  assert.equal(fuelDelivered({ lastFuelDeliveredRaw: '0' }, notRecorded('fuel unread'), zeroValue(0n)), false);
  assert.equal(fuelDelivered({ lastFuelDeliveredRaw: '0' }, zeroValue(0n), knownValue(32_768n)), true, 'fuel lost on the way was sent');
  assert.deepEqual(fuelUnits(knownValue(2n ** 53n + 32_767n)), knownValue(2n ** 53n / 32_768n), 'whole units are divided exactly');
});

test('an empty list is no lend-lease only when every agreement was read and placed', () => {
  assert.deepEqual(emptyAgreements({ unattributed: 0, agreementsMissing: null }, 'sent'), { empty: 'No lend-lease sent', emptyKind: 'empty' });
  assert.deepEqual(emptyAgreements({ unattributed: 0, agreementsMissing: 'rows unread' }, 'received'), { empty: 'Not recorded', emptyDetail: 'rows unread', emptyKind: 'unavailable' });
  assert.equal(emptyAgreements({ unattributed: 2, agreementsMissing: null }, 'received').emptyKind, 'unavailable');
});

test('a lease without the reasons its sides are unnamed is not a valid reply', () => {
  const lease = { key: '1', id: '9', giver: 'GER', giverMissing: null, receiver: null, receiverMissing: 'unlinked', revision: '1', activeSince: 1, lastDelivery: 0,
    monthly: [], percentage: [], once: [], delivered: [], sunk: [], fuelDailyRaw: '0', fuelPercentageRaw: '0', lastFuelDeliveredRaw: '0' };
  const reply = (agreements: unknown[]) => ({ branchId: '1', offers: null, contracts: null, agreements, agreementsMissing: null, purchasesMissing: null, unattributed: 0, refused: null, variants: [], icons: [], warnings: [] });
  assert.doesNotThrow(() => validateMarketReply(reply([lease])));
  const { giverMissing: _, ...older } = lease;
  assert.throws(() => validateMarketReply(reply([older])));
});

test('an agreement whose terms list nothing still has its row and its delivery date', () => {
  const lease = { key: '1', id: '9', giver: 'GER', giverMissing: null, receiver: 'FRA', receiverMissing: null, revision: '1', activeSince: 100, lastDelivery: 115,
    monthly: [], percentage: [], once: [], delivered: [], sunk: [], fuelDailyRaw: '0', fuelPercentageRaw: '0', lastFuelDeliveredRaw: '0' };
  const reply = { branchId: '1', offers: null, contracts: null, agreements: [lease], agreementsMissing: null, purchasesMissing: null, unattributed: 0, refused: null, variants: [], icons: [], warnings: [] };
  const model = agreementsProjection.project(createBoundedReadView({ hour: 120, panels: new Map([['main', reply]]) }), { direction: 'sent' });
  assert.equal(model.rows.length, 1);
  assert.deepEqual(model.rows[0]!.cells.last!.value, { kind: 'known', value: 115 });
  assert.equal(model.empty, undefined);
});
