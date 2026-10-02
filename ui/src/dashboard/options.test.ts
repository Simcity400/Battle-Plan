import test from 'node:test';
import assert from 'node:assert/strict';
import { contentKey, contentSet, MAX_OPTION_KEYS, MAX_SET_TOKENS, MAX_TOKEN, parseStoredOptions, resolveOptions, sectionFields, setLimit, setOption, setToken, storableToken,
  toggleContentSet } from './options.ts';
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
  // The longest stored text is a full content set of the longest tokens.
  const fullSet = Array.from({ length: MAX_SET_TOKENS }, (_, index) => `${index}`.padEnd(MAX_TOKEN, 'x')).join(' ');
  assert.deepEqual(parseStoredOptions({ picked: fullSet }), { picked: fullSet });
  for (const bad of [null, [], 'x', { 'bad key': 1 }, { __proto__x: 1, constructor: 1 }, { a: NaN }, { a: {} }, { a: `${fullSet}x` }]) {
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

test('a content set stores distinct tokens up to its limit and empties back to automatic', () => {
  const fields: readonly OptionField[] = [{ key: 'picked', section: 'options', label: 'Picked', kind: 'content-set', automatic: 'Automatic', max: 2, default: '' }];
  assert.deepEqual(resolveOptions(fields, { picked: 'steel mod_ore' }), { picked: 'steel mod_ore' });
  for (const bad of ['steel steel', 'a b c', 'steel  oil', ' steel', 'steel,oil', 3]) {
    assert.deepEqual(resolveOptions(fields, { picked: bad }), { picked: '' }, String(bad));
  }
  assert.deepEqual(contentSet('steel oil'), ['steel', 'oil']);
  assert.deepEqual(contentSet(''), []);
  assert.equal(toggleContentSet('', 'steel', 2), 'steel');
  assert.equal(toggleContentSet('steel', 'oil', 2), 'steel oil');
  assert.equal(toggleContentSet('steel oil', 'coal', 2), null);
  assert.equal(toggleContentSet('steel oil', 'steel', 2), 'oil');
  assert.equal(toggleContentSet('oil', 'oil', 2), '');
  assert.equal(toggleContentSet('', 'x'.repeat(161), 8), null);
});

test('a content set holds its whole limit of long tokens, and says which tokens it cannot hold', () => {
  const eight: readonly OptionField[] = [{ key: 'picked', section: 'options', label: 'Picked', kind: 'content-set', automatic: 'Automatic', max: 8, default: '' }];
  // Eight 20-character tokens are 167 characters with their separators.
  const tokens = Array.from({ length: 8 }, (_, index) => `modded_resource_${index}`.padEnd(20, 'x'));
  let stored = '';
  for (const token of tokens) {
    const next = toggleContentSet(stored, token, 8);
    assert.notEqual(next, null, token);
    stored = next!;
  }
  assert.equal(stored.length, 167);
  // It survives the generic bounds on load and the field's own check.
  assert.deepEqual(resolveOptions(eight, parseStoredOptions({ picked: stored })!), { picked: stored });
  assert.deepEqual(setOption(eight, undefined, 'picked', stored), { picked: stored });
  // The limit is the field's, and never past what any set stores.
  assert.equal(toggleContentSet(stored, 'one_more', 8), null);
  assert.equal(setLimit(8), 8);
  assert.equal(setLimit(40), MAX_SET_TOKENS);
  // The longest token fits; a longer or unusual one cannot be stored, and chips say so.
  assert.equal(storableToken('x'.repeat(MAX_TOKEN)), true);
  assert.equal(storableToken('x'.repeat(MAX_TOKEN + 1)), false);
  assert.equal(storableToken('two words'), false);
  assert.equal(storableToken(''), false);
  // A mod's script tokens may hold any non-space character, non-ASCII included.
  for (const token of ['mod_rés', 'КОМ_idea', 'mod-cat.v2:x', 'ideas@mod']) assert.equal(storableToken(token), true, token);
  assert.equal(storableToken('tab\there'), false);
  const category: readonly OptionField[] = [{ key: 'category', section: 'options', label: 'Category', kind: 'content-choice', automatic: 'First', default: '' }];
  assert.deepEqual(setOption(category, undefined, 'category', 'mod_catégorie'), { category: 'mod_catégorie' });
  assert.deepEqual(resolveOptions(category, parseStoredOptions({ category: 'mod_catégorie' })!), { category: 'mod_catégorie' });
});

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

test('any content key round-trips through a storable token, plain keys unchanged', () => {
  for (const key of ['infantry_equipment', 'mod.type-1', '?mod_gun', '?#77', 'a:b', 'Ärmel', 'x y', '🙂']) {
    const token = setToken(key);
    assert.ok(storableToken(token), `${key} → ${token}`);
    assert.equal(contentKey(token), key);
  }
  assert.equal(setToken('infantry_equipment'), 'infantry_equipment');
  assert.equal(setToken('?mod_gun'), ':3f:mod_gun');
});

test('stored content escapes accept only Unicode scalar values and invalid choices default safely', () => {
  const fields: readonly OptionField[] = [
    { key: 'included', section: 'options', label: 'Included', kind: 'content-set', automatic: 'All', max: 8, default: '' },
    { key: 'picked', section: 'options', label: 'Picked', kind: 'content-choice', automatic: 'Automatic', default: '' },
  ];
  for (const token of [':ffffff:', ':110000:', ':d800:', ':dfff:', 'valid:41::dabc:']) {
    const stored = parseStoredOptions({ included: token, picked: token, unrelated: 'kept' });
    assert.ok(stored);
    assert.deepEqual(resolveOptions(fields, stored), { included: '', picked: '' }, token);
    assert.equal(storableToken(token), false, token);
    assert.equal(setOption(fields, undefined, 'included', token), null);
    assert.equal(setOption(fields, undefined, 'picked', token), null);
    assert.doesNotThrow(() => contentKey(token));
  }
  for (const point of [0, 0xd7ff, 0xe000, 0xffff, 0x10000, 0x10ffff]) {
    const token = `:${point.toString(16)}:`;
    assert.deepEqual(resolveOptions(fields, parseStoredOptions({ included: token, picked: token })!), { included: token, picked: token });
    assert.equal(contentKey(token), String.fromCodePoint(point));
  }
  assert.equal(contentKey(':ffffff:'), ':ffffff:');
});
