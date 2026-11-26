import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { FUEL_QUANTITY, validateMarketReply } from '../../dashboard/market.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { resolveTableRows } from '../../families/table/model.ts';
import { tableDefaults } from '../../families/table/settings.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import { COLUMNS, TABLE_BASE } from './definition.ts';
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
  assert.equal(lastDelivery(terms(0, []), zeroValue(0n), notRecorded('loss unread')).kind, 'notRecorded', 'an unread fuel loss is never no delivery');
  assert.match(unread.kind === 'notRecorded' ? unread.reason : '', /fuel unread/);
});

test('an agreement shows its fuel when fuel was delivered, scheduled or not', () => {
  assert.equal(fuelDelivered({ lastFuelDeliveredRaw: '0' }, zeroValue(0n), zeroValue(0n)), false);
  assert.equal(fuelDelivered({ lastFuelDeliveredRaw: '32768' }, zeroValue(0n), zeroValue(0n)), true);
  assert.equal(fuelDelivered({ lastFuelDeliveredRaw: '0' }, knownValue(32_768n), zeroValue(0n)), true);
  assert.equal(fuelDelivered({ lastFuelDeliveredRaw: '0' }, notRecorded('fuel unread'), zeroValue(0n)), false);
  assert.equal(fuelDelivered({ lastFuelDeliveredRaw: '0' }, zeroValue(0n), knownValue(32_768n)), true, 'fuel lost on the way was sent');
  assert.deepEqual(fuelUnits(knownValue(2n ** 53n + 32_767n)), knownValue(2n ** 53n / 32_768n), 'whole units are divided exactly');
  assert.deepEqual(fuelUnits(knownValue(1n)), knownValue(1 / 32_768), 'smallest positive raw fuel stays numeric and nonzero');
  assert.deepEqual(fuelUnits(knownValue(-1n)), knownValue(-1 / 32_768), 'smallest negative raw fuel stays numeric with its sign');
  assert.deepEqual(fuelUnits(zeroValue(0n)), zeroValue(0n), 'recorded zero stays zero');
  const fuelLease = { key: '2', id: '10', giver: 'GER', giverMissing: null, receiver: 'ITA', receiverMissing: null, revision: '1', activeSince: 100, lastDelivery: 110,
    monthly: [], percentage: [], once: [], delivered: [], sunk: [], fuelDailyRaw: '1', fuelPercentageRaw: '0', lastFuelDeliveredRaw: '0' };
  const negFuelLease = { ...fuelLease, key: '3', fuelDailyRaw: '-1' };
  const mixedFuelLease = { ...fuelLease, key: '4', fuelDailyRaw: String(FUEL_QUANTITY + 16_384n) };
  const fuelReply = { branchId: '1', offers: null, contracts: null, agreements: [negFuelLease, fuelLease, mixedFuelLease], agreementsMissing: null, purchasesMissing: null, unattributed: 0, refused: null, variants: [], icons: [], warnings: [] };
  const liveValues = new Map<string, bigint>([
    [liveKey('3', 'fuelSent'), -1n],
    [liveKey('3', 'fuelLost'), 0n],
    [liveKey('2', 'fuelSent'), 1n],
    [liveKey('2', 'fuelLost'), 1n],
    [liveKey('4', 'fuelSent'), FUEL_QUANTITY + 16_384n],
    [liveKey('4', 'fuelLost'), 0n],
  ]);
  const live = { get: (k: string) => liveValues.get(k), missing: () => 'Not read' };
  const fuelModel = agreementsProjection.project(createBoundedReadView({ hour: 120, panels: new Map([['main', fuelReply]]), live: new Map([['main', live]]) }), { direction: 'sent' });
  assert.deepEqual(
    fuelModel.rows.map(row => row.cells.monthly!.value),
    [
      { kind: 'notApplicable', reason: 'Daily -0.000030517578125' },
      { kind: 'notApplicable', reason: 'Daily 0.000030517578125' },
      { kind: 'notApplicable', reason: 'Daily 1.5' },
    ],
    'lend-lease daily fuel schedule preserves exact positive, negative, and mixed amounts',
  );
  assert.deepEqual(
    fuelModel.rows.map(row => {
      const cell = row.cells.delivered!;
      return cell.value.kind === 'known' ? formatScalar(cell.value.value, cell.format) : cell.value.kind;
    }),
    ['\u22120.000030517578125', '0.000030517578125', '1'],
    'subunit live fuel deliveries render exact nonzero text instead of zero',
  );
  const sorted = resolveTableRows(fuelModel, tableDefaults(COLUMNS, { ...TABLE_BASE, sort: { column: 'delivered', direction: 'descending' } })).filter(row => row.kind === 'data');
  assert.deepEqual(sorted.map(row => row.key), ['4:fuel', '2:fuel', '3:fuel'], 'subunit fuel deliveries sort numerically');
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
  const unread = agreementsProjection.project(createBoundedReadView({ hour: 120, panels: new Map([['main', reply]]), live: new Map([['main', { get: () => undefined, missing: () => 'Not read yet' }]]) }), { direction: 'sent' });
  assert.deepEqual(unread.rows.map(row => row.key), ['1:terms'], 'an unread fuel counter alone is not a fuel line');
  assert.deepEqual(unread.rows[0]!.cells.delivered!.value, { kind: 'notRecorded', reason: 'Not read yet' }, 'what was delivered is not known while the fuel counter is unread');
});

test('agreements with an unrecorded giver or receiver keep both flag slots with named unavailable art and preserve separate name/icon reasons', () => {
  const lease = {
    key: '2', id: '10', giver: 'GER', giverMissing: null, receiver: null, receiverMissing: 'Receiver tag was unlinked at this hour',
    revision: '1', activeSince: 100, lastDelivery: 115,
    monthly: [{ variant: '77', name: null, amountRaw: '100000' }], percentage: [], once: [], delivered: [], sunk: [],
    fuelDailyRaw: '0', fuelPercentageRaw: '0', lastFuelDeliveredRaw: '0',
  };
  const reply = {
    branchId: '1', offers: null, contracts: null, agreements: [lease], agreementsMissing: null, purchasesMissing: null, unattributed: 1, refused: null,
    variants: [{ id: '77', token: 'eq_77', name: null, typeName: null, nameUnknown: 'Localisation missing', icon: 'GFX_eq_77', iconUnknown: 'Sprite missing' }],
    icons: [], warnings: [],
  };
  const model = agreementsProjection.project(createBoundedReadView({ hour: 120, panels: new Map([['main', reply]]) }), { direction: 'sent' });
  const flags = model.rows[0]!.cells.country!.flags!;
  assert.equal(flags.length, 2, 'both giver and receiver flag slots are preserved');
  assert.equal(flags[0]!.tag, 'GER');
  assert.equal(flags[1]!.tag, '?');
  assert.equal(flags[1]!.art.status, 'unavailable');
  assert.equal((flags[1]!.art as { readonly reason: string }).reason, 'Receiver tag was unlinked at this hour');
  const eq = model.rows[0]!.cells.equipment!;
  assert.deepEqual(eq.value, { kind: 'notRecorded', reason: 'Localisation missing' });
  assert.equal(eq.art?.status, 'unavailable');
  assert.equal((eq.art as { readonly reason: string }).reason, 'Sprite missing');
});
