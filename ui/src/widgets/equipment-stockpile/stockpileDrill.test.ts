import assert from 'node:assert/strict';
import test from 'node:test';
import type { AmountRun, EquipmentStockpileReply } from '../../generated/dashboard';
import { runAt } from '../../charts/series.ts';
import { buildStack, stackExtent } from '../../charts/stack.ts';
import { NumericReply } from '../../dashboard/numericReply.ts';
import { UNRESOLVED_MARK } from '../../dashboard/equipment.ts';
import { createWidget } from '../../dashboard/model.ts';
import { datedCountrySpans, readDatedCountry } from '../../dashboard/datedCountry.ts';
import { contentKey, resolveOptions } from '../../dashboard/options.ts';
import { registerWidgets } from '../../dashboard/definitions.ts';
import { definition, OPTIONS } from './definition.ts';
import { balanceAt, stockGroups } from './stockpileData.ts';
import { pinStockpileDrill, stockpileCountryData, stockpileDrillGroup, stockpileNumbers, stockpilePartAmount, stockpilePartHistory, stockpilePartsAt } from './stockpileDrill.ts';

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

test('moment and history use the same signed available/need operands, not later stock or daily production', () => {
  const group = stockGroups(reply, 'type', Infinity)[0];
  const before = stockpilePartsAt(reply, group, 110, 'balance');
  assert.deepEqual(before.parts.map(part => [part.id, part.raw]), [['available', 2000000n], ['need', -1500000n], ['other', 0n]]);
  assert.equal(before.total, 500000n);
  assert.deepEqual(before.parts.map(part => stockpilePartAmount(part.raw, reply.decimals)), ['+20', '−15', '0']);
  assert.equal(stockpilePartAmount(1n, reply.decimals), '+0.00001');
  assert.equal(stockpilePartAmount(null, reply.decimals), 'Not recorded');
  assert.equal(balanceAt(reply, group, 110)!.stock, 99900000n);
  const history = stockpilePartHistory(reply, group, 'balance');
  for (const hour of [100, 111, 112, 123, 130, 160]) {
    const sample = stockpilePartsAt(reply, group, hour, 'balance');
    assert.equal(sample.parts.reduce((total, part) => total + part.raw!, 0n), sample.total);
    assert.deepEqual(history.series.map(part => runAt(part.runs, hour)?.value), sample.parts.map(part => Number(part.raw!) / 100000));
  }
  assert.ok(history.series.every(part => runAt(part.runs, 125) == null && runAt(part.runs, 130)?.breakBefore));
  const percent = stockpilePartHistory(reply, group, 'balance', true);
  assert.equal(runAt(percent.series[1].runs, 110)!.value, -100);
  assert.equal(runAt(percent.series[0].runs, 112)!.value, 80);
  assert.ok(percent.series.every(part => runAt(part.runs, 130) == null));
  assert.deepEqual(stockpilePartsAt(reply, group, 130, 'balance').parts.map(part => part.raw), [0n, 0n, 0n]);
});

test('unknown need, AI, variants and native wrapping retain known amounts without a partial history stack', () => {
  const missing: EquipmentStockpileReply = { ...reply, types: [{ ...reply.types[0], needNote: 'Required availability was not recorded' }],
    logistics: [{ ...reply.logistics[0], available: [] }] };
  const group = stockGroups(missing, 'type', Infinity)[0];
  assert.deepEqual(stockpilePartsAt(missing, group, 110, 'balance').parts.map(part => part.raw), [null, null, null]);
  assert.equal(stockpilePartsAt(missing, group, 110, 'balance').total, 500000n);
  assert.match(stockpilePartsAt(missing, group, 110, 'balance').note!, /Need unavailable/);
  assert.ok(stockpilePartHistory(missing, group, 'balance').series.every(part => part.runs.length === 0));
  const bounded = { ...reply, needBoundRaw: '1500000' };
  const boundedParts = stockpilePartsAt(bounded, group, 110, 'balance');
  assert.deepEqual(boundedParts.parts.map(part => part.raw), [2000000n, null, null]);
  assert.equal(boundedParts.total, 500000n);
  assert.match(boundedParts.note!, /Need unavailable/);
  const boundedHistory = stockpilePartHistory(bounded, group, 'balance');
  assert.equal(runAt(boundedHistory.series[0].runs, 110)!.value, 20);
  assert.equal(stackExtent(buildStack(boundedHistory.series.map(part => part.runs)), 110, 110), null);
  // A listed, unresolved zero type may hold native availability in any arm;
  // an already refused need must not leave a partial arm's available sum known.
  const unresolved = { ...bounded, logistics: [...bounded.logistics,
    { key: `${UNRESOLVED_MARK}mod_unknown`, available: [], balance: [run(100, 123, 0n)], stock: [run(100, 123, 0n)] }] };
  const land = stockGroups(unresolved, 'domain', Infinity).find(item => item.key === 'land')!;
  assert.deepEqual(stockpilePartsAt(unresolved, land, 110, 'balance').parts.map(part => part.raw), [null, null, null]);
  const ai = { ...reply, balanceHours: [], aiHours: spans };
  assert.equal(stockpilePartsAt(ai, group, 110, 'balance').total, null);
  assert.equal(stockpilePartsAt(ai, group, 110, 'stock').total, 1200000n);
  const variant = stockGroups(reply, 'variant', Infinity)[0];
  assert.deepEqual(stockpilePartsAt(reply, variant, 110, 'stock').parts.map(part => part.raw), [1200000n]);
  assert.match(stockpilePartsAt(reply, variant, 110, 'stock').note!, /by equipment type/);

  const wrap: EquipmentStockpileReply = { ...reply, logistics: [{ ...reply.logistics[0],
    balance: [run(100, 123, -(1n << 63n))], available: [run(100, 123, (1n << 63n) - 1n)] }] };
  const wrapped = stockpilePartsAt(wrap, group, 110, 'balance');
  assert.deepEqual(wrapped.parts.map(part => part.raw), [(1n << 63n) - 1n, 1n, -(1n << 64n)]);
  assert.equal(wrapped.parts.reduce((total, part) => total + part.raw!, 0n), wrapped.total);
  assert.equal(balanceAt(wrap, group, 110)!.need, -1n);
});

test('Pin keeps one native country and semantic type/design; a different campaign or reused handle is refused', () => {
  registerWidgets([definition]);
  const widget = createWidget('equipment-stockpile', 'track', 'source-widget');
  widget.countries = { kind: 'tags', tags: ['AAA', 'BBB'] };
  const track = { id: 'track', branch: { firstHour: 90 } } as Parameters<typeof pinStockpileDrill>[1];
  const country = { campaign: 'campaign-a', branch: 'branch', tag: 'AAA', originOffset: 20 };
  const type = stockGroups(reply, 'type', Infinity)[0];
  const pinned = pinStockpileDrill(widget, track, 'AAA', type, 'type', 'balance', 112.9, false, country)!;
  assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['AAA'] });
  assert.deepEqual(pinned.time, { mode: 'pin', windows: { track: { firstOffset: 22, lastOffset: 22 } } });
  const options = resolveOptions(OPTIONS, pinned.options);
  assert.equal(options.show, 'stockpile');
  assert.deepEqual(readDatedCountry(options.stockpileCountry, 'campaign-a', 'branch', pinned.countries), country);
  assert.equal(contentKey(String(options.partGroup)), 'mod/rifles');
  assert.equal(stockpileDrillGroup(stockGroups(reply, 'type', Infinity), String(options.partGroup))?.key, type.key);
  const variant = stockGroups(reply, 'variant', Infinity)[0];
  const variantPin = pinStockpileDrill(widget, track, 'AAA', variant, 'variant', 'stock', 110, true, country)!;
  assert.deepEqual(variantPin.time, { mode: 'follow' });
  const token = String(resolveOptions(OPTIONS, variantPin.options).partGroup);
  assert.ok(stockpileDrillGroup(stockGroups(reply, 'variant', Infinity), token));
  assert.equal(stockpileDrillGroup(stockGroups({ ...reply, campaignKey: 'campaign-b' }, 'variant', Infinity), token), undefined);
  const reused = { ...reply, variants: [{ ...reply.variants[0], names: [{ ...reply.variants[0].names[0], design: 'design-b' }] }] };
  assert.equal(stockpileDrillGroup(stockGroups(reused, 'variant', Infinity), token), undefined);
  const unrecorded = stockGroups({ ...reply, variants: [{ ...reply.variants[0], names: [] }] }, 'variant', Infinity)[0];
  assert.equal(pinStockpileDrill(widget, track, 'AAA', unrecorded, 'variant', 'stock', 110, false, country), null);
});

test('content-only pending/recovery keeps numeric identity but request, design and amount changes do not', () => {
  const retained = new NumericReply<EquipmentStockpileReply>();
  const original = retained.update('request-a', reply, stockpileNumbers)!;
  assert.equal(original.types[0].name, null);
  assert.equal(original.variants[0].names[0].name, null);
  assert.equal(original.variants[0].names[0].design, 'design-a');
  assert.equal(retained.update('request-a', undefined, stockpileNumbers), original);
  const renamed = { ...reply, types: [{ ...reply.types[0], name: 'Authored rename' }],
    variants: [{ ...reply.variants[0], names: [{ ...reply.variants[0].names[0], name: 'Recorded rename' }] }] };
  assert.equal(retained.update('request-a', renamed, stockpileNumbers), original);
  assert.notEqual(retained.update('request-a', { ...renamed, variants: [{ ...renamed.variants[0],
    names: [{ ...renamed.variants[0].names[0], design: 'new-design' }] }] }, stockpileNumbers), original);
  assert.equal(retained.update('request-b', undefined, stockpileNumbers), undefined);
});


test('stockpile quantities and part histories follow the native aliases and keep an absent interval', () => {
  const pick = { campaign: 'campaign-a', branch: 'branch', tag: 'X', originOffset: 5 };
  const identities = [{ tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 },
    { firstHour: 110, lastHour: 160, slot: 2, countryId: 20 }] }, { tag: 'Y', identities: [
    { firstHour: 110, lastHour: 114, slot: 1, countryId: 10 }, { firstHour: 117, lastHour: 160, slot: 1, countryId: 10 }] }];
  const spans = datedCountrySpans(identities, pick, 100, 160)!;
  const x = { ...reply, tag: 'X', stockHours: [{ firstHour: 100, lastHour: 160 }], reads: [100, 110],
    variants: [{ ...reply.variants[0], stock: [run(100, 109, 1100000n), run(110, 160, 9900000n)] }] };
  const y = { ...reply, tag: 'Y', stockHours: [{ firstHour: 110, lastHour: 160 }], reads: [110, 117],
    variants: [{ ...reply.variants[0], stock: [run(110, 160, 1300000n)] }] };
  const data = stockpileCountryData([x, y], spans, 'X')!;
  const group = stockGroups(data, 'type', Infinity)[0];
  assert.equal(stockpilePartsAt(data, group, 105, 'stock').total, 1100000n);
  assert.equal(stockpilePartsAt(data, group, 112, 'stock').total, 1300000n);
  assert.equal(stockpilePartsAt(data, group, 116, 'stock').total, null);
  assert.equal(stockpilePartsAt(data, group, 118, 'stock').total, 1300000n);
  const history = stockpilePartHistory(data, group, 'stock');
  assert.equal(runAt(history.series[0].runs, 112)?.value, 13);
  assert.equal(runAt(history.series[0].runs, 116), null);
  assert.equal(x.variants[0].stock[1].amountRaw, '9900000', 'source replies are not mutated');
  const clipped = stockpileCountryData([y], new Map([['Y', [{ firstHour: 112, lastHour: 114 }]]]), 'X')!;
  assert.deepEqual(clipped.reads, [110], 'a clipped history retains its actual supplying read date');
});
