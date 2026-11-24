import assert from 'node:assert/strict';
import test from 'node:test';
import type { EquipmentStockpileReply, StockpileCatalogueReply } from './types';
import { holderAt, stockpileHolders, stockpileView } from './stockpileCatalogue.ts';
import { stockAt, stockGroups, validateStockpileCatalogue } from './stockpileData.ts';

const at = (hour: number, amountRaw: string | null) => [{ firstHour: hour, lastHour: hour, amountRaw }];
const catalogue: StockpileCatalogueReply = {
  branchId: '1', countries: ['GER'], campaignKey: 'campaign', decimals: 5, needBoundRaw: '9223372036854775807', warnings: ['catalogue'],
  types: [{ key: 'infantry_equipment', name: 'Infantry equipment', domain: 'land', needNote: null }],
  variants: [
    { id: '10', token: 'infantry_equipment_1', archetype: 'infantry_equipment', designs: ['design-a'],
      names: [{ firstHour: 100, name: 'Rifle', known: true, design: 'design-a' }] },
    { id: '20', token: 'infantry_equipment_1', archetype: 'infantry_equipment', designs: [], names: [] },
  ],
  logistics: [{ key: 'infantry_equipment', nonzero: true }],
  reads: [100, 124], lineReads: [100, 124], logisticsReads: [100], stockHours: [{ firstHour: 100, lastHour: 147 }],
  lineHours: [{ firstHour: 100, lastHour: 147 }], balanceHours: [{ firstHour: 100, lastHour: 147 }], aiHours: [],
  choices: { '["type","infantry_equipment"]': { raw: '30', stockDrawable: true, shareDrawable: false } },
};
const hour: EquipmentStockpileReply = {
  branchId: '1', tag: 'GER', campaignKey: 'campaign', decimals: 5, needBoundRaw: '9223372036854775807', warnings: ['catalogue', 'hour'],
  types: [], variants: [
    { id: '10', token: 'infantry_equipment_1', archetype: 'infantry_equipment', names: [], stock: at(120, '30'), daily: [], dailyIc: [], unitCostLow: [], unitCostHigh: [] },
    { id: '30', token: '', archetype: null, names: [], stock: at(120, '1'), daily: [], dailyIc: [], unitCostLow: [], unitCostHigh: [] },
  ],
  logistics: [], reads: [100], lineReads: [], logisticsReads: [], stockHours: [{ firstHour: 120, lastHour: 120 }], lineHours: [], balanceHours: [], aiHours: [],
};

test('an hour’s values join the catalogue: its variants, reads and peaks, with any variant it names first', () => {
  assert.equal(validateStockpileCatalogue(catalogue), catalogue);
  const peaked = { ...catalogue, choices: { '["type","infantry_equipment"]': { raw: '101457092405402533877', stockDrawable: true, shareDrawable: false } } };
  assert.equal(validateStockpileCatalogue(peaked), peaked);
  assert.throws(() => validateStockpileCatalogue({ ...catalogue, choices: { x: { raw: '-1', stockDrawable: true, shareDrawable: false } } }));
  const view = stockpileView(catalogue, hour, 120);
  assert.deepEqual(view.variants.map(variant => [variant.id, variant.stock.length]), [['10', 1], ['20', 0], ['30', 1]]);
  assert.equal(view.variants[0].names[0].name, 'Rifle');
  assert.deepEqual(view.reads, [100]);
  assert.deepEqual(view.warnings, ['catalogue', 'hour']);
  assert.deepEqual(view.prepared, { hour: 120, reads: [100, 124], logisticsReads: [100], stockRecorded: true, linesRecorded: true,
    balanceRecorded: true, aiRecorded: false, nonzeroLogistics: ['infantry_equipment'], choices: catalogue.choices, designs: { 10: ['design-a'], 20: [] } });
  const land = stockGroups(view, 'domain', 120).find(group => group.key === 'land')!;
  assert.equal(stockAt(view, land, 120), null);
  assert.equal(stockAt(stockpileView(catalogue, undefined, 120), land, 120), null);
});

test('an alias history reads each hour from the tag it held then', () => {
  const holders = stockpileHolders(new Map([['GER', [{ firstHour: 0, lastHour: 109 }]], ['DDR', [{ firstHour: 110, lastHour: 114 }]]]));
  assert.deepEqual(holders.map(holder => holder.country), ['GER', 'DDR']);
  assert.equal(holderAt(holders, 100), 'GER');
  assert.equal(holderAt(holders, 112), 'DDR');
  assert.equal(holderAt(holders, 116), null);
  assert.equal(holderAt([{ country: 'GER' }], 116), 'GER');
});

test('a reply held while the next hour loads is shown at its own hour, never read as the next', () => {
  const held = stockpileView(catalogue, hour, 120);
  const rifles = stockGroups(held, 'variant', 120)[0];
  assert.equal(held.prepared?.hour, 120);
  assert.equal(stockAt(held, rifles, held.prepared!.hour), 30n);
  assert.equal(stockAt(held, rifles, 121), null);
});
