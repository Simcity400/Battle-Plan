import assert from 'node:assert/strict';
import test from 'node:test';
import type { AmountRun, EquipmentStockpileReply, StockVariant } from './types';
import { stockGroups } from './stockpileData.ts';
import { stockRows } from './stockpileView.ts';

const run = (value: number | null): AmountRun[] => [{ firstHour: 100, lastHour: 130, amountRaw: value == null ? null : String(Math.round(value * 100000)) }];
const variant = (id: string, archetype: string, name: string, stock: number, daily?: number, ic?: number): StockVariant => ({
  id, token: archetype, archetype, names: [{ firstHour: 100, name, known: true, design: null }], stock: run(stock),
  daily: daily == null ? [] : run(daily), dailyIc: ic == null ? [] : run(ic), unitCostLow: [], unitCostHigh: [] });
const logistics = (key: string, balance: number, stock: number, available: number) => ({ key, balance: run(balance), stock: run(stock), available: run(available) });
const spans = [{ firstHour: 100, lastHour: 130 }];

const REPLY: EquipmentStockpileReply = {
  branchId: '1', tag: 'GER', campaignKey: 'campaign', decimals: 5, needBoundRaw: '9223372036854775807', warnings: [],
  types: [
    { key: 'infantry_equipment', name: 'Infantry Equipment', domain: 'land', needNote: null },
    { key: 'train_equipment', name: 'Train', domain: 'land', needNote: 'This train archetype needs the native available-stock operand.' },
    { key: 'light_tank_chassis', name: 'Light Tank', domain: 'land', needNote: null },
    { key: 'cas_equipment', name: 'Close Air Support', domain: 'air', needNote: null },
    { key: 'tac_bomber_equipment', name: 'Tactical Bomber', domain: 'air', needNote: null },
  ],
  variants: [
    variant('1', 'infantry_equipment', 'Karabiner 98k', 155_629, 49.3218, 24.6609),
    variant('2', 'train_equipment', 'Class 18.4-5', 71),
    variant('3', 'light_tank_chassis', 'Panzer II Ausf. A', 1_554, 0.78472, 4.653),
    variant('4', 'light_tank_chassis', 'Panzer I Ausf. A', 720),
    variant('5', 'cas_equipment', 'Hs 123', 0),
    variant('6', 'tac_bomber_equipment', 'He 111', 0, 0.03569, 1.74487),
  ],
  logistics: [
    logistics('infantry_equipment', 155_629, 155_629, 155_629), logistics('train_equipment', 56, 71, 56),
    logistics('light_tank_chassis', 2_274, 2_274, 2_274), logistics('cas_equipment', 0, 0, 0), logistics('tac_bomber_equipment', -104, 0, 0),
  ],
  reads: [100], stockHours: spans, lineHours: spans, lineReads: [100], logisticsReads: [100], balanceHours: spans, aiHours: [],
};

const rows = (reply: EquipmentStockpileReply, count: 'units' | 'ic' = 'units') => {
  const groups = stockGroups(reply, 'type', 110);
  return stockRows(reply, groups, 110, reply.aiHours.length ? 'ai' : 'player', count);
};

test('the table lists only types with something, led by the surplus, with models named once', () => {
  const { rows: listed, quiet } = rows(structuredClone(REPLY));
  assert.deepEqual(Object.fromEntries(listed.map(row => [row.group.key, [row.models, row.surplus, row.share]])), {
    infantry_equipment: ['Karabiner 98k', 15_562_900_000n, null],
    train_equipment: ['Class 18.4-5 · 71 in stock, 56 available', 5_600_000n, null],
    light_tank_chassis: ['1,554 Panzer II Ausf. A · 720 Panzer I Ausf. A', 227_400_000n, null],
    tac_bomber_equipment: ['He 111 · 0 in stock', -10_400_000n, -1],
  });
  assert.equal(quiet, 1);
});

test('output is the game’s per-day figure, or the IC put in a day; a type without lines makes none', () => {
  const reply = structuredClone(REPLY);
  const units = Object.fromEntries(rows(reply).rows.map(row => [row.group.key, row.output]));
  assert.equal(units.infantry_equipment, 4_932_180n);
  assert.equal(units.light_tank_chassis, 78_472n);
  assert.equal(units.train_equipment, 0n);
  const ic = Object.fromEntries(rows(reply, 'ic').rows.map(row => [row.group.key, row.output]));
  assert.equal(ic.infantry_equipment, 2_466_090n);
  assert.equal(ic.tac_bomber_equipment, 174_487n);
});

test('for an AI-controlled country the game computes no surplus: the table holds stock alone', () => {
  const ai: EquipmentStockpileReply = { ...structuredClone(REPLY), balanceHours: [], logisticsReads: [], aiHours: spans };
  const { rows: listed, quiet } = rows(ai);
  assert.deepEqual(listed.map(row => [row.group.key, row.surplus, row.stock, row.models]), [
    ['infantry_equipment', null, 15_562_900_000n, 'Karabiner 98k'],
    ['train_equipment', null, 7_100_000n, 'Class 18.4-5'],
    ['light_tank_chassis', null, 227_400_000n, '1,554 Panzer II Ausf. A · 720 Panzer I Ausf. A'],
    ['tac_bomber_equipment', null, 0n, 'He 111'],
  ]);
  assert.equal(quiet, 1);
});

test('a type showing nothing only because a table was not read is not recorded, never empty', () => {
  const { quiet, unrecorded } = rows({ ...structuredClone(REPLY), lineHours: [], lineReads: [] });
  assert.equal(quiet, 0);
  assert.equal(unrecorded, 1);
});
