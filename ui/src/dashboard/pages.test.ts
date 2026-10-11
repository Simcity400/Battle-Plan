import test from 'node:test';
import assert from 'node:assert/strict';
import { widgetDefinition, widgetDefinitions } from './definitions.ts';
import { defaultTemplate, GRID_COLUMNS } from './model.ts';
import { defaultPages, loadPages, movePage, PAGES, parsePageList, savePages } from './pages.ts';
import { parseTemplate } from './storage.ts';
import '../widgets/definitions.ts';

test('every widget has a page, and every page arrangement fits the grid without overlaps', () => {
  const placed = new Set<string>(PAGES.flatMap(page => page.panels.map(([kind]) => kind)));
  assert.deepEqual(widgetDefinitions().map(definition => definition.kind).filter(kind => !placed.has(kind)), []);
  for (const page of PAGES) {
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
  const pages = [{ id: 'page-1a', label: 'Mine' }, { id: 'navy', label: 'Fleet' }];
  assert.equal(savePages(pages, storage), null);
  assert.deepEqual(loadPages(storage), pages);
  for (const bad of [[], [{ id: 'unknown', label: 'X' }], [{ id: 'navy', label: '  ' }], [{ id: 'navy', label: 'A' }, { id: 'navy', label: 'B' }]]) {
    assert.equal(parsePageList(bad), null);
  }
  values.set('hoi4-replay.analytics-pages.v1', '{');
  assert.deepEqual(loadPages(storage), defaultPages());
});

test('a dragged page lands before the page it was dropped on, or at the end', () => {
  const ids = (list: { id: string }[]) => list.map(page => page.id);
  const pages = defaultPages().slice(0, 4);
  assert.deepEqual(ids(movePage(pages, 0, 2)), ['politics', 'overview', 'diplomacy', 'economy']);
  assert.deepEqual(ids(movePage(pages, 3, 0)), ['economy', 'overview', 'politics', 'diplomacy']);
  assert.deepEqual(ids(movePage(pages, 1, 4)), ['overview', 'diplomacy', 'economy', 'politics']);
});
