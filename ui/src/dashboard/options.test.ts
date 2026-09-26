import test from 'node:test';
import assert from 'node:assert/strict';
import { MAX_OPTION_KEYS, parseStoredOptions, resolveOptions, sectionFields, setOption } from './options.ts';
import type { OptionField } from './options';
import { settingsSections, SETTINGS_SECTIONS } from './settingsSections.ts';
import type { WidgetMeta } from './definitions';
import { mergeViewPreferences, migrateWidget } from './model.ts';
import type { WidgetSpec } from './model';
import { registerWidgets } from './definitions.ts';

const FIELDS: readonly OptionField[] = [
  { key: 'ticks', section: 'options', label: 'Spend ticks', kind: 'toggle', default: true },
  { key: 'order', section: 'options', label: 'Order', kind: 'choice', default: 'game', choices: [{ value: 'game', label: 'Game order' }, { value: 'size', label: 'Size' }] },
  { key: 'window', section: 'time', label: 'Window', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365, visibleWhen: options => options.ticks === true },
  { key: 'yMax', section: 'axes', label: 'Y maximum', kind: 'number', min: 0, max: 1e9, step: 1, default: null },
];

test('stored options are bounded generically, so an uninstalled widget keeps its choices', () => {
  assert.deepEqual(parseStoredOptions({ a: true, b: 3, c: 'x', d: null }), { a: true, b: 3, c: 'x', d: null });
  for (const bad of [null, [], 'x', { 'bad key': 1 }, { __proto__x: 1, constructor: 1 }, { a: NaN }, { a: {} }, { a: 'x'.repeat(161) }]) {
    assert.equal(parseStoredOptions(bad), null, JSON.stringify(bad));
  }
  const many = Object.fromEntries(Array.from({ length: MAX_OPTION_KEYS + 1 }, (_, index) => [`k${index}`, 1]));
  assert.equal(parseStoredOptions(many), null);
});

test('illegal or missing values fall back to the field default', () => {
  assert.deepEqual(resolveOptions(FIELDS, undefined), { ticks: true, order: 'game', window: 365, yMax: null });
  assert.deepEqual(resolveOptions(FIELDS, { ticks: 'yes', order: 'unknown', window: 2, yMax: 50, other: 1 }),
    { ticks: true, order: 'game', window: 365, yMax: 50 });
});

test('setting an option keeps unknown keys, drops defaults and refuses illegal values', () => {
  const stored = setOption(FIELDS, { other: 'kept' }, 'order', 'size');
  assert.deepEqual(stored, { other: 'kept', order: 'size' });
  assert.deepEqual(setOption(FIELDS, stored!, 'order', 'game'), { other: 'kept' });
  assert.equal(setOption(FIELDS, stored!, 'order', 'nope'), null);
  assert.equal(setOption(FIELDS, stored!, 'missing', true), null);
  assert.deepEqual(setOption(FIELDS, undefined, 'yMax', 10), { yMax: 10 });
  assert.deepEqual(setOption(FIELDS, { yMax: 10 }, 'yMax', null), {});
});

test('fields show only in their own section and when their condition holds', () => {
  assert.deepEqual(sectionFields(FIELDS, 'options', resolveOptions(FIELDS)).map(field => field.key), ['ticks', 'order']);
  assert.deepEqual(sectionFields(FIELDS, 'time', { ticks: true }).map(field => field.key), ['window']);
  assert.deepEqual(sectionFields(FIELDS, 'time', { ticks: false }), []);
});

const META: WidgetMeta = { kind: 'test-options', version: 2, title: 'Test', icon: 'grid', timeModes: ['follow', 'campaign'], defaultTime: 'campaign',
  scope: 'history', data: true, history: true, minSize: { w: 2, h: 2 }, defaultSize: { w: 4, h: 4 }, options: FIELDS,
  migrate: (from, widget) => from === 1 ? { ...widget, options: { order: 'size' } } : null };
registerWidgets([META]);

test('settings sections keep one fixed order and appear only when they apply', () => {
  const all = settingsSections(META, resolveOptions(FIELDS), true);
  assert.deepEqual(all, ['campaign', 'countries', 'time', 'size', 'options', 'axes', 'reset']);
  assert.deepEqual(all, SETTINGS_SECTIONS.filter(section => all.includes(section)));
  const plain = settingsSections({ ...META, data: false, history: false, options: undefined }, {}, false);
  assert.deepEqual(plain, ['size']);
});

function spec(extra: Partial<WidgetSpec> = {}): WidgetSpec {
  return { id: 'w', kind: 'test-options', configVersion: 2, title: 'T', campaign: 'default', trackIds: ['main'],
    countries: { kind: 'selected' }, time: { mode: 'campaign' }, contentScale: 100, ...extra };
}

test('older settings migrate through the definition; a declined migration keeps them unchanged', () => {
  assert.deepEqual(migrateWidget(spec({ configVersion: 1 })), spec({ options: { order: 'size' } }));
  assert.deepEqual(migrateWidget(spec({ configVersion: 0 })), spec({ configVersion: 0 }));
  assert.deepEqual(migrateWidget(spec()), spec());
});

test('view preferences save settings but leave unknown campaigns and pins as drafts', () => {
  const saved = spec();
  const tracks = new Set(['main']);
  const next = spec({ title: 'New', contentScale: 125, countries: { kind: 'tags', tags: ['ENG'] }, options: { ticks: false }, time: { mode: 'follow' } });
  assert.deepEqual(mergeViewPreferences(saved, saved, next, tracks), next);
  const elsewhere = mergeViewPreferences(saved, saved, spec({ campaign: 'tracks', trackIds: ['other'], time: { mode: 'pin', windows: { other: { firstOffset: 0, lastOffset: 5 } } } }), tracks);
  assert.deepEqual(elsewhere.trackIds, ['main']);
  assert.equal(elsewhere.campaign, 'default');
  assert.deepEqual(elsewhere.time, { mode: 'campaign' });
  assert.equal('options' in mergeViewPreferences(saved, saved, spec(), tracks), false);
});
