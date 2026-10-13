import assert from 'node:assert/strict';
import test from 'node:test';
import { runAt } from '../../charts/series.ts';
import { bindCountryParts, countryPartsRemainder, exactPartAmount, exactPartShare } from '../../charts/race/countryPartsDrillData.ts';
import { countryRaceStack, forEachCountryRaceSegment } from '../../charts/race/countryRace.ts';
import { drillPartShare, drillPartsTotal, orderedDrillParts } from '../../charts/drill/parts.ts';
import { NumericReply } from '../../dashboard/numericReply.ts';
import { producedModel, producedRequest, storedValues, validateProducedReply } from './producedData.ts';
import { producedDrillData, producedDrillNumbers } from './producedDrill.ts';
import type { EquipmentProducedReply, ProducedKey } from '../../generated/dashboard';

const key = (token: string): ProducedKey => ({ key: token, name: token, typeKey: token, domain: null, names: [] });
const reply: EquipmentProducedReply = { branchId: 'branch', measure: 'total', count: 'units', decimals: 0, grouping: 'type', types: [], warnings: [],
  keys: [key('recorded type'), key('correction type'), key('unresolved type')],
  series: [{ tag: 'MOD', firstHours: [8, 32, 56, 104], lastHours: [31, 55, 79, 127], breaks: [0, 3], membership: null,
    values: [{ key: 0, values: { from: 0, deltas: [12], unknown: [] } },
      { key: 1, values: { from: 0, deltas: [-2], unknown: [] } },
      { key: 2, values: { from: 0, deltas: [], unknown: [0, 4] } }],
    total: { from: 0, deltas: [17, 0, 0, -17], unknown: [2, 3] } }],
  known: [{ firstHour: 8, lastHour: 55 }, { firstHour: 104, lastHour: 127 }], reads: [8, 32, 56, 104], observations: [8] };

test('a Rocket-unknown drill keeps combined inventory unknown and names the signed equipment subtotal separately', () => {
  const data: EquipmentProducedReply = { ...reply, keys: [reply.keys[0]], series: [{ ...reply.series[0],
    values: [{ key: 0, values: { from: 0, deltas: [], unknown: [0, 4] },
      equipmentValues: { from: 0, deltas: [-7], unknown: [] } }],
    total: { from: 0, deltas: [], unknown: [0, 4] } }] };
  const drill = producedDrillData(producedModel(validateProducedReply(data)));
  assert.deepEqual(drill.at('MOD', 40)?.values, [null]);
  assert.equal(drill.at('MOD', 40)?.total, null);
  assert.ok(drill.bands('MOD', { firstHour: 8, lastHour: 127 }).every(runs => runs.length === 0));
  const tooltip = [...drill.tooltip('MOD', 40, 'Recorded country', new Map())!][0];
  assert.deepEqual(tooltip.rows?.map(row => [row.label, row.value]), [
    ['recorded type', 'Not known'], ['recorded type — Equipment lines', '-7'], ['Running total', 'Not known'],
  ]);
});

test('retained produced numbers preserve type classification through label refresh and replace changed domains', () => {
  const store = new NumericReply<EquipmentProducedReply>();
  const initial: EquipmentProducedReply = { ...reply, types: [
    { key: 'recorded type', name: 'Recorded label', domain: 'land' },
    { key: 'unresolved type', name: null, domain: null },
  ] };
  const first = store.update('same-request', initial, producedDrillNumbers)!;
  assert.deepEqual(first.types.map(type => type.domain), ['land', null]);
  const renamed = { ...initial, types: initial.types.map(type => ({ ...type, name: 'Reloaded label' })) };
  assert.equal(store.update('same-request', renamed, producedDrillNumbers), first);
  const changed: EquipmentProducedReply = { ...renamed, types: [
    { ...renamed.types[0], domain: 'naval' }, renamed.types[1],
  ] };
  const next = store.update('same-request', changed, producedDrillNumbers)!;
  assert.notEqual(next, first);
  assert.equal(next.types[0].domain, 'naval');
});

test('produced full drill keeps corrections, independent residual, unknown totals, supplying reads and identity breaks', () => {
  const drill = producedDrillData(producedModel(validateProducedReply(reply)));
  assert.deepEqual(drill.at('MOD', 40)?.values, [12, -2, null]);
  assert.equal(countryPartsRemainder(drill.at('MOD', 40)), 7n);
  assert.equal(drill.readHour('MOD', 40), 32);
  assert.doesNotMatch(drill.at('MOD', 40)?.note ?? '', /Read /);
  assert.equal(drill.readHour('MOD', 90), null);
  const bands = drill.bands('MOD', { firstHour: 8, lastHour: 127 });
  assert.deepEqual(bands.map(runs => runAt(runs, 40)?.value), [12, -2, 0, 7]);
  assert.ok(bands.every(runs => runAt(runs, 60) == null && runAt(runs, 90) == null && runAt(runs, 104)?.breakBefore));
  assert.equal(drill.at('MOD', 60)?.total, null);
  assert.equal(drill.at('MOD', 104)?.total, 0);
  assert.deepEqual(drill.keyHours, [8, 32, 56, 104]);
  const tooltip = [...drill.tooltip('MOD', 40, 'Name at supplying read', new Map())!][0];
  assert.equal(tooltip.rows.find(row => row.label === 'correction type')?.value, '-2');
  assert.equal(tooltip.rows.find(row => row.label === 'unresolved type')?.value, 'Not known');
  assert.equal(tooltip.rows.find(row => row.label === 'Other')?.value, '7');
  assert.deepEqual(producedRequest('branch', { tags: ['MOD'] }, 'week', 'variant', ['mod type'], true),
    { branchId: 'branch', tags: ['MOD'], top: null, measure: 'week', count: 'units', grouping: 'variant', included: ['mod type'], refits: true });
});

test('exact amounts and residuals never round wide cancellation, missing totals or known zero into another value', () => {
  const maximum = Number.MAX_SAFE_INTEGER;
  const residual = countryPartsRemainder({ total: maximum, values: [-maximum, 5, null] });
  assert.equal(residual, 18_014_398_509_481_977n);
  assert.equal(exactPartAmount(residual!), '18,014,398,509,481,977');
  assert.equal(exactPartAmount(maximum), '9,007,199,254,740,991');
  assert.equal(countryPartsRemainder({ total: null, values: [5] }), null);
  assert.equal(drillPartShare(0, 0, true), '—');
  assert.equal(drillPartShare(0, null, true), '—');
  assert.equal(drillPartShare(null, 10, true), '—');
  assert.equal(drillPartShare(-2, 10, true), '-20%');
  assert.equal(drillPartsTotal([{ id: 'unknown', label: 'Unknown', value: null }]), null);
  const wide = countryPartsRemainder({ total: 1, values: [-maximum, -1] });
  assert.equal(wide, 9007199254740993n);
  assert.equal(exactPartShare(wide, 1), '900719925474099300%');
  assert.equal(exactPartShare(-2n, 10), '-20%');
  assert.equal(exactPartShare(0n, 0), '—');
  assert.equal(exactPartShare(12n, null), '—');
});

test('all produced designs remain accessible without expanding constant compact columns into sample arrays', () => {
  const count = 5000;
  const keys = Array.from({ length: count }, (_, index) => key(`mod design ${index}`));
  keys[0].names = [{ firstHour: 0, name: 'Earlier design', known: true, design: null }, { firstHour: 624, name: 'Later design', known: true, design: null }];
  const firstHours = Array.from({ length: 50 }, (_, index) => index * 24);
  const source: EquipmentProducedReply = { ...reply, keys, grouping: 'variant', reads: firstHours,
    series: [{ tag: 'MOD', firstHours, lastHours: firstHours.map(hour => hour + 23), breaks: [0], membership: null,
      values: keys.map((_, key) => ({ key, values: { from: 0, deltas: [1], unknown: [] } })),
      total: { from: 0, deltas: [count], unknown: [] } }] };
  const model = producedModel(validateProducedReply(source));
  const drill = producedDrillData(model);
  const at = drill.at('MOD', 600)!;
  const rows = orderedDrillParts(drill.parts(600).map((part, index) => ({ ...part, value: at.values[index] })));
  assert.equal(rows.length, count);
  assert.equal(new Set(rows.map(row => row.id)).size, count);
  assert.equal(storedValues(model), count + 1);
  assert.ok(drill.bands('MOD', { firstHour: 0, lastHour: 1199 }).every(runs => runs.length === 1));
  assert.equal(drill.parts(600)[0].label, 'Earlier design');
  assert.equal(drill.parts(624)[0].label, 'Later design');
});

test('production binding follows a peaceful original holder through rename and refuses tag reuse and gaps', () => {
  const source: EquipmentProducedReply = { ...reply, keys: [key('native design')], reads: [10, 11, 13], series: [
    { tag: 'OLD', firstHours: [13], lastHours: [20], breaks: [0], membership: null,
      identities: [{ firstHour: 10, lastHour: 10, slot: 3, countryId: 9 }, { firstHour: 13, lastHour: 20, slot: 8, countryId: 99 }],
      values: [{ key: 0, values: { from: 0, deltas: [99], unknown: [] } }], total: { from: 0, deltas: [99], unknown: [] } },
    { tag: 'NEW', firstHours: [11], lastHours: [20], breaks: [0], membership: null,
      identities: [{ firstHour: 11, lastHour: 20, slot: 3, countryId: 9 }],
      values: [{ key: 0, values: { from: 0, deltas: [12], unknown: [] } }], total: { from: 0, deltas: [12], unknown: [] } },
  ] };
  const native = producedDrillData(producedModel(validateProducedReply(source)));
  const binding = bindCountryParts(native, 'OLD', 10);
  assert.equal(binding.data.at('OLD', 10), null); // peaceful origin precedes the first production sample
  assert.equal(binding.tagAt(19), 'NEW');
  assert.deepEqual(binding.data.at('OLD', 19)?.values, [12]);
  assert.equal(binding.data.readHour('OLD', 19), 13);
  assert.equal(runAt(binding.data.bands('OLD', { firstHour: 10, lastHour: 25 })[0], 19)?.value, 12);
  assert.equal(binding.data.at('OLD', 21), null);
  assert.equal(bindCountryParts(native, 'OLD', 9).data.at('OLD', 19), null);
});

test('5000 constant compact designs plus one changing daily design stay compact through the real chart boundary', () => {
  const count = 5000; const days = 3650;
  const keys = Array.from({ length: count + 1 }, (_, index) => key(`native design ${index}`));
  const firstHours = Array.from({ length: days }, (_, day) => day * 24);
  const source: EquipmentProducedReply = { ...reply, keys, reads: firstHours,
    series: [{ tag: 'MOD', firstHours, lastHours: firstHours.map(hour => hour + 23), breaks: [0], membership: null,
      values: keys.map((_, key) => ({ key, values: { from: 0, deltas: key === count ? firstHours.map(() => 1) : [1], unknown: [] } })),
      total: { from: 0, deltas: firstHours.map((_, index) => index === 0 ? count + 1 : 1), unknown: [] } }] };
  const native = producedModel(validateProducedReply(source));
  const drill = producedDrillData(native);
  const bands = drill.bands('MOD', { firstHour: 0, lastHour: days * 24 - 1 });
  assert.equal(bands.reduce((sum, runs) => sum + runs.length, 0), count + days + 1);
  assert.equal(storedValues(native), count + days * 2);
  const stack = countryRaceStack(bands, true);
  assert.ok('bands' in stack);
  assert.equal('edges' in stack, false);
  assert.ok(stack.hours.length <= days * 2 + 1);
  let geometryPoints = 0;
  const pixels = 20;
  forEachCountryRaceSegment(stack, 0, days * 24 - 1, days * 24 / pixels, true, piece => {
    geometryPoints += piece.hours.length * piece.bands.length;
  });
  assert.ok(geometryPoints <= (pixels * 6 + 2) * (count + 2) * 3);
});
