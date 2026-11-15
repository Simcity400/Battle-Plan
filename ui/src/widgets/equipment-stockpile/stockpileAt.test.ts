import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { atReply, lineFigures, validateStockpileAt } from './stockpileAt.ts';
import { stockAt, dailyAt, balanceMode, balanceAt } from './stockpileData.ts';
import type { StockpileAtReply } from '../../generated/dashboard';

const cells = (entries: [string, string, number][]) => valuesOf(new Map(entries.map(([row, column, value]) => [liveKey(row, column), value])), 'Not recorded at this hour');

const reply = (over: Partial<StockpileAtReply> = {}): StockpileAtReply => validateStockpileAt({
  tag: 'GER', campaignKey: 'c', decimals: 5, types: [{ key: 'infantry_equipment', name: 'Infantry Equipment', domain: 'land', needNote: null }],
  variants: [
    { id: '11', token: 'infantry_equipment_1', archetype: 'infantry_equipment', name: 'Rifle I', known: true, design: 'aa', held: true, lines: ['5', '6'], outputMissing: null },
    { id: '12', token: 'infantry_equipment_2', archetype: 'infantry_equipment', name: 'Rifle II', known: true, design: 'bb', held: true, lines: [], outputMissing: null },
  ],
  logistics: [{ key: 'infantry_equipment' }], stockMissing: null, linesMissing: null, balance: 'player', balanceMissing: null,
  needBoundRaw: '92233720368', warnings: [], ...over,
});

test('a design’s output a day sums each line’s own truncated rate; a line without a cost leaves it unknown', () => {
  assert.deepEqual(lineFigures(['5', '6'], cells([['l5', 'speed', 250000], ['l5', 'cost', 700000], ['l6', 'speed', 100000], ['l6', 'cost', 300000]])),
    { daily: 35714n + 33333n, ic: 350000n, low: 300000n, high: 700000n });
  assert.equal(lineFigures(['5'], cells([['l5', 'speed', 250000], ['l5', 'cost', 0]])).daily, null);
  assert.equal(lineFigures(['5'], cells([['l5', 'speed', 250000]])).ic, null, 'a missing cell is unknown, never zero');
});

test('the hour’s view holds the cells’ values, a missing cell as not recorded and an AI read without a balance', () => {
  const values = cells([['v11', 'stock', 1000], ['v12', 'stock', 0], ['l5', 'speed', 100000], ['l5', 'cost', 100000], ['l6', 'speed', 100000], ['l6', 'cost', 100000],
    ['tinfantry_equipment', 'balance', -500], ['tinfantry_equipment', 'stock', 1000], ['tinfantry_equipment', 'available', 1000]]);
  const view = atReply(reply(), values, 40, '1');
  const type = { variants: view.variants, types: ['infantry_equipment'], kind: 'type' as const, key: 'infantry_equipment' };
  assert.equal(stockAt(view, type, 40), 1000n);
  assert.equal(dailyAt(view, type, 40), 200000n);
  assert.equal(balanceMode(view, 40), 'player');
  assert.deepEqual(balanceAt(view, type, 40), { balance: -500n, stock: 1000n, need: 1500n, needNote: null });
  const missing = atReply(reply(), cells([]), 40, '1');
  assert.equal(stockAt(missing, { variants: missing.variants }, 40), null);
  const ai = atReply(reply({ balance: 'ai', logistics: [] }), values, 40, '1');
  assert.equal(balanceMode(ai, 40), 'ai');
  const unread = atReply(reply({ stockMissing: 'The stockpile was not read within a day of this hour.' }), values, 40, '1');
  assert.equal(stockAt(unread, type, 40), null, 'a stockpile not read is not recorded, never empty');
  assert.ok(unread.warnings.includes('The stockpile was not read within a day of this hour.'));
});
