import test from 'node:test';
import assert from 'node:assert/strict';
import { defaultTemplate } from './model.ts';
import { loadLayouts, MAX_LAYOUTS, parseLayoutLibrary, saveLayouts } from './storage.ts';

function memoryStorage() {
  const values = new Map<string, string>();
  return { values, getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
}

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
