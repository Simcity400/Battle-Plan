import test from 'node:test';
import assert from 'node:assert/strict';
import type { MarketHistoryReply } from '../../generated/dashboard';
import { rosterCount, validateMarketHistory } from '../../dashboard/marketHistory.ts';
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
  const [row] = contractRows(reply, 11, tag => tag === 'BUY').rows;
  assert.deepEqual([row.seller, row.buyer, row.equipment.length, row.equipment[0].name, row.equipment[1].name], ['SEL', 'BUY', 2, 'Synthetic tank', null]);
  assert.equal(cic(row.prices[0].amount), '20 CIC');
  // Before the purchase contract's first daily read its progress is unknown.
  assert.equal(row.assigned, null);
  const later = contractRows(reply, 20, () => true).rows[0];
  assert.deepEqual([later.assigned, later.convoys, later.deliveries], [2n, 4n, null]);
  assert.deepEqual(contractRows(reply, 40, () => true), { rows: [], unattributed: 0 });
  assert.deepEqual(contractRows(reply, 20, tag => tag === 'OTH'), { rows: [], unattributed: 0 });
});

test('a contract whose buyer has no identity stands, and may be any country the seller is not', () => {
  // The buyer is identified until 19; the contract stands, with its speed read, through 30.
  const reply = validateMarketHistory('contracts')({ ...base, view: 'contracts', offers: [], contracts: [{
    seller: '1', id: '0000000000000009', buyer: [[10, 19, '2']], speed: [[10, 30, '3']],
    items: [{ pool: 0, ordinal: 0, variant: 'a1', amounts: [[10, 30, '500000']] }], levels: [], purchase: [[], [], [], []],
  }] } as unknown as MarketHistoryReply);
  const all = contractRows(reply, 25, () => true);
  assert.deepEqual([all.rows.length, all.rows[0].seller, all.rows[0].buyer, all.rows[0].equipment[0].amount, all.unattributed], [1, 'SEL', null, 500000n, 0]);
  assert.deepEqual(contractRows(reply, 25, tag => tag === 'SEL').rows.length, 1);
  // Another country may be the buyer: not listed, but no proof of none.
  assert.deepEqual(contractRows(reply, 25, tag => tag === 'OTH'), { rows: [], unattributed: 1 });
  assert.deepEqual(contractRows(reply, 15, tag => tag === 'OTH'), { rows: [], unattributed: 0 });
});

test('a header counts rows only over a known, prepared roster, and only as a lower bound while some cannot be attributed', () => {
  const reply = validateMarketHistory('contracts')({ ...base, view: 'contracts', offers: [], contracts: [], known: [[0, 30, '1'], [55, 70, '1']] } as unknown as MarketHistoryReply);
  assert.equal(rosterCount(reply, 20, 12, 0), '12');
  assert.equal(rosterCount(reply, 20, 0, 2), '0+');
  // A contract whose seller has no identity leaves the roster unknown: no count, not zero.
  assert.equal(rosterCount(reply, 40, 0, 0), null);
  // Past the prepared history nothing is counted either, though the roster was read.
  assert.equal(rosterCount(reply, 60, 0, 0), null);
});

test('inventory lists each variant with its asking level, unset levels unknown', () => {
  const reply = validateMarketHistory('inventory')({ ...base, view: 'inventory', contracts: [], offers: [{ seller: '1',
    inventory: [{ pool: 0, ordinal: 0, variant: 'a1', amounts: [[0, 50, '700000']] }, { pool: 0, ordinal: 1, variant: 'b2', amounts: [[0, 50, '0']] }],
    levels: [{ pool: 2, ordinal: 0, variant: 'a1', amounts: [[0, 50, '2']] }] }] } as unknown as MarketHistoryReply);
  const rows = offerRows(reply, 5, () => true);
  assert.deepEqual(rows.map(row => [row.variant, row.amount, levelText(row.level)]), [['a1', 700000n, 'High'], ['b2', 0n, '—']]);
});
