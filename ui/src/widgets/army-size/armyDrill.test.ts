import { gatedTime, heldCountry } from '../../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { countryValuesModel } from '../../charts/race/countryValues.ts';
import { countryDrillSpans, countryDrillTagAt } from '../../dashboard/countryIdentity.ts';
import { NumericReply } from '../../dashboard/numericReply.ts';
import { createWidget } from '../../dashboard/model.ts';
import { resolveOptions, sectionFields } from '../../dashboard/options.ts';
import type { ArmySizeReply } from './armySizeData';
import '../definitions.ts';
import { armyDrillActors, armyDrillBands, armyDrillIdentityReply, armyDrillNumbers, armyDrillValue, pinArmyDrill } from './armyDrill.ts';
import { definition, normaliseArmyOptions, OPTIONS } from './definition.ts';
import { mergeArmyWindows, validateArmySizeReply } from './armySizeData.ts';

const reply: ArmySizeReply = { branchId: 'synthetic', window: 0, windows: [], interval: { firstHour: 100, lastHour: 105 },
  available: true, measures: ['battalions', 'strength'], warnings: [], types: [
    { key: 'group:mod', name: 'mod group' }, { key: 'unknown:orphan', name: 'orphan (unknown-group)' },
  ], series: [{ tag: 'A', segments: [{ firstHour: 100, lastHour: 105 }], membership: null,
    runs: [[100, Number('9007199254740995'), 105, '9007199254740995']],
    mix: [{ firstHour: 100, lastHour: 105, values: [Number('9007199254740993'), null],
      rawValues: ['9007199254740993', null] }] }] };

const OPAQUE = heldCountry({ tag: 'opaque country', origin: 101, slot: 1, countryId: 1 });
test('Army drill retains exact wide parts and the native independent residual', () => {
  assert.equal(validateArmySizeReply(reply), reply);
  const at = armyDrillValue(countryValuesModel(reply), 'A', 101, 'battalions')!;
  assert.equal(at.totalText, '9,007,199,254,740,995');
  assert.equal(at.parts[0].text, '9,007,199,254,740,993');
  assert.equal(at.residual, 2);
  assert.equal(at.residualText, '2');
  assert.deepEqual(at.missing, ['orphan (unknown-group)']);
  assert.equal(at.parts[1].muted, true);
  assert.throws(() => validateArmySizeReply({ ...reply, series: [{ ...reply.series[0], mix: [
    { ...reply.series[0].mix![0], rawValues: ['3', null] },
  ] }] }));
});

test('unsplit HP keeps its exact fixed5 total under Other; unknown totals remain unknown', () => {
  const raw = '9007199254740987';
  const hp: ArmySizeReply = { ...reply, series: [{ ...reply.series[0], runs: [[100, Number(raw) / 100000, 105, raw]] }] };
  const at = armyDrillValue(countryValuesModel(hp), 'A', 101, 'strength')!;
  assert.deepEqual(at.parts, []);
  assert.equal(at.totalText, '90,071,992,547.40987');
  assert.equal(at.residualText, at.totalText);
  assert.equal(at.residualShare, '100.00%');
  const unknown = { ...hp, series: [{ ...hp.series[0], runs: [[100, null, 105]] }] } as ArmySizeReply;
  assert.equal(armyDrillValue(countryValuesModel(unknown), 'A', 101, 'strength')?.residual, null);
});

test('Army drill history preserves scalar gaps and missing-composition residuals without hourly expansion', () => {
  const sparse: ArmySizeReply = { ...reply, types: [{ key: 'group:mod', name: 'mod group' }], series: [{
    tag: 'A', segments: [{ firstHour: 100, lastHour: 105 }], membership: null,
    runs: [[100, 6, 101, '6'], [104, 9, 105, '9']],
    mix: [{ firstHour: 100, lastHour: 100, values: [4], rawValues: ['4'] }],
  }] };
  const bands = armyDrillBands(sparse, 'A', 'battalions', { firstHour: 100, lastHour: 105 });
  assert.deepEqual(bands.map(runs => runs.map(run => [run.firstHour, run.lastHour, run.value])), [
    [[100, 100, 4], [101, 101, 0], [104, 105, 0]], [[100, 100, 2], [101, 101, 6], [104, 105, 9]],
  ]);
  assert.equal(bands[1][2].breakBefore, true);
});

test('Army native origin follows a rename and excludes a later country reusing the tag', () => {
  const identity = (firstHour: number, lastHour: number, slot: number, countryId: number) => ({ firstHour, lastHour, slot, countryId });
  const renamed: ArmySizeReply = { ...reply, types: [], series: [
    { tag: 'A', membership: null, segments: [{ firstHour: 100, lastHour: 102 }, { firstHour: 103, lastHour: 105 }],
      identities: [identity(100, 102, 1, 7), identity(103, 105, 2, 8)], runs: [[100, 4, 102, '4'], [103, 99, 105, '99']] },
    { tag: 'B', membership: null, segments: [{ firstHour: 103, lastHour: 105 }],
      identities: [identity(103, 105, 1, 7)], runs: [[103, 5, 105, '5']] },
  ] };
  const spans = countryDrillSpans(renamed.series, 'A', 100);
  assert.equal(countryDrillTagAt(spans, 100), 'A');
  assert.equal(countryDrillTagAt(spans, 104), 'B');
  assert.equal(countryDrillTagAt(spans, 106), null);
  const selected = armyDrillIdentityReply(renamed, 'A', 100);
  assert.equal(armyDrillValue(countryValuesModel(selected), 'A', 104, 'divisions')?.totalText, '5');
  assert.deepEqual(selected.series[0].runs, [[100, 4, 102, '4'], [103, 5, 105, '5']]);
  const ambiguous = [...renamed.series, { ...renamed.series[1], tag: 'C' }];
  assert.equal(countryDrillTagAt(countryDrillSpans(ambiguous, 'A', 100), 104), null);
});

test('Army window remapping retains exact part strings and native identity dates', () => {
  const identity = { firstHour: 100, lastHour: 105, slot: 1, countryId: 7 };
  const window = { ...reply, series: [{ ...reply.series[0], identities: [identity] }] };
  const joined = mergeArmyWindows({ ...reply, types: [], series: [], window: null, interval: null }, new Map([[0, window]]), [0]);
  assert.deepEqual(joined.series[0].mix?.[0].rawValues, ['9007199254740993', null]);
  assert.deepEqual(joined.series[0].identities, [identity]);
});

test('Army breakdown options and pin preserve the shown battalion measure and branch-relative date', () => {
  const widget = { ...createWidget('army-size', 'main', 'army'), options: { view: 'mix', measure: 'strength' } };
  const options = resolveOptions(OPTIONS, widget.options);
  assert.equal(options.drill, 'inline');
  assert.equal(options.view, 'mix');
  assert.equal(options.show, undefined);
  const breakdown = { ...options, view: 'breakdown' };
  assert.equal(sectionFields(OPTIONS, 'options', breakdown).some(field => field.key === 'measure'), true);
  assert.equal(sectionFields(OPTIONS, 'options', breakdown).some(field => field.key === 'view'), true);
  const pinned = pinArmyDrill(widget, { id: 'main', branch: { firstHour: 100 } } as Parameters<typeof pinArmyDrill>[1],
    OPAQUE, gatedTime({ mode: 'pin', windows: { main: { firstOffset: 23, lastOffset: 23 } } }), 'battalions');
  assert.deepEqual(pinned.countries, { kind: 'tags', tags: ['opaque country'] });
  assert.deepEqual(pinned.time.value, { mode: 'pin', windows: { main: { firstOffset: 23, lastOffset: 23 } } });
  assert.equal(pinned.options?.measure, 'battalions');
  assert.equal(pinned.options?.view, 'breakdown');
  assert.equal(pinned.options?.show, undefined);
  assert.equal(pinned.options?.drillOriginHour, 101);
});


test('standalone Army keeps all manual choices and a chosen identity through rank reversal', () => {
  const many: ArmySizeReply = { ...reply, types: [], series: Array.from({ length: 5_000 }, (_, index) => ({
    tag: `opaque ${index}`, membership: null, segments: [{ firstHour: 100, lastHour: 105 }],
    identities: [{ firstHour: 100, lastHour: 105, slot: index, countryId: index + 1 }],
    runs: [[100, index, 102, String(index)], [103, 5_000 - index, 105, String(5_000 - index)]],
  })) };
  const model = countryValuesModel(many);
  assert.equal(armyDrillActors(model, model.tags, 100).length, 5_000);
  const choices = armyDrillActors(model, model.tags, 100, 10);
  assert.equal(choices.length, 10);
  assert.equal(choices[0], 'opaque 4999');
  assert.equal(armyDrillActors(model, model.tags, 104, 10)[0], 'opaque 0');
  const origin = countryDrillSpans(many.series, choices[0], 100);
  assert.equal(countryDrillTagAt(origin, 104), 'opaque 4999');
  assert.equal(armyDrillValue(countryValuesModel(armyDrillIdentityReply(many, choices[0], 100)),
    choices[0], 104, 'divisions')?.totalText, '1');
});

test('Army drawn operands survive decoration and manifest refresh but preserve exact raw changes', () => {
  const decorated: ArmySizeReply = { ...reply, warnings: ['content refresh'],
    windows: [{ firstHour: 100, lastHour: 999 }], types: reply.types.map(type => ({ ...type, name: 'new name' })) };
  const retained = new NumericReply<ArmySizeReply>();
  const first = retained.update('same source / selection', reply, armyDrillNumbers);
  assert.equal(retained.update('same source / selection', undefined, armyDrillNumbers), first);
  assert.equal(retained.update('same source / selection', decorated, armyDrillNumbers), first);
  const changed = { ...decorated, series: [{ ...decorated.series[0], mix: [{
    ...decorated.series[0].mix![0], rawValues: ['9007199254740992', null],
  }] }] };
  assert.notEqual(retained.update('same source / selection', changed, armyDrillNumbers), first);
  assert.equal(retained.update('other native selection', undefined, armyDrillNumbers), undefined);
});

test('Pin as widget saves actual Follow after leaving a saved pin', () => {
  const widget = { ...createWidget('army-size', 'main', 'army'),
    time: { mode: 'pin' as const, windows: { main: { firstOffset: 2, lastOffset: 2 } } } };
  const pinned = pinArmyDrill(widget, { id: 'main', branch: { firstHour: 100 } } as Parameters<typeof pinArmyDrill>[1],
    OPAQUE, gatedTime({ mode: 'follow' }), 'strength');
  assert.deepEqual(pinned.time.value, { mode: 'follow' });
  assert.equal(pinned.options?.drillOriginHour, 101);
});


test('one army View preserves every saved chart/breakdown and only shows relevant settings', () => {
  for (const view of ['race', 'lines', 'mix']) for (const show of ['main', 'breakdown']) {
    const saved = { configVersion: 1, options: { show, view, measure: 'battalions', byType: true, dateOverlay: true, countFor: 'host' } } as unknown as Parameters<typeof definition.migrate>[1];
    const migrated = definition.migrate(1, saved);
    assert.equal(migrated.configVersion, 2);
    assert.deepEqual(migrated.options, { view: show === 'breakdown' ? 'breakdown' : view,
      measure: 'battalions', byType: true, dateOverlay: true, countFor: 'host' });
    assert.deepEqual(normaliseArmyOptions(migrated.options), migrated.options);
  }
  assert.deepEqual(normaliseArmyOptions(), { view: 'race' });
  assert.deepEqual(normaliseArmyOptions({ view: 'breakdown' }), { view: 'breakdown' });
  for (const view of ['race', 'lines', 'mix', 'breakdown']) {
    const options = resolveOptions(OPTIONS, { view, measure: 'battalions', byType: true, dateOverlay: true });
    const fields = sectionFields(OPTIONS, 'options', options).map(field => field.key);
    assert.equal(fields.includes('show'), false);
    assert.equal(fields.filter(key => key === 'view').length, 1);
    assert.equal(fields.includes('measure'), view !== 'mix');
    assert.equal(fields.includes('byType'), view === 'race');
    assert.equal(fields.includes('dateOverlay'), view === 'race');
    assert.equal(sectionFields(OPTIONS, 'interaction', options).some(field => field.key === 'drill'), view !== 'breakdown');
    assert.equal(options.byType, true, 'hidden battalion grouping remains saved');
    assert.equal(options.dateOverlay, true);
  }
});
