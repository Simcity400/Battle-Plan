import test from 'node:test';
import assert from 'node:assert/strict';
import { defaultTemplate, WIDGET_CONTENT_SCALES } from './model.ts';
import { layoutsAtDefault, layoutsUnsaved, loadLayouts, MAX_LAYOUTS, parseLayoutLibrary, parseLayoutLibraryWithNotes, parseTemplate, saveLayouts } from './storage.ts';
import '../widgets/definitions.ts';

function memoryStorage() {
  const values = new Map<string, string>();
  return { values, getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
}

test('a page is at its default until its saved layouts differ from the designed arrangement', () => {
  const storage = memoryStorage();
  assert.equal(layoutsAtDefault('overview', storage), true, 'nothing saved');
  const { library } = loadLayouts(storage, 'overview');
  saveLayouts(library, storage, 'overview');
  assert.equal(layoutsAtDefault('overview', storage), true, 'a saved copy of the default');
  const [first] = library.layouts;
  const widgets = first.template.widgets.map((widget, index) => index ? widget : { ...widget, title: 'Renamed' });
  assert.equal(saveLayouts({ ...library, layouts: [{ ...first, template: { ...first.template, widgets } }] }, storage, 'overview'), null);
  assert.equal(layoutsAtDefault('overview', storage), false, 'a renamed widget');
  assert.equal(layoutsAtDefault('politics', storage), true, 'each page is judged on its own');
  // Unreadable saved layouts open the default with a warning; a reset is what clears them.
  storage.values.set('hoi4-replay.workspace-layouts.v1.politics', '{not json');
  assert.notEqual(loadLayouts(storage, 'politics').warning, null);
  assert.equal(layoutsAtDefault('politics', storage), false, 'unreadable saved layouts');
});

test('a layout change not yet saved, on the shown page or one kept aside, counts as unsaved', () => {
  const { library } = loadLayouts(memoryStorage(), 'overview');
  const saved = library.layouts[0].template;
  const edited = { ...saved, widgets: saved.widgets.slice(1) };
  assert.equal(layoutsUnsaved([{ library, template: saved }]), false);
  assert.equal(layoutsUnsaved([{ library, template: edited }]), true, 'a widget removed in Edit without saving');
  assert.equal(layoutsUnsaved([{ library, template: saved }, { library, template: edited }]), true, 'another page kept its unsaved edit');
  assert.equal(layoutsUnsaved([{ library, template: { ...saved } }], template => ({ ...template, name: 'merged' })), false, 'merged the same way, equal');
});

test('saved Territory faction counts follow its uncapped definition without changing shared validation', () => {
  const template = defaultTemplate();
  const widget = template.widgets[0];
  widget.kind = 'territory';
  for (const count of [1, 30, 31, 64, 5_000, Number.MAX_SAFE_INTEGER]) {
    widget.countries = { kind: 'factions', count, membership: 'hour' };
    assert.equal(parseTemplate(template)?.widgets[0].countries.kind, 'factions');
  }
  for (const count of [Number.MAX_SAFE_INTEGER + 1, 0, 1.5]) {
    widget.countries = { kind: 'factions', count, membership: 'hour' };
    assert.equal(parseTemplate(template), null);
  }
});

test('widget content scales migrate, persist independently and reject invalid saved values', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  const widgets = library.layouts[0].template.widgets;
  delete widgets[0].contentScale;
  widgets[1].contentScale = 70;
  assert.equal(saveLayouts(library, storage), null);
  const restored = loadLayouts(storage).library.layouts[0].template.widgets;
  assert.equal(restored[0].contentScale, 100);
  assert.equal(restored[1].contentScale, 70);
  assert.deepEqual(restored[1].time, widgets[1].time);
  for (const scale of WIDGET_CONTENT_SCALES) {
    widgets[1].contentScale = scale;
    assert.equal(parseLayoutLibrary(library)?.layouts[0].template.widgets[1].contentScale, scale);
  }
  for (const invalid of [0, -50, 201, 75.5, NaN, Infinity, null, '70']) {
    const raw = structuredClone(library);
    Object.assign(raw.layouts[0].template.widgets[1], { contentScale: invalid });
    assert.equal(parseLayoutLibrary(raw), null);
  }
});

test('migrates a single saved layout without overwriting the original', () => {
  const storage = memoryStorage();
  const legacy = JSON.stringify({ ...defaultTemplate(), name: 'My campaign' });
  storage.setItem('hoi4-replay.workspace-template.v1', legacy);
  const { library, warning } = loadLayouts(storage);
  assert.equal(warning, null);
  assert.equal(library.layouts[0].template.name, 'My campaign');
  assert.equal(saveLayouts(library, storage), null);
  assert.equal(storage.getItem('hoi4-replay.workspace-template.v1'), legacy);
  assert.deepEqual(loadLayouts(storage).library, library);
});

test('renames the original default without changing custom layouts or creating duplicate names', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  library.layouts[0].template.name = 'Campaign overview';
  library.layouts.push({ id: 'custom', template: { ...defaultTemplate(), name: 'My layout' } });
  assert.equal(saveLayouts(library, storage), null);
  assert.deepEqual(loadLayouts(storage).library.layouts.map(entry => entry.template.name), ['Default layout', 'My layout']);
  library.layouts[1].template.name = 'Default layout';
  assert.equal(saveLayouts(library, storage), null);
  assert.deepEqual(loadLayouts(storage).library.layouts.map(entry => entry.template.name), ['Campaign overview', 'Default layout']);
});

test('multiple layouts, updates and active selection survive reload independently', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  const first = structuredClone(library.layouts[0]);
  const other = { id: 'other', template: { ...defaultTemplate(), name: 'Industry', widgets: [], layouts: { lg: [] } } };
  library.layouts.push(other);
  library.activeId = other.id;
  assert.equal(saveLayouts(library, storage), null);
  const restored = loadLayouts(storage).library;
  assert.equal(restored.activeId, 'other');
  assert.equal(restored.layouts[1].template.widgets.length, 0);
  restored.layouts[1].template.name = 'Industry comparison';
  restored.activeId = first.id;
  assert.equal(saveLayouts(restored, storage), null);
  const reloaded = loadLayouts(storage).library;
  assert.equal(reloaded.activeId, first.id);
  assert.deepEqual(reloaded.layouts[0].template.widgets, first.template.widgets);
  assert.equal(reloaded.layouts[1].template.name, 'Industry comparison');
});

test('layout library validates identity, unique names, bounds and strips runtime data', () => {
  const { library } = loadLayouts(memoryStorage());
  const entry = library.layouts[0];
  assert.equal(parseLayoutLibrary({ ...library, activeId: 'missing' }), null);
  assert.equal(parseLayoutLibrary({ ...library, layouts: [entry, entry] }), null);
  assert.equal(parseLayoutLibrary({ ...library, layouts: [entry, { ...entry, id: 'copy', template: { ...entry.template, name: ` ${entry.template.name.toUpperCase()} ` } }] }), null);
  assert.equal(parseLayoutLibrary({ ...library, layouts: [{ ...entry, template: { ...entry.template, name: '   ' } }] }), null);
  assert.equal(parseLayoutLibrary({ ...library, layouts: Array.from({ length: MAX_LAYOUTS + 1 }, (_, index) => ({ id: `layout-${index}`, template: { ...entry.template, name: `Layout ${index}` } })) }), null);
  const safe = parseLayoutLibrary({ ...library, sourceId: 'runtime-lease', layouts: [{ ...entry, template: { ...entry.template, path: 'private-recording', roles: entry.template.roles.map((role) => ({ ...role, sourceId: 'runtime-lease' })) } }] });
  assert.ok(safe);
  assert.equal(JSON.stringify(safe).includes('runtime-lease'), false);
  assert.equal(JSON.stringify(safe).includes('private-recording'), false);
});

test('failed storage preserves the saved library and unsaved draft', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  assert.equal(saveLayouts(library, storage), null);
  const previous = loadLayouts(storage).library;
  const draft = structuredClone(previous);
  draft.layouts[0].template.name = 'Unsaved edits';
  const blocked = { getItem: storage.getItem, setItem: () => { throw new Error('Quota exceeded'); } };
  assert.match(saveLayouts(draft, blocked)!, /could not be saved/);
  assert.deepEqual(loadLayouts(storage).library, previous);
  assert.equal(draft.layouts[0].template.name, 'Unsaved edits');
  storage.setItem('hoi4-replay.workspace-layouts.v1', '{damaged');
  assert.ok(loadLayouts(storage).warning);
});

test('widget options round-trip, survive an uninstalled widget type and refuse unsafe values', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  const widgets = library.layouts[0].template.widgets;
  widgets[0].options = { order: 'size', ticks: false, window: 90, yMax: null };
  widgets[1].kind = 'not-installed';
  widgets[1].options = { future: 'kept' };
  assert.equal(saveLayouts(library, storage), null);
  const restored = loadLayouts(storage).library.layouts[0].template.widgets;
  assert.deepEqual(restored[0].options, { order: 'size', ticks: false, window: 90, yMax: null });
  assert.equal(restored[1].kind, 'not-installed');
  assert.deepEqual(restored[1].options, { future: 'kept' });
  for (const invalid of [[], 'x', { a: {} }, { 'two words': 1 }, Object.fromEntries(Array.from({ length: 49 }, (_, i) => [`k${i}`, i]))]) {
    const raw = structuredClone(library);
    Object.assign(raw.layouts[0].template.widgets[0], { options: invalid });
    assert.equal(parseLayoutLibrary(raw), null);
  }
});

test('large explicit country selections survive saving and reloading without a count cap', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  const widget = library.layouts[0].template.widgets[0];
  widget.kind = 'scoreboard';
  for (const count of [300, 5000]) {
    const tags = Array.from({ length: count }, (_, index) => `synthetic_country_${index}`);
    widget.countries = { kind: 'tags', tags };
    assert.equal(saveLayouts(library, storage), null);
    const restored = loadLayouts(storage);
    assert.equal(restored.warning, null);
    assert.deepEqual(restored.library.layouts[0].template.widgets[0].countries, { kind: 'tags', tags });
  }
});

test('a factions widget retains every stored choice past its definition maximum', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  const widget = library.layouts[0].template.widgets[0];
  widget.kind = 'factions';
  const tags = Array.from({ length: 300 }, (_, index) => `synthetic_country_${index}`);
  widget.countries = { kind: 'tags', tags };
  assert.equal(saveLayouts(library, storage), null);
  assert.deepEqual(loadLayouts(storage).library.layouts[0].template.widgets[0].countries, { kind: 'tags', tags });
});

test('a saved Top-100 group reloads in every uncapped ranked widget', () => {
  for (const kind of ['territory', 'scoreboard', 'agency-comparison']) {
    const storage = memoryStorage();
    const { library } = loadLayouts(storage);
    const widget = library.layouts[0].template.widgets[0];
    widget.kind = kind;
    widget.countries = { kind: 'group', group: { type: 'top', count: 100 }, membership: 'hour', aggregate: false };
    assert.equal(saveLayouts(library, storage), null, kind);
    const restored = loadLayouts(storage);
    assert.equal(restored.warning, null, kind);
    assert.deepEqual(restored.library.layouts[0].template.widgets[0].countries, widget.countries, kind);
    widget.countries = { ...widget.countries, group: { type: 'top', count: Number.MAX_SAFE_INTEGER } };
    assert.deepEqual(parseLayoutLibrary(library)?.layouts[0].template.widgets[0].countries, widget.countries);
  }
});

for (const [label, count] of Object.entries({ zero: 0, negative: -1, fractional: 1.5, NaN, Infinity, string: '100', null: null, object: {}, boolean: true })) {
  test(`a stored top-N group refuses an invalid ${label} count`, () => {
    const storage = memoryStorage();
    const { library } = loadLayouts(storage);
    const countries = { kind: 'group', group: { type: 'top', count }, membership: 'hour', aggregate: false };
    Object.assign(library.layouts[0].template.widgets[0], { countries });
    assert.equal(parseLayoutLibrary(library), null);
    assert.match(saveLayouts(library, storage)!, /could not be saved/);
  });
}

test('a long content-set selection persists with an uninstalled widget', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  const widget = library.layouts[0].template.widgets[0];
  widget.kind = 'not-installed';
  widget.options = { picked: Array.from({ length: 20 }, (_, index) => `modded_type_${index}`.padEnd(200, 'x')).join(' ') };
  assert.equal(saveLayouts(library, storage), null);
  assert.deepEqual(loadLayouts(storage).library.layouts[0].template.widgets[0].options, widget.options);
});

test('modded, dynamic and unusual country tags survive saving and reloading', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  const template = library.layouts[0].template;
  // Tags are opaque: spaces, lower case, dynamic countries, long mod tags and non-ASCII text.
  const tags = ['D01', 'ger', 'my tag', 'MOD_LONG_COUNTRY_TAG_FROM_A_TOTAL_CONVERSION_MOD', 'ÉTAT', 'x'.repeat(300), '長'.repeat(100)];
  template.widgets[0].countries = { kind: 'tags', tags };
  template.roles[0].selectedTag = 'x'.repeat(300);
  assert.equal(saveLayouts(library, storage), null);
  const restored = loadLayouts(storage).library.layouts[0].template;
  assert.deepEqual(restored.widgets[0].countries, { kind: 'tags', tags });
  assert.equal(restored.roles[0].selectedTag, 'x'.repeat(300));
  // A tag the products would refuse costs only itself, never the widget or the library.
  const raw = structuredClone(library);
  raw.layouts[0].template.widgets[0].countries = { kind: 'tags', tags: ['GER', 'a\0b', 'x'.repeat(129), ''] };
  raw.layouts[0].template.roles[0].selectedTag = 'a\0b';
  const parsed = parseLayoutLibraryWithNotes(raw);
  assert.ok(parsed);
  assert.equal(parsed.droppedTags, 3);
  assert.deepEqual(parsed.library.layouts[0].template.widgets[0].countries, { kind: 'tags', tags: ['GER', 'x'.repeat(129)] });
  assert.equal(parsed.library.layouts[0].template.roles[0].selectedTag, null);
  storage.setItem('hoi4-replay.workspace-layouts.v1', JSON.stringify(raw));
  const loaded = loadLayouts(storage);
  assert.equal(loaded.droppedTags, 3);
  assert.match(loaded.warning!, /3 country tag entries were left out/);
  assert.deepEqual(loaded.library, parsed.library);
  const before = storage.getItem('hoi4-replay.workspace-layouts.v1');
  assert.match(saveLayouts(raw, storage)!, /3 country tag entries could not be saved/);
  assert.equal(storage.getItem('hoi4-replay.workspace-layouts.v1'), before);
  storage.values.delete('hoi4-replay.workspace-layouts.v1');
  storage.setItem('hoi4-replay.workspace-template.v1', JSON.stringify(raw.layouts[0].template));
  assert.equal(loadLayouts(storage).droppedTags, 3);
  assert.match(loadLayouts(storage).warning!, /3 country tag entries were left out/);
});

test('restoration drops non-string tag entries and counts notes across layouts', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  library.layouts.push({ id: 'other', template: { ...defaultTemplate(), name: 'Other choices' } });
  for (const entry of library.layouts) {
    Object.assign(entry.template.widgets[0], { countries: { kind: 'tags', tags: ['valid', 42, null, {}, true] } });
    entry.template.roles[0].selectedTag = '';
  }
  const parsed = parseLayoutLibraryWithNotes(library);
  assert.ok(parsed);
  assert.equal(parsed.droppedTags, 10);
  for (const entry of parsed.library.layouts) {
    assert.deepEqual(entry.template.widgets[0].countries, { kind: 'tags', tags: ['valid'] });
    assert.equal(entry.template.roles[0].selectedTag, null);
  }
  const saved = JSON.stringify(library);
  storage.setItem('hoi4-replay.workspace-layouts.v1', saved);
  const loaded = loadLayouts(storage);
  assert.equal(loaded.droppedTags, 10);
  assert.match(loaded.warning!, /10 country tag entries were left out/);
  assert.deepEqual(loaded.library, parsed.library);
  assert.equal(storage.getItem('hoi4-replay.workspace-layouts.v1'), saved);
});

test('an uninstalled widget retains faction selections beyond thirty without a private default limit', () => {
  const template = defaultTemplate(); template.widgets[0].kind = 'uninstalled-faction-widget';
  template.widgets[0].countries = { kind: 'factions', count: 5000, membership: 'end' };
  assert.deepEqual(parseTemplate(template)?.widgets[0].countries, template.widgets[0].countries);
});

test('stored faction-total counts survive an installed widget choice maximum', () => {
  const template = defaultTemplate(); template.widgets[0].kind = 'factions';
  template.widgets[0].countries = { kind: 'factions', count: 5000, membership: 'ever' };
  assert.deepEqual(parseTemplate(template)?.widgets[0].countries, template.widgets[0].countries);
});
