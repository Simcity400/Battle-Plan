import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { knownValue, notRecorded, partialValue, zeroValue } from '../../families/value.ts';
import type { DivisionEquipmentReply, EquipmentTemplate, EquipmentType } from '../../generated/dashboard';
import { equipmentTemplateProjection, equipmentTypeProjection } from './divisionEquipmentProjection.ts';

const type = (archetype: string | null, extra: Partial<EquipmentType> = {}): EquipmentType => ({
  key: archetype ? '17' : 'unclassified', unclassified: archetype === null, archetype, name: archetype ? 'Infantry Equipment' : null, nameMissing: null, heldRaw: '26514100000', neededRaw: '26937000000',
  filledRaw: '26514100000', missingRaw: '422900000', divisionsShort: 102, unknownDivisions: 0, stockpileRaw: null, stockpileHour: null, variants: [], variantsMissing: null, ...extra,
});

const template = (extra: Partial<EquipmentTemplate> = {}): EquipmentTemplate => ({
  template: '7', name: 'Infantry Division', nameMissing: null, icon: null, iconMissing: 'Template icon not recorded', divisions: 24,
  manpower: '240000', manpowerRequired: '256000', hpRaw: null, hpMaxRaw: null, neededRaw: '100000000', heldRaw: '50000000', typesShort: 3, unknownDivisions: 0, ...extra,
});

const reply = (extra: Partial<DivisionEquipmentReply> = {}): DivisionEquipmentReply => ({
  tag: 'GER', countFor: 'owner', missing: null, poolHour: 40, divisions: 60, divisionsShort: 213, unknownDivisions: 0,
  types: [type('infantry_equipment'), type(null, { missingRaw: '0', divisionsShort: 0, filledRaw: null })], templates: [template()],
  warnings: [], ...extra,
});

const project = (view: 'equipment' | 'template', raw: unknown) =>
  (view === 'template' ? equipmentTemplateProjection : equipmentTypeProjection).project(createBoundedReadView({ hour: 48, panels: new Map([['main', raw]]) }), {});

test('by equipment: whole items from the product, the shortfall as a red negative, fill missing when the product has none', () => {
  const model = project('equipment', reply());
  const [infantry, unclassified] = model.rows;
  assert.deepEqual([infantry!.cells.held!.value, infantry!.cells.missing!.value, infantry!.cells.missing!.shortage], [knownValue(265141), knownValue(-4229), true]);
  assert.deepEqual(infantry!.cells.fill!.value, knownValue(98.43));
  assert.deepEqual([unclassified!.cells.type!.value, unclassified!.cells.missing!.value], [knownValue('Unclassified equipment'), zeroValue(0)]);
  assert.equal(unclassified!.cells.fill!.value.kind, 'notRecorded');
  assert.equal(model.titleSummary, '213 divisions short');
});

test('two types whose spelling was not read keep their own rows and say so; only equipment with no type is unclassified', () => {
  const unread = 'Equipment names unavailable: Lexical tokens not read at this hour';
  const model = project('equipment', reply({ types: [
    type(null, { key: '31', unclassified: false, nameMissing: unread }),
    type(null, { key: '32', unclassified: false, nameMissing: unread }),
    type(null),
  ] }));
  assert.deepEqual(model.rows.map(row => row.key), ['31', '32', 'unclassified']);
  assert.deepEqual(model.rows.map(row => row.cells.type!.value), [notRecorded(unread), notRecorded(unread), knownValue('Unclassified equipment')]);
});

test('by template: shares of the product’s own figures; a figure it does not hold is missing; unrecorded equipment is the body state', () => {
  const [row] = project('template', reply()).rows;
  assert.deepEqual([row!.cells.manpower!.value, row!.cells.fill!.value], [knownValue(93.75), knownValue(50)]);
  assert.deepEqual(row!.cells.strength!.value, notRecorded('Strength not recorded'));
  const missing = project('equipment', reply({ missing: 'Division equipment is not recorded at this hour', types: [], templates: [] }));
  assert.deepEqual([missing.emptyKind, missing.empty], ['unavailable', 'Division equipment is not recorded at this hour']);
});

test('a type’s picture is the one the stockpile artwork product answers for its archetype; no answer leaves the slot without art', () => {
  const pictures = { tag: 'GER', entries: [{ key: 'type:Infantry_Equipment', variant: '9', name: null, icon: 'asset://infantry.png', reason: null }], warnings: [] };
  const model = equipmentTypeProjection.project(
    createBoundedReadView({ hour: 48, panels: new Map<string, unknown>([['main', reply()], ['pictures', pictures]]) }),
    {},
  );
  const [infantry, unclassified] = model.rows;
  const picture = infantry!.cells.picture!;
  assert.ok(picture.icon === 'asset://infantry.png' || picture.iconPending === true, JSON.stringify(picture));
  assert.equal(picture.artShape, 'equipment');
  assert.deepEqual([unclassified!.cells.picture!.icon, unclassified!.cells.picture!.iconPending], [undefined, undefined]);
});

test('a figure summed over the compared divisions is partial, and unranked, while some of the row’s divisions are not; complete readings stay exact', () => {
  const [row] = project('template', reply({ unknownDivisions: 1, templates: [template({ heldRaw: '100000000', unknownDivisions: 1 })] })).rows;
  const one = '1 division not recorded';
  assert.deepEqual([row!.cells.fill!.value, row!.cells.fill!.sortValue], [partialValue(100, one), notRecorded(one)]);
  assert.deepEqual([row!.cells.short!.value, row!.cells.short!.sortValue], [partialValue(3, one), notRecorded(one)]);
  assert.deepEqual([row!.cells.divisions!.value, row!.cells.manpower!.value, row!.cells.manpower!.sortValue], [knownValue(24), knownValue(93.75), undefined]);
  assert.deepEqual(row!.cells.fill!.bar?.fraction, partialValue(1, one));

  const two = '2 divisions not recorded';
  const [infantry] = project('equipment', reply({ unknownDivisions: 2, types: [type('infantry_equipment', { unknownDivisions: 2 })] })).rows;
  assert.deepEqual(
    ['needed', 'missing', 'fill', 'short'].map(key => [infantry!.cells[key]!.value.kind, infantry!.cells[key]!.sortValue]),
    ['needed', 'missing', 'fill', 'short'].map(() => ['partial', notRecorded(two)]),
  );
  assert.deepEqual(infantry!.cells.missing!.value, partialValue(-4229, two));
  assert.deepEqual([infantry!.cells.held!.value, infantry!.cells.held!.sortValue], [knownValue(265141), undefined], 'what every read pool holds is complete on its own');

  const [unknown] = project('equipment', reply({ unknownDivisions: null, types: [type('infantry_equipment', { unknownDivisions: null })] })).rows;
  assert.deepEqual(unknown!.cells.needed!.value, partialValue(269370, 'Divisions not recorded'));
});

test('a type row keeps its holdings when its designs were not read; its tooltip says why the design list is missing', () => {
  const unread = 'Designs held were not recorded at this hour: designs unread';
  const model = project('equipment', reply({ types: [type('infantry_equipment', { variantsMissing: unread })] }));
  const [infantry] = model.rows;
  assert.deepEqual(infantry!.cells.held!.value, knownValue(265141));
  const tip = typeof infantry!.tooltipExtra === 'function' ? infantry!.tooltipExtra() : infantry!.tooltipExtra;
  assert.deepEqual(tip?.find(row => row.label === 'Designs')?.value, notRecorded(unread));
});
