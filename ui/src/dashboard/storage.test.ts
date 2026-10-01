import test from 'node:test';
import assert from 'node:assert/strict';
import { defaultTemplate, WIDGET_CONTENT_SCALES } from './model.ts';
import { loadLayouts, MAX_LAYOUTS, parseLayoutLibrary, parseLayoutLibraryWithNotes, saveLayouts } from './storage.ts';
import '../widgets/definitions.ts';

function memoryStorage() {
  const values = new Map<string, string>();
  return { values, getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
}

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

test('modded, dynamic and unusual country tags survive saving and reloading', () => {
  const storage = memoryStorage();
  const { library } = loadLayouts(storage);
  const template = library.layouts[0].template;
  // Tags are opaque: spaces, lower case, dynamic countries, long mod tags and non-ASCII text.
  const tags = ['D01', 'ger', 'my tag', 'MOD_LONG_COUNTRY_TAG_FROM_A_TOTAL_CONVERSION_MOD', 'ÉTAT'];
  template.widgets[0].countries = { kind: 'tags', tags };
  template.roles[0].selectedTag = 'my tag';
  assert.equal(saveLayouts(library, storage), null);
  const restored = loadLayouts(storage).library.layouts[0].template;
  assert.deepEqual(restored.widgets[0].countries, { kind: 'tags', tags });
  assert.equal(restored.roles[0].selectedTag, 'my tag');
  // A tag the products would refuse costs only itself, never the widget or the library.
  const raw = structuredClone(library);
  raw.layouts[0].template.widgets[0].countries = { kind: 'tags', tags: ['GER', 'a\0b', 'x'.repeat(129), ''] };
  raw.layouts[0].template.roles[0].selectedTag = 'x'.repeat(129);
  const parsed = parseLayoutLibraryWithNotes(raw);
  assert.ok(parsed);
  assert.equal(parsed.droppedTags, 4);
  assert.deepEqual(parsed.library.layouts[0].template.widgets[0].countries, { kind: 'tags', tags: ['GER'] });
  assert.equal(parsed.library.layouts[0].template.roles[0].selectedTag, null);
  storage.setItem('hoi4-replay.workspace-layouts.v1', JSON.stringify(raw));
  const loaded = loadLayouts(storage);
  assert.equal(loaded.droppedTags, 4);
  assert.match(loaded.warning!, /4 country tag entries were left out/);
  assert.deepEqual(loaded.library, parsed.library);
  const before = storage.getItem('hoi4-replay.workspace-layouts.v1');
  assert.match(saveLayouts(raw, storage)!, /4 country tag entries could not be saved/);
  assert.equal(storage.getItem('hoi4-replay.workspace-layouts.v1'), before);
  storage.values.delete('hoi4-replay.workspace-layouts.v1');
  storage.setItem('hoi4-replay.workspace-template.v1', JSON.stringify(raw.layouts[0].template));
  assert.equal(loadLayouts(storage).droppedTags, 4);
  assert.match(loadLayouts(storage).warning!, /4 country tag entries were left out/);
});
