import test from 'node:test';
import assert from 'node:assert/strict';
import { registerWidgets } from './definitions.ts';
import { defaultTemplate, mergeViewPreferences, stableJson } from './model.ts';
import type { WidgetSpec } from './model';
import { validOption } from './options.ts';
import type { OptionField } from './options';
import { parseTemplate } from './storage.ts';
import { selectedTags } from './countrySelection.ts';
import '../widgets/definitions.ts';

const spec = (extra: Partial<WidgetSpec> = {}): WidgetSpec => ({ id: 'w', kind: 'research', configVersion: 1, title: 'T', campaign: 'default',
  trackIds: ['main'], countries: { kind: 'selected' }, time: { mode: 'campaign' }, contentScale: 100, ...extra });

test('countries and time save only with the campaign they were chosen for', () => {
  const saved = spec();
  const other = mergeViewPreferences(saved, saved, spec({ campaign: 'tracks', trackIds: ['new'], countries: { kind: 'tags', tags: ['ENG'] },
    time: { mode: 'follow' }, title: 'Renamed' }), new Set(['main']));
  assert.deepEqual([other.campaign, other.trackIds, other.countries, other.time, other.title], ['default', ['main'], { kind: 'selected' }, { mode: 'campaign' }, 'Renamed']);
  const same = mergeViewPreferences(saved, saved, spec({ countries: { kind: 'tags', tags: ['ENG'] }, configVersion: 2 }), new Set(['main']));
  assert.deepEqual([same.countries, same.configVersion], [{ kind: 'tags', tags: ['ENG'] }, 2]);
});

test('a setting saves alone, leaving other unsaved edits to the widget as drafts', () => {
  const saved = spec({ title: 'Mine', countries: { kind: 'tags', tags: ['ENG'] }, options: { a: true } });
  // An unsaved Reset layout put the defaults back; then only the size changes.
  const reset = spec({ options: { b: 1 } });
  const merged = mergeViewPreferences(saved, reset, { ...reset, contentScale: 150 }, new Set(['main']));
  assert.deepEqual(merged, { ...saved, contentScale: 150 });
  // One option changing saves that option, keeping the saved others.
  const option = mergeViewPreferences(saved, reset, { ...reset, options: { b: 1, c: 'x' } }, new Set(['main']));
  assert.deepEqual(option.options, { a: true, c: 'x' });
});

test('layout comparison ignores the order fields were written in', () => {
  assert.equal(stableJson({ b: 1, a: { d: [2, { y: 1, x: 2 }], c: 3 } }), stableJson({ a: { c: 3, d: [2, { x: 2, y: 1 }] }, b: 1 }));
  assert.notEqual(stableJson([1, 2]), stableJson([2, 1]));
});

test('number options must land on their step', () => {
  const days: OptionField = { key: 'days', section: 'time', label: 'Days', kind: 'number', min: 7, max: 3650, step: 1, default: 365 };
  assert.equal(validOption(days, 30), true);
  assert.equal(validOption(days, 30.5), false);
  const tenths: OptionField = { key: 't', section: 'axes', label: 'T', kind: 'number', min: 0, max: 1, step: 0.1, default: null };
  assert.equal(validOption(tenths, 0.3), true);
  assert.equal(validOption(tenths, 0.35), false);
  // Auto (null) is legal only for a field whose default is Auto.
  assert.equal(validOption(tenths, null), true);
  assert.equal(validOption(days, null), false);
});

registerWidgets([
  { kind: 'migrate-throws', version: 2, title: 'Throws', icon: 'grid', timeModes: ['campaign'], defaultTime: 'campaign', scope: 'moment', data: true, history: false,
    minSize: { w: 2, h: 2 }, defaultSize: { w: 4, h: 4 }, migrate: () => { throw new Error('bad migration'); } },
  { kind: 'migrate-invalid', version: 2, title: 'Invalid', icon: 'grid', timeModes: ['campaign'], defaultTime: 'campaign', scope: 'moment', data: true, history: false,
    minSize: { w: 2, h: 2 }, defaultSize: { w: 4, h: 4 }, migrate: (_from, widget) => ({ ...widget, trackIds: ['nowhere'] }) },
]);

test('a migration that throws or returns bad settings keeps the stored widget', () => {
  const template = defaultTemplate();
  template.widgets[0] = { ...template.widgets[0], kind: 'migrate-throws', configVersion: 1 };
  template.widgets[1] = { ...template.widgets[1], kind: 'migrate-invalid', configVersion: 1 };
  const parsed = parseTemplate(template);
  assert.ok(parsed, 'the layout still loads');
  assert.equal(parsed.widgets[0].configVersion, 1);
  assert.deepEqual(parsed.widgets[1].trackIds, template.widgets[1].trackIds);
});

test('"all countries" is not mistaken for the dashboard country', () => {
  assert.deepEqual(selectedTags({ kind: 'all' }, undefined), []);
});
