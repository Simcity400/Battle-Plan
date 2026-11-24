import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import type { DivisionEquipmentReply, EquipmentTemplate, EquipmentType } from '../../generated/dashboard';
import { divisionEquipmentProjection } from './divisionEquipmentProjection.ts';

const type = (archetype: string | null, extra: Partial<EquipmentType> = {}): EquipmentType => ({
  archetype, name: archetype ? 'Infantry Equipment' : null, nameMissing: null, heldRaw: '26514100000', neededRaw: '26937000000',
  filledRaw: '26514100000', missingRaw: '422900000', divisionsShort: 102, unknownDivisions: 0, stockpileRaw: null, stockpileHour: null, variants: [], ...extra,
});

const template = (extra: Partial<EquipmentTemplate> = {}): EquipmentTemplate => ({
  template: '7', name: 'Infantry Division', nameMissing: null, icon: null, iconMissing: 'Template icon not recorded', divisions: 24,
  manpower: '240000', manpowerRequired: '256000', hpRaw: null, hpMaxRaw: null, neededRaw: '100000000', heldRaw: '50000000', typesShort: 3, unknownDivisions: 0, ...extra,
});

const reply = (extra: Partial<DivisionEquipmentReply> = {}): DivisionEquipmentReply => ({
  tag: 'GER', countFor: 'owner', missing: null, poolHour: 40, divisions: 60, divisionsShort: 213, unknownDivisions: 0,
  types: [type('infantry_equipment'), type(null, { missingRaw: '0', divisionsShort: 0, filledRaw: null })], templates: [template()],
  divisionRows: null, pool: null, warnings: [], ...extra,
} as DivisionEquipmentReply);

const project = (view: 'equipment' | 'template', raw: unknown) =>
  divisionEquipmentProjection.project(createBoundedReadView({ hour: 48, panels: new Map([['main', raw]]) }), { view });

test('by equipment: whole items from the product, the shortfall as a red negative, fill missing when the product has none', () => {
  const model = project('equipment', reply());
  const [infantry, unclassified] = model.rows;
  assert.deepEqual([infantry!.cells.held!.value, infantry!.cells.missing!.value, infantry!.cells.missing!.shortage], [knownValue(265141), knownValue(-4229), true]);
  assert.deepEqual(infantry!.cells.fill!.value, knownValue(98.43));
  assert.deepEqual([unclassified!.cells.type!.value, unclassified!.cells.missing!.value], [knownValue('Unclassified equipment'), zeroValue(0)]);
  assert.equal(unclassified!.cells.fill!.value.kind, 'notRecorded');
  assert.equal(model.titleSummary, '213 divisions short');
});

test('by template: shares of the product’s own figures; a figure it does not hold is missing; unrecorded equipment is the body state', () => {
  const [row] = project('template', reply()).rows;
  assert.deepEqual([row!.cells.manpower!.value, row!.cells.fill!.value], [knownValue(93.75), knownValue(50)]);
  assert.deepEqual(row!.cells.strength!.value, notRecorded('Strength not recorded'));
  const missing = project('equipment', reply({ missing: 'Division equipment is not recorded at this hour', types: [], templates: [] }));
  assert.deepEqual([missing.emptyKind, missing.empty], ['unavailable', 'Division equipment is not recorded at this hour']);
});
