import test from 'node:test';
import assert from 'node:assert/strict';
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
        if (id.replaceAll('\\', '/').endsWith('/shell/AnalyticsNav.tsx')) return source.replace("import { useRef, useState } from 'react';", 'const useState = value => [value, () => {}]; const useRef = value => ({ current: value });');
        if (id.replaceAll('\\', '/').endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open: globalThis.__pageMenu.open, menu: null });';
      },
    }] });
  try {
    const { default: Nav } = await vite.ssrLoadModule('/ui/src/shell/AnalyticsNav.tsx');
    let pages: PageEntry[] = ['a', 'b', 'c'].map(id => ({ id: `page-${id}`, label: id }));
    const render = () => Nav({ pages, page: 'page-b', active: true, disabled: false, onChoose() {}, onDelete() {},
      onPages: (next: PageEntry[]) => { pages = next; }, iconMenu: { custom: () => undefined, icon: () => undefined, items: () => [] } });
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
    const remove = openMenu('b')[1];
    assert.equal(remove.danger, true);
    assert.deepEqual(remove.confirm, { title: 'Delete “b”?', detail: 'Its widgets and saved layouts are removed.', action: 'Delete page' }, 'deleting a page asks first, naming it');
    press('b', 'ArrowUp');
    assert.deepEqual(pages.map(page => page.label), ['b', 'a', 'c']);
    press('b', 'ArrowUp');
    assert.deepEqual(pages.map(page => page.label), ['b', 'a', 'c'], 'the first page cannot move up');
    press('b', 'ArrowDown');
    assert.deepEqual(pages.map(page => page.label), ['a', 'b', 'c']);
    press('c', 'ArrowDown');
    assert.deepEqual(pages.map(page => page.label), ['a', 'b', 'c'], 'the last page cannot move down');
    pages = Array.from({ length: 40 }, (_, index) => ({ id: `page-${index}`, label: `Page ${index}` }));
    const list = elements(render()).find(node => node.props.className === 'sidebar-page-list scroll-fade');
    assert.ok(list, 'long page lists have their own scroll container');
    assert.equal(elements(list.props.children).filter(node => node.type === 'button').length, 40);
  } finally {
    await vite.close(); delete host.__pageMenu;
  }
});

test('Enter or Esc in a page rename gives focus back to its row, and Esc keeps the old name', async () => {
  const slots: any[] = [];
  let cursor = 0;
  const hooks = {
    useState(initial: any) { const index = cursor++; if (!(index in slots)) slots[index] = initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === 'function' ? value(slots[index]) : value; }]; },
    useRef(initial: any) { const index = cursor++; return slots[index] ??= { current: initial }; },
  };
  const host = globalThis as typeof globalThis & { __pageHooks?: typeof hooks };
  host.__pageHooks = hooks;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'page-rename-fixture', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/shell/AnalyticsNav.tsx')) return source.replace("import { useRef, useState } from 'react';", 'const { useRef, useState } = globalThis.__pageHooks;');
        if (id.replaceAll('\\', '/').endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open() {}, menu: null });';
      },
    }] });
  try {
    const { default: Nav } = await vite.ssrLoadModule('/ui/src/shell/AnalyticsNav.tsx');
    let pages: PageEntry[] = [{ id: 'page-a', label: 'Alpha' }, { id: 'page-b', label: 'Beta' }];
    let nodes: Node[] = [];
    const render = () => { cursor = 0; nodes = elements(Nav({ pages, page: 'page-a', active: true, disabled: false, onChoose() {}, onDelete() {},
      onPages: (next: PageEntry[]) => { pages = next; }, iconMenu: { custom: () => undefined, icon: () => undefined, items: () => [] } })); };
    const row = (label: string) => nodes.find(node => node.type === 'button' && node.props.children?.some?.((child: any) => child?.props?.children === label))!;
    const input = () => nodes.find(node => node.type === 'input')!;
    const focused: string[] = [];
    const element = (name: string) => ({ focus: () => { focused.push(name); } });
    render();
    row('Beta').props.onKeyDown({ key: 'F2', preventDefault() {} }); render();
    const field = { value: 'Typed', blur() { input().props.onBlur({ currentTarget: field }); } };
    let stopped = false;
    const editor = input();
    editor.props.onKeyDown({ key: 'Escape', preventDefault() {}, stopPropagation() { stopped = true; }, currentTarget: field }); render();
    editor.props.onBlur({ currentTarget: field }); render();
    assert.ok(stopped, 'Esc ends only the rename');
    assert.deepEqual(pages.map(page => page.label), ['Alpha', 'Beta'], 'Esc keeps the old name');
    row('Beta').props.ref(element('Beta after Esc'));
    row('Beta').props.ref(element('Beta again'));
    row('Alpha').props.ref(element('Alpha'));
    assert.deepEqual(focused, ['Beta after Esc']);
    row('Beta').props.onKeyDown({ key: 'F2', preventDefault() {} }); render();
    field.value = 'Gamma';
    input().props.onKeyDown({ key: 'Enter', currentTarget: field }); render();
    assert.deepEqual(pages.map(page => page.label), ['Alpha', 'Gamma']);
    row('Gamma').props.ref(element('Gamma after Enter'));
    assert.deepEqual(focused, ['Beta after Esc', 'Gamma after Enter']);
    row('Alpha').props.onKeyDown({ key: 'F2', preventDefault() {} }); render();
    field.value = 'Clicked away'; field.blur(); render();
    row('Clicked away').props.ref(element('Clicked away'));
    assert.deepEqual(focused, ['Beta after Esc', 'Gamma after Enter'], 'a rename left by a click leaves focus where the click put it');
  } finally {
    await vite.close(); delete host.__pageHooks;
  }
});

