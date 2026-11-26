import test from 'node:test';
import assert from 'node:assert/strict';
import { choiceVariants, contentKey, contentSet, MAX_OPTION_KEYS, parseStoredOptions, resolveOptions, sectionFields, setLimit, setOption, setToken, storableToken,
  toggleContentSet, UNSTORABLE } from './options.ts';
import type { OptionField } from './options';
import { settingsSections, SETTINGS_SECTIONS } from './settingsSections.ts';
import type { WidgetMeta } from './definitions';
import { mergeViewPreferences } from './model.ts';
import type { WidgetSpec } from './model';
import { registerWidgets, widgetUnavailable } from './definitions.ts';

const FIELDS: readonly OptionField[] = [
  { key: 'ticks', section: 'options', label: 'Spend ticks', kind: 'toggle', default: true },
  { key: 'order', section: 'options', label: 'Order', kind: 'choice', default: 'game', choices: [{ value: 'game', label: 'Game order' }, { value: 'size', label: 'Size' }] },
  { key: 'window', section: 'time', label: 'Window', kind: 'number', min: 7, max: 3650, step: 1, unit: 'days', default: 365, visibleWhen: options => options.ticks === true },
  { key: 'yMax', section: 'axes', label: 'Y maximum', kind: 'number', min: 0, max: 1e9, step: 1, default: null },
];

test('stored options validate shape while keeping every choice for an uninstalled widget', () => {
  assert.deepEqual(parseStoredOptions({ a: true, b: 3, c: 'x', d: null }), { a: true, b: 3, c: 'x', d: null });
  const fullSet = Array.from({ length: 20 }, (_, index) => `${index}`.padEnd(200, 'x')).join(' ');
  assert.deepEqual(parseStoredOptions({ picked: fullSet }), { picked: fullSet });
  for (const bad of [null, [], 'x', { 'bad key': 1 }, { __proto__x: 1, constructor: 1 }, { a: NaN }, { a: Infinity }, { a: {} }]) {
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

test('scalar content choices round-trip opaque keys without the content-set delimiter rules', () => {
  const fields: readonly OptionField[] = [{ key: 'category', section: 'options', label: 'Category', kind: 'content-choice', automatic: 'First', default: '' }];
  const key = 'mod council,elite:20:seat';
  const selected = setOption(fields, undefined, 'category', key);
  assert.deepEqual(selected, { category: key });
  assert.deepEqual(resolveOptions(fields, parseStoredOptions(JSON.parse(JSON.stringify(selected)))!), { category: key });
  assert.deepEqual(setOption(fields, selected!, 'category', ''), {}, 'empty still means automatic');
  assert.equal(setOption(fields, undefined, 'category', 3), null);
  assert.equal(storableToken(key), false, 'content sets keep their distinct token rules');
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

test('a field that does nothing in a time mode is hidden in that mode, and its section with it', () => {
  const departed: OptionField = { key: 'departed', section: 'axes', label: 'Include departed', kind: 'toggle', default: false,
    visibleWhen: (_options, _countries, time) => time === 'campaign' };
  const meta: WidgetMeta = { ...META, options: [departed] };
  assert.deepEqual(sectionFields([departed], 'axes', {}, undefined, 'campaign').map(field => field.key), ['departed']);
  assert.deepEqual(sectionFields([departed], 'axes', {}, undefined, 'follow'), []);
  assert.equal(settingsSections(meta, {}, undefined, 'campaign').includes('axes'), true);
  assert.equal(settingsSections(meta, {}, undefined, 'pin').includes('axes'), false);
});

const META: WidgetMeta = { kind: 'test-options', version: 2, title: 'Test', icon: 'grid', timeModes: ['follow', 'campaign'], defaultTime: 'campaign',
  scope: 'history', data: true, history: true, minSize: { w: 2, h: 2 }, defaultSize: { w: 4, h: 4 }, options: FIELDS };
registerWidgets([META]);

test('a content set stores distinct tokens and empties back to automatic', () => {
  assert.equal(UNSTORABLE, 'This choice can’t be saved.');
  const fields: readonly OptionField[] = [{ key: 'picked', section: 'options', label: 'Picked', kind: 'content-set', automatic: 'Automatic', default: '' }];
  assert.deepEqual(resolveOptions(fields, { picked: 'steel mod_ore' }), { picked: 'steel mod_ore' });
  for (const bad of ['steel steel', 'steel  oil', ' steel', 'steel,oil', 3]) {
    assert.deepEqual(resolveOptions(fields, { picked: bad }), { picked: '' }, String(bad));
  }
  assert.deepEqual(contentSet('steel oil'), ['steel', 'oil']);
  assert.deepEqual(contentSet(''), []);
  assert.equal(toggleContentSet('', 'steel', Number.MAX_SAFE_INTEGER), 'steel');
  assert.equal(toggleContentSet('steel', 'oil', Number.MAX_SAFE_INTEGER), 'steel oil');
  assert.equal(toggleContentSet('steel oil', 'coal', Number.MAX_SAFE_INTEGER), 'steel oil coal');
  assert.equal(toggleContentSet('steel oil', 'steel', Number.MAX_SAFE_INTEGER), 'oil');
  assert.equal(toggleContentSet('oil', 'oil', Number.MAX_SAFE_INTEGER), '');
  assert.equal(toggleContentSet('', 'two words', Number.MAX_SAFE_INTEGER), null);
});

test('a content set accepts more than sixteen long tokens through choosing, storage and validation', () => {
  const fields: readonly OptionField[] = [{ key: 'picked', section: 'options', label: 'Picked', kind: 'content-set', automatic: 'Automatic', default: '' }];
  const tokens = Array.from({ length: 20 }, (_, index) => `modded_resource_${index}`.padEnd(200, 'x'));
  let stored = '';
  for (const token of tokens) {
    const next = toggleContentSet(stored, token, Number.MAX_SAFE_INTEGER);
    assert.notEqual(next, null, token);
    stored = next!;
  }
  assert.deepEqual(contentSet(stored), tokens);
  assert.deepEqual(resolveOptions(fields, parseStoredOptions({ picked: stored })!), { picked: stored });
  assert.deepEqual(setOption(fields, undefined, 'picked', stored), { picked: stored });
  assert.equal(toggleContentSet(stored, 'one_more', Number.MAX_SAFE_INTEGER), `${stored} one_more`);
  assert.equal(setLimit(8), 8);
  assert.equal(setLimit(40), 40);
  assert.equal(toggleContentSet(stored, 'one_more', 20), null);
  assert.equal(toggleContentSet(stored, 'one_more', 40), `${stored} one_more`);
  assert.equal(storableToken('x'.repeat(300)), true);
  assert.equal(storableToken('two words'), false);
  assert.equal(storableToken(''), false);
  for (const token of ['mod_rés', 'КОМ_idea', 'mod-cat.v2:x', 'ideas@mod']) assert.equal(storableToken(token), true, token);
  assert.equal(storableToken('tab\there'), false);
  const category: readonly OptionField[] = [{ key: 'category', section: 'options', label: 'Category', kind: 'content-choice', automatic: 'First', default: '' }];
  assert.deepEqual(setOption(category, undefined, 'category', 'mod_catégorie'), { category: 'mod_catégorie' });
  assert.deepEqual(resolveOptions(category, parseStoredOptions({ category: 'mod_catégorie' })!), { category: 'mod_catégorie' });
});

test('settings sections keep one fixed order and appear only when they apply', () => {
  const all = settingsSections(META, resolveOptions(FIELDS));
  assert.deepEqual(all, ['campaign', 'countries', 'time', 'size', 'options', 'axes']);
  assert.deepEqual(all, SETTINGS_SECTIONS.filter(section => all.includes(section)));
  const plain = settingsSections({ ...META, data: false, history: false, options: undefined }, {});
  assert.deepEqual(plain, ['size']);
  const world = settingsSections({ ...META, countrySelection: false }, resolveOptions(FIELDS));
  assert.deepEqual(world, ['campaign', 'time', 'size', 'options', 'axes']);
});

function spec(extra: Partial<WidgetSpec> = {}): WidgetSpec {
  return { id: 'w', kind: 'test-options', configVersion: 2, title: 'T', campaign: 'default', trackIds: ['main'],
    countries: { kind: 'selected' }, time: { mode: 'campaign' }, contentScale: 100, ...extra };
}

test('unknown or removed widget options fall back to defaults, older versions load and newer versions stay unsupported', () => {
  const stored = parseStoredOptions({ breakdownOpens: 'drawer', order: 'removed', ticks: false, window: 30 });
  assert.ok(stored);
  assert.deepEqual(resolveOptions(FIELDS, stored), { order: 'game', ticks: false, window: 30, yMax: null });
  assert.equal('breakdownOpens' in resolveOptions(FIELDS, stored), false);
  assert.equal(widgetUnavailable(spec({ configVersion: 1 })), null);
  assert.equal(widgetUnavailable(spec()), null);
  assert.match(widgetUnavailable(spec({ configVersion: 3 }))!, /version 3 is unsupported/);
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

test('any content key round-trips through a storable token, plain keys unchanged', () => {
  for (const key of ['infantry_equipment', 'mod.type-1', '?mod_gun', '?#77', 'a:b', 'Ärmel', 'x y', '🙂', '長'.repeat(200)]) {
    const token = setToken(key);
    assert.ok(storableToken(token), `${key} → ${token}`);
    assert.equal(contentKey(token), key);
  }
  assert.equal(setToken('infantry_equipment'), 'infantry_equipment');
  assert.equal(setToken('?mod_gun'), ':3f:mod_gun');
});

test('content-set escapes require Unicode scalars while scalar choices preserve raw text', () => {
  const fields: readonly OptionField[] = [
    { key: 'included', section: 'options', label: 'Included', kind: 'content-set', automatic: 'All', default: '' },
    { key: 'picked', section: 'options', label: 'Picked', kind: 'content-choice', automatic: 'Automatic', default: '' },
  ];
  for (const token of [':ffffff:', ':110000:', ':d800:', ':dfff:', 'valid:41::dabc:']) {
    const stored = parseStoredOptions({ included: token, picked: token, unrelated: 'kept' });
    assert.ok(stored);
    assert.deepEqual(resolveOptions(fields, stored), { included: '', picked: token }, token);
    assert.equal(storableToken(token), false, token);
    assert.equal(setOption(fields, undefined, 'included', token), null);
    assert.deepEqual(setOption(fields, undefined, 'picked', token), { picked: token });
    const roundtrip = parseStoredOptions(JSON.parse(JSON.stringify({ picked: token })));
    assert.ok(roundtrip);
    assert.equal(resolveOptions(fields, roundtrip).picked, token);
    assert.doesNotThrow(() => contentKey(token));
  }
  for (const point of [0, 0xd7ff, 0xe000, 0xffff, 0x10000, 0x10ffff]) {
    const token = `:${point.toString(16)}:`;
    assert.deepEqual(resolveOptions(fields, parseStoredOptions({ included: token, picked: token })!), { included: token, picked: token });
    assert.equal(contentKey(token), String.fromCodePoint(point));
  }
  assert.equal(contentKey(':ffffff:'), ':ffffff:');
});

test('a choice lists its other values as catalogue variants, each opening the widget with that value', () => {
  const view: OptionField = { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'table',
    choices: [{ value: 'table', label: 'Table' }, { value: 'race', label: 'Race' }, { value: 'map', label: 'Map' }] };
  assert.deepEqual(choiceVariants(view, choice => `Score ${choice.label.toLowerCase()}`), [
    { key: 'view=race', initial: { title: 'Score race', options: { view: 'race' } } },
    { key: 'view=map', initial: { title: 'Score map', options: { view: 'map' } } }], 'every value but the default');
  assert.deepEqual(choiceVariants(view, () => 'Score map', ['map']).map(item => item.key), ['view=map'], 'or only the values named');
  assert.deepEqual(choiceVariants(view, choice => `Score ${choice.label.toLowerCase()}`, undefined, choice => (choice.value === 'map' ? 'Overview' : undefined)), [
    { key: 'view=race', initial: { title: 'Score race', options: { view: 'race' } } },
    { key: 'view=map', initial: { title: 'Score map', options: { view: 'map' } }, category: 'Overview' }], 'a variant can declare its own category');
  assert.deepEqual(choiceVariants(undefined, () => 'None'), []);
  assert.deepEqual(choiceVariants({ key: 'grow', section: 'axes', label: 'Grow', kind: 'toggle', default: false }, () => 'Grow'), [], 'only choices have views');
});
