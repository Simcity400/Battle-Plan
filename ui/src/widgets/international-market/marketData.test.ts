import test from 'node:test';
import assert from 'node:assert/strict';
import { validateMarketReply } from '../../dashboard/market.ts';
import type { MarketAt } from '../../dashboard/market';
import { contractRows, offerRows } from './marketData.ts';
import { internationalMarketProjection } from './internationalMarketProjection.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { liveKey, valuesOf } from '../../charts/live.ts';

const item = (variant: string, amountRaw: string) => ({ variant, name: null, amountRaw });
const raw = { branchId: '1', purchasesMissing: null, unattributed: 1, warnings: [],
  variants: [{ id: '5', hour: 7, known: true, token: 'infantry_equipment', name: 'Rifle', typeName: 'Infantry equipment', nameUnknown: null, icon: 'a', iconUnknown: null }],
  icons: [{ key: 'a', image: 'data:rifle' }],
  offers: [
    { country: 44, tag: 'FRA', inventory: [item('5', '300000'), item('6', '100000')], prices: [{ variant: '6', name: null, level: 2 }], subsidies: [], subsidyTotalRaw: '0', autoSendMarketAccess: false },
    { country: 45, tag: null, inventory: [item('5', '1')], prices: [], subsidies: [], subsidyTotalRaw: '0', autoSendMarketAccess: false },
  ],
  contracts: [
    { key: '1', id: '51', seller: 'FRA', buyer: 'GER', request: false, speed: 1, equipment: [item('5', '100000'), item('6', '1')], prices: [], levels: [], subsidies: [], purchase: true },
    { key: '2', id: '52', seller: 'FRA', buyer: null, request: true, speed: 0, equipment: [item('5', '1')], prices: [], levels: [{ variant: '5', name: null, level: 0 }], subsidies: [], purchase: false },
    { key: '3', id: '53', seller: 'ITA', buyer: 'ENG', request: false, speed: 0, equipment: [], prices: [], levels: [], subsidies: [], purchase: false },
    { key: '4', id: '54', seller: 'ITA', buyer: 'ENG', request: true, speed: 0, equipment: [item('6', '200000')], prices: [], levels: [{ variant: '5', name: null, level: 0 }, { variant: '6', name: null, level: 2 }], subsidies: [], purchase: false },
  ],
  agreements: null };
const reply: MarketAt = { ...validateMarketReply(raw), hour: 7 };

test('contracts and requests are the rows either party matches', () => {
  const ger = (tag: string | null) => tag === 'GER';
  const contracts = contractRows(reply, 'contracts', ger);
  assert.deepEqual(contracts.map(row => row.key), ['1']);
  assert.deepEqual(contracts[0].equipment.map(entry => [entry.name, entry.icon, entry.amount]), [['Rifle', 'data:rifle', 100000n], [null, null, 1n]]);
  assert.deepEqual(contractRows(reply, 'requests', ger), []);
  assert.deepEqual(contractRows(reply, 'requests', () => true)[0].levels, [{ variant: '5', level: 0 }]);
});

test('inventory lists each matching seller\'s variants in native order with their price levels', () => {
  const rows = offerRows(reply, tag => tag !== null);
  assert.deepEqual(rows.map(row => [row.key, row.name, row.amount, row.level]), [['44:5', 'Rifle', 300000n, null], ['44:6', null, 100000n, 2]]);
  assert.equal(rows[1].missing, 'The recording does not name this design at this hour.');
});

test('under All countries an offer whose seller has no country is still listed; under a selection the product\'s count says it may match', () => {
  const view = createBoundedReadView({ hour: 7, panels: new Map([['main', raw]]) });
  const all = internationalMarketProjection.project(view, { view: 'inventory', filterAll: true, filter: { passing: null, unrecorded: [], bounds: [] } });
  assert.ok(all.rows.some(row => row.key === '45:5'), 'the recorded quantity is not dropped');
  const chosen = internationalMarketProjection.project(view, { view: 'requests', filterAll: false, filter: { passing: { GER: [[0, 9]] }, unrecorded: [], bounds: [10] } });
  assert.equal(chosen.emptyKind, 'unavailable', 'a request with an unnamed buyer may be GER\'s');
  assert.equal(internationalMarketProjection.nextChange?.(view, { view: 'requests', filterAll: false, filter: { passing: { GER: [[0, 9]] }, unrecorded: [], bounds: [10] } }), 10);
});


test('contract cells show the live figures exactly as recorded and say why one is missing', () => {
  const collectedRaw = (1n << 60n) + 7n;
  const values = new Map<string, number>([[liveKey('1', 'assigned'), 2], [liveKey('1', 'convoys'), 3], [liveKey('1', 'deliveries'), 4], [liveKey('1', 'collected'), Number(collectedRaw)]]);
  const live = { ...valuesOf(values, 'Convoy progress was not recorded at this hour'), raw: (key: string) => (key === liveKey('1', 'collected') ? collectedRaw : undefined) };
  const model = internationalMarketProjection.project(createBoundedReadView({ hour: 7, panels: new Map([['main', raw]]), live: new Map([['main', live]]) }), {
    view: 'contracts', filterAll: true, filter: { passing: null } as never,
  });
  const cells = model.rows.find(row => row.key === '1')!.cells;
  assert.deepEqual(cells.collected!.value, { kind: 'known', value: collectedRaw }, 'a collected payment beyond 2^53 stays exact');
  assert.deepEqual([cells.convoys!.value, cells.convoys!.secondaryValue], [{ kind: 'known', value: 2 }, { kind: 'known', value: 3 }]);
  assert.deepEqual(cells.deliveries!.value, { kind: 'known', value: 4 });
  const unread = model.rows.find(row => row.key === '3')!.cells;
  assert.deepEqual(unread.collected!.value, { kind: 'notRecorded', reason: 'Convoy progress was not recorded at this hour' });
});

test('each amount, price and level belongs to its own variant: reordered levels never swap prices between variants, and several equipment are listed one by one', () => {
  const view = createBoundedReadView({ hour: 7, panels: new Map([['main', raw]]) });
  const all = { passing: null, unrecorded: [], bounds: [] };
  const requests = internationalMarketProjection.project(view, { view: 'requests', filterAll: true, filter: all });
  assert.deepEqual(requests.rows.find(row => row.key === '4')!.cells.level!.value, { kind: 'known', value: 'High' }, 'variant 6 is High though variant 5 is listed first');
  const contracts = internationalMarketProjection.project(view, { view: 'contracts', filterAll: true, filter: all });
  const several = contracts.rows.find(row => row.key === '1')!;
  assert.equal(several.cells.price!.value.kind, 'notApplicable', 'two equipment never show one price');
  const extra = (typeof several.tooltipExtra === 'function' ? several.tooltipExtra() : several.tooltipExtra) ?? [];
  assert.deepEqual(extra.filter(entry => entry.lookupKey).map(entry => entry.lookupKey), ['5', '6']);
});
