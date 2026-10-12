import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createServer } from 'vite';
import type { ReactElement } from 'react';
import type { MenuItem } from '../components/ContextMenu';
import type { PageEntry } from '../dashboard/pages';

type Node = ReactElement<Record<string, any>>;
function elements(value: any): Node[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  if (!value || typeof value !== 'object' || !('props' in value)) return [];
  return [value, ...elements(value.props.children)];
}

test('pages reorder by drag or Alt+arrow keys, not from the page menu', async () => {
  let items: MenuItem[] = [];
  const fixture = { open: (_x: number, _y: number, next: MenuItem[]) => { items = next; } };
  const host = globalThis as typeof globalThis & { __pageMenu?: typeof fixture };
  host.__pageMenu = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'page-menu-fixture', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/shell/AnalyticsNav.tsx')) return source.replace("import { useState } from 'react';", 'const useState = value => [value, () => {}];');
        if (id.replaceAll('\\', '/').endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open: globalThis.__pageMenu.open, menu: null });';
      },
    }] });
  try {
    const { default: Nav } = await vite.ssrLoadModule('/ui/src/shell/AnalyticsNav.tsx');
    let pages: PageEntry[] = ['a', 'b', 'c'].map(id => ({ id: `page-${id}`, label: id }));
    const render = () => Nav({ pages, page: 'page-b', active: true, disabled: false, onChoose() {}, onDelete() {},
      onPages: (next: PageEntry[]) => { pages = next; }, iconMenu: { colour: () => '', items: () => [] } });
    const openMenu = (id: string) => {
      const row = elements(render()).find(node => node.type === 'button' && node.props.children?.some?.((child: any) => child?.props?.children === id))!;
      assert.ok(row, `page ${id} remains rendered`);
      let prevented = false;
      row.props.onKeyDown({ key: 'F10', shiftKey: true, preventDefault() { prevented = true; },
        currentTarget: { getBoundingClientRect: () => ({ left: 0, bottom: 32 }) } });
      assert.ok(prevented);
      return items;
    };
    const press = (id: string, key: string) => {
      const row = elements(render()).find(node => node.type === 'button' && node.props.children?.some?.((child: any) => child?.props?.children === id))!;
      row.props.onKeyDown({ key, altKey: true, preventDefault() {} });
    };
    assert.deepEqual(openMenu('b').map(item => item.label), ['Rename', 'Delete'], 'the page menu has no move actions');
    press('b', 'ArrowUp');
    assert.deepEqual(pages.map(page => page.label), ['b', 'a', 'c']);
    press('b', 'ArrowUp');
    assert.deepEqual(pages.map(page => page.label), ['b', 'a', 'c'], 'the first page cannot move up');
    press('b', 'ArrowDown');
    assert.deepEqual(pages.map(page => page.label), ['a', 'b', 'c']);
    press('c', 'ArrowDown');
    assert.deepEqual(pages.map(page => page.label), ['a', 'b', 'c'], 'the last page cannot move down');
    pages = Array.from({ length: 40 }, (_, index) => ({ id: `page-${index}`, label: `Page ${index}` }));
    const list = elements(render()).find(node => node.props.className === 'sidebar-page-list');
    assert.ok(list, 'long page lists have their own scroll container');
    assert.equal(elements(list.props.children).filter(node => node.type === 'button').length, 40);
  } finally {
    await vite.close(); delete host.__pageMenu;
  }
});

test('sidebar gives overflowing pages a bounded scroll area without shrinking footer controls', () => {
  const css = readFileSync(new URL('./shell.css', import.meta.url), 'utf8');
  const rule = (selector: string) => {
    const declarations = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].find(match => match[1].trim() === selector)?.[2];
    assert.ok(declarations, selector);
    return declarations;
  };
  assert.match(rule('.sidebar-menu'), /min-height:\s*0/);
  assert.match(rule('.sidebar-page-list'), /min-height:\s*0/);
  assert.match(rule('.sidebar-page-list'), /overflow-y:\s*auto/);
  assert.match(rule('.sidebar-page-list > *'), /flex-shrink:\s*0/);
  assert.match(rule('.sidebar-campaign-host'), /overflow:\s*hidden/);
  assert.match(rule('.sidebar-footer'), /flex-shrink:\s*0/);
});
