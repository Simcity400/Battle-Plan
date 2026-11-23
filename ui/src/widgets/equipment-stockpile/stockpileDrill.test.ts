import { boundCountry, gatedTime } from '../../test-support/gated.ts';
import { boundCountryToken } from '../../dashboard/datedCountry.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import type { AmountRun, EquipmentStockpileReply } from './types';
import { UNRESOLVED_MARK } from '../../dashboard/equipment.ts';
import { createWidget } from '../../dashboard/model.ts';
import { readDatedCountry } from '../../dashboard/datedCountry.ts';
import { contentKey, resolveOptions } from '../../dashboard/options.ts';
import { registerWidgets } from '../../dashboard/definitions.ts';
import { definition, OPTIONS } from './definition.ts';
import { balanceAt, balanceOperandsAt, stockGroups } from './stockpileData.ts';
import { pinStockpileDrill, stockpileDrillGroup } from './stockpileDrill.ts';

const run = (firstHour: number, lastHour: number, amount: bigint): AmountRun => ({ firstHour, lastHour, amountRaw: String(amount) });
const spans = [{ firstHour: 100, lastHour: 123 }, { firstHour: 130, lastHour: 160 }];
const reply: EquipmentStockpileReply = {
  branchId: 'branch', tag: 'AAA', campaignKey: 'campaign-a', decimals: 5, warnings: [],
  reads: [100, 130], logisticsReads: [100, 112, 130], lineReads: [100],
  stockHours: spans, balanceHours: spans, lineHours: spans, aiHours: [], needBoundRaw: '9223372036854775807',
  types: [{ key: 'mod/rifles', name: 'Authored rifles', domain: 'land', needNote: null }],
  variants: [{ id: '9007199254741001', token: 'mod_rifles_a', archetype: 'mod/rifles',
    names: [{ firstHour: 100, known: true, name: 'Recorded A', design: 'design-a' }],
    stock: [run(100, 123, 1200000n), run(130, 160, 0n)], daily: [run(100, 160, 99900000n)], dailyIc: [], unitCostLow: [], unitCostHigh: [] }],
  logistics: [{ key: 'mod/rifles', balance: [run(100, 111, 500000n), run(112, 123, -200000n), run(130, 160, 0n)],
    stock: [run(100, 123, 99900000n)], available: [run(100, 111, 2000000n), run(112, 123, 800000n), run(130, 160, 0n)] }],
};

test('the breakdown uses the same write\u2019s signed available and need operands, not later stock or daily production', () => {
  const group = stockGroups(reply, 'type', Infinity)[0];
  const before = balanceOperandsAt(reply, group, 110)!;
  assert.deepEqual([before.available, before.need, before.balance], [2000000n, 1500000n, 500000n]);
  assert.equal(balanceAt(reply, group, 110)!.stock, 99900000n);
  assert.deepEqual([balanceOperandsAt(reply, group, 112)!.available, balanceOperandsAt(reply, group, 112)!.need], [800000n, 1000000n]);
  assert.equal(balanceOperandsAt(reply, group, 125), null);
  assert.deepEqual([balanceOperandsAt(reply, group, 130)!.available, balanceOperandsAt(reply, group, 130)!.need], [0n, 0n]);
});

test('unknown need, AI and native wrapping keep the known balance and leave the rest unknown', () => {
  const missing: EquipmentStockpileReply = { ...reply, types: [{ ...reply.types[0], needNote: 'Required availability was not recorded' }],
    logistics: [{ ...reply.logistics[0], available: [] }] };
  const group = stockGroups(missing, 'type', Infinity)[0];
  const unknown = balanceOperandsAt(missing, group, 110)!;
  assert.deepEqual([unknown.available, unknown.need, unknown.balance], [null, null, 500000n]);
  const bounded = balanceOperandsAt({ ...reply, needBoundRaw: '1500000' }, group, 110)!;
  assert.deepEqual([bounded.available, bounded.need, bounded.balance], [2000000n, null, 500000n]);
  assert.match(bounded.needNote!, /Need unavailable/);
  const unresolved = { ...reply, logistics: [...reply.logistics,
    { key: `${UNRESOLVED_MARK}mod_unknown`, available: [], balance: [run(100, 123, 0n)], stock: [run(100, 123, 0n)] }] };
  const land = stockGroups(unresolved, 'domain', Infinity).find(item => item.key === 'land')!;
  assert.deepEqual([balanceOperandsAt(unresolved, land, 110)!.available, balanceOperandsAt(unresolved, land, 110)!.need], [null, null]);
  assert.equal(balanceOperandsAt({ ...reply, balanceHours: [], aiHours: spans }, group, 110), null);
  const wrap: EquipmentStockpileReply = { ...reply, logistics: [{ ...reply.logistics[0],
    balance: [run(100, 123, -(1n << 63n))], available: [run(100, 123, (1n << 63n) - 1n)] }] };
  assert.equal(balanceAt(wrap, group, 110)!.need, -1n);
});

test('Pin keeps one native country and semantic type/design; a different campaign or reused handle is refused', () => {
  registerWidgets([definition]);
  const widget = createWidget('equipment-stockpile', 'track', 'source-widget');
  widget.countries = { kind: 'tags', tags: ['AAA', 'BBB'] };
  const track = { id: 'track', branch: { firstHour: 90 } } as Parameters<typeof pinStockpileDrill>[1];
  const country = { campaign: 'campaign-a', branch: 'branch', tag: 'AAA', originOffset: 20 };
  const type = stockGroups(reply, 'type', Infinity)[0];
  const pinned = pinStockpileDrill(widget, track, 'AAA', type, 'type', 'balance', 112.9, gatedTime({ mode: 'pin', windows: { track: { firstOffset: 22, lastOffset: 22 } } }), boundCountryToken(boundCountry(country)))!;
  assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['AAA'] });
  assert.deepEqual(pinned.time.value, { mode: 'pin', windows: { track: { firstOffset: 22, lastOffset: 22 } } });
  const options = resolveOptions(OPTIONS, pinned.options);
  assert.equal(options.show, 'stockpile');
  assert.deepEqual(readDatedCountry(options.stockpileCountry, 'campaign-a', 'branch', pinned.countries), country);
  assert.equal(contentKey(String(options.partGroup)), 'mod/rifles');
  assert.equal(stockpileDrillGroup(stockGroups(reply, 'type', Infinity), String(options.partGroup))?.key, type.key);
  const variant = stockGroups(reply, 'variant', Infinity)[0];
  const variantPin = pinStockpileDrill(widget, track, 'AAA', variant, 'variant', 'stock', 110, gatedTime({ mode: 'follow' }), boundCountryToken(boundCountry(country)))!;
  assert.deepEqual(variantPin.time.value, { mode: 'follow' });
  const token = String(resolveOptions(OPTIONS, variantPin.options).partGroup);
  assert.ok(stockpileDrillGroup(stockGroups(reply, 'variant', Infinity), token));
  assert.equal(stockpileDrillGroup(stockGroups({ ...reply, campaignKey: 'campaign-b' }, 'variant', Infinity), token), undefined);
  const reused = { ...reply, variants: [{ ...reply.variants[0], names: [{ ...reply.variants[0].names[0], design: 'design-b' }] }] };
  assert.equal(stockpileDrillGroup(stockGroups(reused, 'variant', Infinity), token), undefined);
  const unrecorded = stockGroups({ ...reply, variants: [{ ...reply.variants[0], names: [] }] }, 'variant', Infinity)[0];
  assert.equal(pinStockpileDrill(widget, track, 'AAA', unrecorded, 'variant', 'stock', 110, gatedTime({ mode: 'pin', windows: { track: { firstOffset: 20, lastOffset: 20 } } }), boundCountryToken(boundCountry(country))), null);
});
