import assert from 'node:assert/strict';
import test from 'node:test';
import type { EquipmentProducedReply, ProducedKey } from '../../generated/dashboard';
import { formatProduced, formatRaw, producedModel, producedRequest, requestParams, validateProducedReply } from './producedData.ts';
import { mostDelivered, producedCaption, typeRows, typeValues } from './producedView.ts';

const c = (from: number, deltas: number[], unknown: number[] = []) => ({ from, deltas, unknown });
const key = (item: Partial<ProducedKey> & { key: string }): ProducedKey => ({ name: null, typeKey: null, domain: null, names: [], ...item });

test('the caption says what is counted, over which window and to which date', () => {
  assert.equal(producedCaption('units', 'day', '22 Jun 1944', '24 Oct 1943'), 'Units delivered on 22 Jun 1944');
  assert.equal(producedCaption('ic', 'month', '22 Jun 1944', null), 'IC put into equipment in the month to 22 Jun 1944');
  assert.equal(producedCaption('units', 'total', '22 Jun 1944', '24 Oct 1943'), 'Units delivered from 24 Oct 1943 to 22 Jun 1944');
  // A chart over time has no single date.
  assert.equal(producedCaption('ic', 'day', null, null), 'IC put into equipment per day');
  assert.equal(producedCaption('units', 'total', null, '24 Oct 1943'), 'Units delivered since 24 Oct 1943');
});

test('IC is written exactly from its fixed5 integer, whole from 10 and two figures below; units stay whole', () => {
  assert.equal(formatRaw(73_705_000, 5), '737');
  assert.equal(formatRaw(172_160, 5), '1.7');
  assert.equal(formatRaw(8_795, 5), '0.088');
  assert.equal(formatRaw(1_310, 0), '1,310');
  // A plotted value recovers its recorded integer before it is written.
  assert.equal(formatProduced(737.05, 5), '737');
  assert.equal(formatProduced(0.10139, 5), '0.1');
});

test('units leave the count out of the request, so an older bridge still answers; IC names it', () => {
  const units = producedRequest('1', { tags: ['SOV'] }, 'day', 'variant', ['infantry_equipment'], false);
  assert.equal(units.count, 'units');
  assert.equal('count' in requestParams(units), false);
  const ic = producedRequest('1', { tags: [], top: 10 }, 'day', 'type', [], false, 'ic');
  assert.deepEqual(requestParams(ic), { branchId: '1', tags: [], top: 10, measure: 'day', count: 'ic', grouping: 'type', included: [], refits: false });
});

/** By type, over two daily reads: rifles far ahead of fighters in units. */
const BY_TYPE: EquipmentProducedReply = {
  branchId: '1', measure: 'total', count: 'units', decimals: 0, grouping: 'type', warnings: [],
  types: [{ key: 'fighter', name: 'Fighter', domain: 'air' }, { key: 'rifle', name: 'Rifle', domain: 'land' }],
  keys: [key({ key: 'fighter', name: 'Fighter' }), key({ key: 'rifle', name: 'Rifle' })],
  series: [
    { tag: 'SOV', firstHours: [8, 32], lastHours: [31, 55], breaks: [0], membership: null,
      values: [{ key: 0, values: c(0, [6, 6]) }, { key: 1, values: c(0, [1310, 1310]) }], total: c(0, [1316, 1316]) },
    { tag: 'ITA', firstHours: [8, 32], lastHours: [31, 55], breaks: [0], membership: null,
      values: [{ key: 0, values: c(0, [5, 5]) }, { key: 1, values: c(0, [0, 0], [1, 2]) }], total: c(0, [5], [1, 2]) },
  ],
  known: [{ firstHour: 8, lastHour: 55 }], reads: [8, 32], observations: [8],
};

test('until one is chosen, the type shown is the one the countries delivered most, read from the recording', () => {
  assert.equal(mostDelivered(producedModel(validateProducedReply(BY_TYPE))), 'rifle');
  // An unknown latest value is left out, never read as zero or as a partial sum.
  const fighters: EquipmentProducedReply = { ...BY_TYPE, series: [BY_TYPE.series[1]] };
  assert.equal(mostDelivered(producedModel(validateProducedReply(fighters))), 'fighter');
  assert.equal(mostDelivered(producedModel(validateProducedReply({ ...BY_TYPE, series: [] }))), null);
});

/** By exact variant: two rifles, a tank, and a gun content does not resolve. */
const variants = (gun: number[], unknownGun: number[] = []): EquipmentProducedReply => ({
  ...BY_TYPE, grouping: 'variant',
  keys: [key({ key: '11', typeKey: 'rifle', names: [{ firstHour: 0, name: 'PPSh-41', known: true, design: null }] }),
    key({ key: '12', typeKey: 'rifle', names: [{ firstHour: 0, name: 'Mosin', known: true, design: null }] }),
    key({ key: '21', typeKey: 'tank', names: [{ firstHour: 0, name: 'T-34 AM', known: true, design: null }] }),
    key({ key: '31', typeKey: ':mod_gun', names: [{ firstHour: 0, name: 'Mod gun', known: true, design: null }] })],
  series: [{ tag: 'SOV', firstHours: [8], lastHours: [31], breaks: [0], membership: null,
    values: [{ key: 0, values: c(0, [1300]) }, { key: 1, values: c(0, [10]) }, { key: 2, values: c(0, [10]) },
      { key: 3, values: c(0, gun, unknownGun) }], total: c(0, [1320 + (gun[0] ?? 0)]) }],
});

test('a type adds only its own models, and is unknown while a gun content does not type holds anything', () => {
  const plain = typeValues(producedModel(validateProducedReply(variants([0]))), 'SOV', 10)!;
  assert.deepEqual(plain.map(entry => [entry.type, entry.value, entry.models.map(model => model.value)]),
    [['rifle', 1310, [1300, 10]], ['tank', 10, [10]]]);
  // The unresolved gun could be a rifle or a tank: their sums are unknown, its own stays exact.
  const unsure = typeValues(producedModel(validateProducedReply(variants([4]))), 'SOV', 10)!;
  assert.deepEqual(unsure.map(entry => [entry.type, entry.value]), [[':mod_gun', 4], ['rifle', null], ['tank', null]]);
  const unknown = typeValues(producedModel(validateProducedReply(variants([], [0, 1]))), 'SOV', 10)!;
  assert.deepEqual(unknown.map(entry => [entry.type, entry.value]), [[':mod_gun', null], ['rifle', null], ['tank', null]]);
});

test('breakdown rows name each type with its models, units and IC, and only IC has a share of the whole', () => {
  const units = producedModel(validateProducedReply(variants([0])));
  const ic = producedModel(validateProducedReply({ ...variants([0]), count: 'ic', decimals: 5,
    series: [{ ...variants([0]).series[0], values: [{ key: 0, values: c(0, [73_000_000]) }, { key: 1, values: c(0, [705_000]) },
      { key: 2, values: c(0, [15_725_000]) }], total: c(0, [89_430_000]) }] }));
  const rows = typeRows(units, ic, 'SOV', 10, 89_430_000)!;
  assert.deepEqual(rows.map(row => [row.label, row.models, row.units, row.ic, row.share]), [
    ['Rifle', 'PPSh-41 1,300 · Mosin 10', 1310, 73_705_000, '82.4%'],
    ['Equipment type name unavailable', 'T-34 AM', 10, 15_725_000, '17.6%'],
  ]);
});

test('a count the recording lacks at the hour is not recorded, never zero, whichever count it is', () => {
  const later = (reply: EquipmentProducedReply): EquipmentProducedReply => ({ ...reply,
    series: reply.series.map(series => ({ ...series, firstHours: [32], lastHours: [55] })) });
  const icReply = (reply: EquipmentProducedReply): EquipmentProducedReply => ({ ...reply, count: 'ic', decimals: 5,
    series: [{ ...reply.series[0], values: [{ key: 0, values: c(0, [73_000_000]) }, { key: 1, values: c(0, [705_000]) }],
      total: c(0, [73_705_000]) }] });
  const units = producedModel(validateProducedReply(variants([0])));
  const ic = producedModel(validateProducedReply(icReply(variants([0]))));
  const unitsLater = producedModel(validateProducedReply(later(variants([0]))));
  const icLater = producedModel(validateProducedReply(icReply(later(variants([0])))));
  const shown = (rows: ReturnType<typeof typeRows>) => rows?.map(row => [row.type, row.models, row.units, row.recorded.units, row.ic, row.recorded.ic]);
  // Units read at the hour, IC not: every IC cell is unrecorded, not 0 IC.
  assert.deepEqual(shown(typeRows(units, icLater, 'SOV', 10, null)), [
    ['rifle', 'PPSh-41 1,300 · Mosin 10', 1310, true, null, false], ['tank', 'T-34 AM', 10, true, null, false]]);
  // IC read at the hour, units not: the rifle's models carry no unit counts, and no type delivered 0.
  assert.deepEqual(shown(typeRows(unitsLater, ic, 'SOV', 10, 73_705_000)), [
    ['rifle', 'PPSh-41 · Mosin', null, false, 73_705_000, true]]);
  // Neither read at the hour: nothing is claimed delivered.
  assert.equal(typeRows(unitsLater, icLater, 'SOV', 10, null), null);
  // Both read: a type absent from one count is a known zero there.
  assert.deepEqual(shown(typeRows(units, ic, 'SOV', 10, 73_705_000))?.find(row => row[0] === 'tank'), ['tank', 'T-34 AM', 10, true, 0, true]);
});
