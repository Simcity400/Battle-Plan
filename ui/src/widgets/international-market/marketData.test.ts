import test from 'node:test';
import assert from 'node:assert/strict';
import type { MarketHistoryReply } from '../../generated/dashboard';
import { validateMarketHistory } from '../../dashboard/marketHistory.ts';
import { cic, contractRows, levelText, offerRows } from './marketData.ts';

const base = { branchId: '1', lastHour: 50, known: [[0, 50, '1']], agreements: [], warnings: [],
  holders: { '1': [{ hour: 0, tag: 'SEL' }], '2': [{ hour: 0, tag: 'BUY' }] },
  variants: { a1: [{ firstHour: 0, name: 'Synthetic tank', known: true, design: null }] },
  coverage: { firstHour: 0, lastHour: 50, complete: true }, pending: [] };

test('contracts join their purchase progress and keep both parties', () => {
  const reply = validateMarketHistory('contracts')({ ...base, view: 'contracts', offers: [], contracts: [{
    seller: '1', id: '0000000000000009', buyer: [[10, 30, '2']], speed: [[10, 30, '3']],
    items: [{ pool: 0, ordinal: 0, variant: 'a1', amounts: [[10, 30, '500000']] }, { pool: 0, ordinal: 1, variant: 'b2', amounts: [[10, 30, '100000']] },
      { pool: 1, ordinal: 0, variant: 'a1', amounts: [[10, 30, '2000000']] }],
    levels: [], purchase: [[[12, 30, '2']], [[12, 30, '4']], [[12, 30, '300000']], []],
  }] } as unknown as MarketHistoryReply);
  const [row] = contractRows(reply, 11, tag => tag === 'BUY');
  assert.deepEqual([row.seller, row.buyer, row.equipment.length, row.equipment[0].name, row.equipment[1].name], ['SEL', 'BUY', 2, 'Synthetic tank', null]);
  assert.equal(cic(row.prices[0].amount), '20 CIC');
  // Before the purchase contract's first daily read its progress is unknown.
  assert.equal(row.assigned, null);
  const later = contractRows(reply, 20, () => true)[0];
  assert.deepEqual([later.assigned, later.convoys, later.deliveries], [2n, 4n, null]);
  assert.equal(contractRows(reply, 40, () => true).length, 0);
  assert.equal(contractRows(reply, 20, tag => tag === 'OTH').length, 0);
});

test('inventory lists each variant with its asking level, unset levels unknown', () => {
  const reply = validateMarketHistory('inventory')({ ...base, view: 'inventory', contracts: [], offers: [{ seller: '1',
    inventory: [{ pool: 0, ordinal: 0, variant: 'a1', amounts: [[0, 50, '700000']] }, { pool: 0, ordinal: 1, variant: 'b2', amounts: [[0, 50, '0']] }],
    levels: [{ pool: 2, ordinal: 0, variant: 'a1', amounts: [[0, 50, '2']] }] }] } as unknown as MarketHistoryReply);
  const rows = offerRows(reply, 5, () => true);
  assert.deepEqual(rows.map(row => [row.variant, row.amount, levelText(row.level)]), [['a1', 700000n, 'High'], ['b2', 0n, '—']]);
});
