import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { MenuItem } from '../../components/ContextMenu';

test('an icon colour resets from its colour picker, and only once it differs from the default', async () => {
  const host = globalThis as typeof globalThis & { __iconMenu?: { editing: string; items: MenuItem[] } };
  host.__iconMenu = { editing: 'map', items: [] };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'icon-menu-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/appearance/SidebarIconMenu.tsx')) return source
          .replace("import { useEffect, useState } from 'react';", 'const useEffect = () => {}; const useState = value => [value, () => {}];')
          .replace('useState<SidebarIconId | null>(null)', 'useState(globalThis.__iconMenu.editing)');
        if (file.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open: (_x, _y, items) => { globalThis.__iconMenu.items = items; }, menu: null });';
        if (file.endsWith('/components/Popover.tsx')) return `export const usePopover = () => ({ trigger: { current: null }, openAt() {}, open: false, close() {} });
          export const Popover = props => props.children;`;
      },
    }] });
  try {
    const { useSidebarIconMenu } = await vite.ssrLoadModule('/ui/src/shell/appearance/SidebarIconMenu.tsx') as typeof import('./SidebarIconMenu');
    const appearance = (sidebarIcons: Record<string, string>) => ({ settings: { sidebarIcons }, resolved: { mode: 'dark' }, update() {}, notice: null }) as never;
    const picker = (sidebarIcons: Record<string, string>) => renderToStaticMarkup(useSidebarIconMenu(appearance(sidebarIcons)).menu);
    assert.doesNotMatch(picker({}), /Reset icon colour/, 'a default colour has nothing to reset');
    assert.match(picker({ map: '#ff0000' }), /aria-label="Reset icon colour"/);
    const menu = useSidebarIconMenu(appearance({ map: '#ff0000' }));
    assert.deepEqual(menu.items('map', {} as HTMLButtonElement).map(item => item.label), ['Change icon colour'], 'the right-click menu only opens the picker');
  } finally { await vite.close(); delete host.__iconMenu; }
});
