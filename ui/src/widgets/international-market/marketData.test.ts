import test from 'node:test';
import assert from 'node:assert/strict';
import { validateMarketReply } from '../../dashboard/market.ts';
import type { MarketAt } from '../../dashboard/market';
import { contractRows, offerRows } from './marketData.ts';

const item = (variant: string, amountRaw: string) => ({ variant, name: null, amountRaw });
const reply: MarketAt = { ...validateMarketReply({ branchId: '1', purchasesMissing: null, unattributed: 1, warnings: [],
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
  ],
  agreements: null }), hour: 7 };

test('contracts and requests are the rows either party matches; an unnamed party may match and is counted', () => {
  const ger = (tag: string) => tag === 'GER';
  const contracts = contractRows(reply, 'contracts', ger);
  assert.deepEqual(contracts.rows.map(row => row.key), ['1']);
  assert.equal(contracts.unattributed, 0);
  assert.deepEqual(contracts.rows[0].equipment.map(entry => [entry.name, entry.icon, entry.amount]), [['Rifle', 'data:rifle', 100000n], [null, null, 1n]]);
  const requests = contractRows(reply, 'requests', ger);
  assert.deepEqual([requests.rows.length, requests.unattributed], [0, 1], 'a request whose buyer has no tag may be GER\'s');
  assert.deepEqual(contractRows(reply, 'requests', () => true).rows[0].levels, [{ variant: '5', level: 0 }]);
});

test('inventory lists each matching seller\'s variants in native order with their price levels; a seller with no tag is not listed', () => {
  const rows = offerRows(reply, () => true);
  assert.deepEqual(rows.map(row => [row.key, row.name, row.amount, row.level]), [['44:5', 'Rifle', 300000n, null], ['44:6', null, 100000n, 2]]);
  assert.equal(rows[1].missing, 'The recording does not name this design at this hour.');
});

