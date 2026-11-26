import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { resolveTableRows } from '../../families/table/model.ts';
import { battalionCount, priorityLabel, templateRows, validateDivisionTemplates } from './divisionTemplateData.ts';
import { divisionTemplateProjection } from './divisionTemplateProjection.ts';
import type { DivisionTemplatesReply, TemplateEntry } from '../../generated/dashboard';

const entry = (template: string, extra: Partial<TemplateEntry> = {}): TemplateEntry => ({ template, name: `Template ${template}`, nameMissing: null, icon: null,
  iconMissing: 'no artwork', rosterOrdinal: Number(template), fielded: 0, units: [], unitsMissing: null, baseWidthRaw: '1800000', priority: 2, priorityName: null, priorityMissing: 'no proved name', locked: false,
  obsolete: false, obsoleteHour: null, designer: 'ordinary', forceRecruiting: false, newEquipment: true, foreignEquipment: false, manpower: 10_000,
  trainingDays: 90, settingsMissing: null, ...extra });
const reply = (templates: TemplateEntry[]): DivisionTemplatesReply => validateDivisionTemplates({ tag: 'GER', countFor: 'owner',
  templates, fieldedMissing: null, warnings: [] });

test('obsolete templates gather in one closed group after the current roster, in roster order', () => {
  const rows = templateRows(reply([entry('1'), entry('2', { obsolete: true, obsoleteHour: 59_000_000 }), entry('3'), entry('4', { obsolete: true })]));
  assert.deepEqual(rows.map(row => row.key), ['1', '3', 'obsolete']);
  const group = rows[2];
  assert.ok(group.kind === 'obsolete' && group.count === 2 && group.children.map(row => row.key).join() === '2,4');
  assert.deepEqual(templateRows(reply([entry('1')])).map(row => row.key), ['1'], 'no empty group');
});

test('an unrecorded composition has no battalion count and sorts after recorded ones either way', () => {
  const line = (count: number) => [{ key: 'line', name: 'Infantry', nameMissing: null, count, support: false, local: false }];
  const entries = [entry('1', { units: line(9) }), entry('2', { units: [], unitsMissing: 'not read' }), entry('3'), entry('4', { units: line(2) })];
  assert.equal(battalionCount(entries[1]), null);
  assert.equal(battalionCount(entries[2]), 0, 'a recorded empty template has none');
  const model = divisionTemplateProjection.project(
    createBoundedReadView({ hour: 50, panels: new Map([['main', reply(entries)]]) }),
    { countryName: 'Germany', canOpenDesigner: false },
  );
  const settings = (direction: 'ascending' | 'descending') => ({
    density: 'normal' as const,
    showRank: false,
    order: model.columns.map(column => column.key),
    columns: {},
    sort: { column: 'battalions', direction },
  });
  for (const direction of ['ascending', 'descending'] as const)
    assert.equal(resolveTableRows(model, settings(direction)).at(-1)?.key, '2', direction);
  assert.deepEqual(resolveTableRows(model, settings('ascending')).map(row => row.key), ['3', '4', '1', '2']);
});

test('a stored priority shows only as the game’s name for it, never as its number', () => {
  assert.equal(priorityLabel(entry('1')), null);
  assert.equal(priorityLabel(entry('1', { priorityName: 'Reserve' })), 'Reserve');
});

test('invalid replies are refused', () => {
  assert.throws(() => validateDivisionTemplates({ tag: 'GER', countFor: 'both', templates: [], fieldedMissing: null, warnings: [] }));
});

test('missing template name projects as notRecorded with nameMissing instead of a known placeholder', () => {
  const rep = reply([entry('1', { name: null, nameMissing: 'Localisation missing for template 1' })]);
  const model = divisionTemplateProjection.project(
    createBoundedReadView({ hour: 50, panels: new Map([['main', rep]]) }),
    { countryName: 'Germany', canOpenDesigner: false },
  );
  assert.deepEqual(model.rows[0]!.cells.template!.value, { kind: 'notRecorded', reason: 'Localisation missing for template 1' });
});

test('unread template settings separate unknown obsolete status from current count, withhold designer, and never wrap unnamed units as known', () => {
  const unnamedUnits = [
    { key: 'a', name: null, nameMissing: 'Unit type not localized', count: 2, support: false, local: false },
  ];
  const mixedUnits = [
    { key: 'inf', name: 'Infantry', nameMissing: null, count: 6, support: false, local: false },
    { key: 'unk', name: null, nameMissing: 'Support name missing', count: 1, support: true, local: false },
  ];
  const rep = reply([
    entry('1', { obsolete: false, designer: 'ordinary', units: unnamedUnits }),
    entry('2', { obsolete: null, designer: null, settingsMissing: 'Template settings table unread', units: mixedUnits }),
  ]);
  const model = divisionTemplateProjection.project(
    createBoundedReadView({ hour: 50, panels: new Map([['main', rep]]) }),
    { countryName: 'Germany', canOpenDesigner: true },
  );
  assert.deepEqual(model.rows.map(r => r.key), ['1', 'status-unknown']);
  assert.deepEqual(model.rows[0]!.cells.battalions!.value, { kind: 'notRecorded', reason: 'Unit type not localized' });
  const unknownChild = model.rows[1]!.children![0]!;
  assert.deepEqual(unknownChild.cells.battalions!.value, { kind: 'partial', value: 'Infantry \u00d76', reason: 'Support name missing' });
  assert.equal((unknownChild.meta as { designer: unknown }).designer, null);
  assert.equal(unknownChild.tooltipAction, undefined);
  assert.equal(unknownChild.tooltipNote, 'Template settings table unread');
  const extra = typeof unknownChild.tooltipExtra === 'function' ? unknownChild.tooltipExtra() : unknownChild.tooltipExtra;
  assert.equal(extra?.some(row => row.value.kind === 'notRecorded' && row.value.reason === 'Support name missing'), true);
});
