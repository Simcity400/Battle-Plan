import test from 'node:test';
import assert from 'node:assert/strict';
import { widgetDefinition, widgetDefinitions } from './definitions.ts';
import { defaultTemplate, GRID_COLUMNS } from './model.ts';
import { defaultPages, loadPages, movePage, PAGES, parsePageList, savePages } from './pages.ts';
import { parseTemplate } from './storage.ts';
import '../widgets/definitions.ts';

test('every default panel names a registered widget, every widget has a page, and every page arrangement fits the grid without overlaps', () => {
  const placed = new Set<string>(PAGES.flatMap(page => page.panels.map(([kind]) => kind)));
  assert.deepEqual(widgetDefinitions().map(definition => definition.kind).filter(kind => !placed.has(kind)), []);
  for (const page of PAGES) {
    assert.ok(page.panels.length, `${page.id}: a built-in page shows at least one widget`);
    const cells = new Set<string>();
    for (const [kind, x, y, w, h] of page.panels) {
      const definition = widgetDefinition(kind);
      assert.ok(definition, `${page.id}: ${kind} is installed`);
      assert.ok(x + w <= GRID_COLUMNS.lg && w >= definition.minSize.w && h >= definition.minSize.h, `${page.id}: ${kind} fits`);
      for (let column = x; column < x + w; column++) for (let row = y; row < y + h; row++) {
        assert.ok(!cells.has(`${column},${row}`), `${page.id}: ${kind} overlaps`);
        cells.add(`${column},${row}`);
      }
    }
    assert.ok(parseTemplate(defaultTemplate(page.id)), `${page.id}: default layout is storable`);
  }
});

test('saved page lists keep names and order, refuse damaged entries and fall back to the built-in pages', () => {
  const values = new Map<string, string>();
  const storage = { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => { values.set(key, value); } };
  assert.deepEqual(loadPages(storage), defaultPages());
  const pages = [{ id: 'page-1a', label: 'Mine' }, { id: 'politics', label: 'Cabinet' }];
  assert.equal(savePages(pages, storage), null);
  assert.deepEqual(loadPages(storage), pages);
  for (const bad of [[], [{ id: 'unknown', label: 'X' }], [{ id: 'politics', label: '  ' }], [{ id: 'politics', label: 'A' }, { id: 'politics', label: 'B' }]]) {
    assert.equal(parsePageList(bad), null);
  }
  values.set('battleplan.analytics-pages.v1', '{');
  assert.deepEqual(loadPages(storage), defaultPages());
});

test('a dragged page lands before the page it was dropped on, or at the end', () => {
  const ids = (list: { id: string }[]) => list.map(page => page.id);
  const pages = ['overview', 'politics', 'page-a', 'page-b'].map(id => ({ id, label: id }));
  assert.deepEqual(ids(movePage(pages, 0, 2)), ['politics', 'overview', 'page-a', 'page-b']);
  assert.deepEqual(ids(movePage(pages, 3, 0)), ['page-b', 'overview', 'politics', 'page-a']);
  assert.deepEqual(ids(movePage(pages, 1, 4)), ['overview', 'page-a', 'page-b', 'politics']);
});
