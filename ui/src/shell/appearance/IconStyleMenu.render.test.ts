import assert from 'node:assert/strict';
import test from 'node:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { MenuItem } from '../../components/ContextMenu';
import type { AppearanceSettings } from './settings';

type Fixture = { editing: { id: string; label: string; part: 'icon' | 'colour' } | null; items: readonly MenuItem[] };

test('icon menus offer each change, reset only what differs from the default, and leave images their own colours', async () => {
  const host = globalThis as typeof globalThis & { __iconMenu?: Fixture };
  const fixture: Fixture = { editing: null, items: [] };
  host.__iconMenu = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'icon-menu-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/appearance/IconStyleMenu.tsx')) return source
          .replace("import { useCallback, useEffect, useId, useRef, useState } from 'react';",
            'const useCallback = callback => callback; const useEffect = () => {}; const useId = () => "hint"; const useRef = value => ({ current: value }); const useState = value => [value, () => {}];')
          .replace('useState<{ id: Id; label: string; part: Part } | null>(null)', 'useState(globalThis.__iconMenu.editing)');
        if (file.endsWith('/appearance/WidgetIconMenu.tsx')) return source.replace("import { createContext, useMemo } from 'react';",
          "import { createContext } from 'react'; const useMemo = make => make();");
        if (file.endsWith('/dashboard/definitions.ts')) return "export const widgetDefinition = kind => kind === 'navy-size' ? { icon: 'anchor', title: 'Navy size' } : undefined;";
        if (file.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open: (_x, _y, items) => { globalThis.__iconMenu.items = items; }, menu: null });';
        if (file.endsWith('/components/Popover.tsx')) return `export const usePopover = () => ({ trigger: { current: null }, openAt() {}, open: false, close() {} });
          export const Popover = props => props.children;`;
      },
    }] });
  try {
    const { useSidebarIconMenu } = await vite.ssrLoadModule('/ui/src/shell/appearance/SidebarIconMenu.tsx') as typeof import('./SidebarIconMenu');
    const { useWidgetIconMenu } = await vite.ssrLoadModule('/ui/src/shell/appearance/WidgetIconMenu.tsx') as typeof import('./WidgetIconMenu');
    const { useIconStyleMenu, restoreFocus } = await vite.ssrLoadModule('/ui/src/shell/appearance/IconStyleMenu.tsx') as typeof import('./IconStyleMenu');
    const image = 'data:image/png;base64,iVBORw0KGgo=';
    let saved: AppearanceSettings | null = null;
    const appearance = (styles: Partial<Pick<AppearanceSettings, 'sidebarIcons' | 'widgetIcons'>>) => {
      const settings = { sidebarIcons: {}, widgetIcons: {}, ...styles } as AppearanceSettings;
      return { settings, resolved: { mode: 'dark', colors: { muted: '#8b95a1' } }, notice: null,
        update(change: (previous: AppearanceSettings) => AppearanceSettings) { saved = change(settings); } } as never;
    };
    const labels = (items: readonly MenuItem[]) => items.map(item => item.label);
    const anchor = { closest: () => null } as unknown as HTMLButtonElement;

    fixture.editing = { id: 'map', label: 'Campaign map', part: 'colour' };
    const picker = (styles: AppearanceSettings['sidebarIcons']) => renderToStaticMarkup(useSidebarIconMenu(appearance({ sidebarIcons: styles })).menu);
    assert.doesNotMatch(picker({}), /Reset icon colour/, 'a default colour has nothing to reset');
    assert.match(picker({ map: { colour: '#ff0000' } }), /aria-label="Reset icon colour" data-tooltip="Reset icon colour"/, 'the reset tooltip names what it resets');
    fixture.editing = { id: 'map', label: 'Campaign map', part: 'icon' };
    assert.doesNotMatch(picker({ map: { colour: '#ff0000' } }), /aria-label="Reset icon"/, 'a default icon has nothing to reset');
    assert.match(picker({ map: { icon: 'globe' } }), /aria-label="Reset icon" data-tooltip="Reset icon"/);
    assert.match(picker({}), />Upload image…</);
    assert.match(picker({ map: { icon: image } }), /<img class="icon-image" src="data:image\/png;base64,iVBORw0KGgo=" alt="Your image"/);
    assert.match(picker({ map: { icon: image } }), />Replace image…</);
    const sidebar = (styles: AppearanceSettings['sidebarIcons']) => useSidebarIconMenu(appearance({ sidebarIcons: styles })).items('map', anchor);
    assert.deepEqual(labels(sidebar({ map: { icon: 'globe', colour: '#ff0000' } })), ['Change icon', 'Change icon colour']);
    assert.equal(sidebar({ map: { icon: 'globe' } })[1].disabled, false);
    assert.equal(sidebar({ map: { icon: image } })[1].disabled, true);
    assert.equal(sidebar({ map: { icon: image } })[1].hint, 'Images keep their own colours');

    const header = (styles: AppearanceSettings['widgetIcons']) => {
      fixture.items = [];
      useWidgetIconMenu(appearance({ widgetIcons: styles })).icons.openMenu('navy-size', anchor, 'Navy size', 0, 0);
      return fixture.items;
    };
    assert.deepEqual(labels(header({})), ['Change icon', 'Change icon colour']);
    assert.deepEqual(labels(header({ 'navy-size': { icon: 'globe' } })), ['Change icon', 'Change icon colour', 'Reset icon']);
    assert.deepEqual(labels(header({ 'navy-size': { colour: '#ff0000' } })), ['Change icon', 'Change icon colour', 'Reset icon colour']);
    assert.deepEqual(labels(header({ aces: { icon: 'globe' } })), ['Change icon', 'Change icon colour'], 'another kind keeps its own icon');
    const imaged = header({ 'navy-size': { icon: image, colour: '#ff0000' } });
    assert.deepEqual(labels(imaged), ['Change icon', 'Change icon colour', 'Reset icon', 'Reset icon colour']);
    assert.equal(imaged[1].disabled, true);
    assert.equal(imaged[1].hint, 'Images keep their own colours');
    imaged[2].run();
    assert.deepEqual(saved!.widgetIcons, { 'navy-size': { colour: '#ff0000' } });
    useWidgetIconMenu(appearance({ widgetIcons: { 'navy-size': { icon: 'globe', colour: '#ff0000' }, aces: { icon: 'star' } } })).icons.reset('navy-size');
    assert.deepEqual(saved!.widgetIcons, { aces: { icon: 'star' } });

    const menu = useIconStyleMenu(appearance({ sidebarIcons: { map: { icon: 'globe', colour: '#ff0000' } } }), {
      label: 'Sidebar icon', read: settings => settings.sidebarIcons, write: (settings, sidebarIcons) => ({ ...settings, sidebarIcons }),
      defaultIcon: () => 'map', defaultColour: () => '#34d399', swatches: [] });
    menu.change('map', { icon: 'map' });
    assert.deepEqual(saved!.sidebarIcons, { map: { colour: '#ff0000' } });
    menu.change('map', { colour: '#34d399' });
    assert.deepEqual(saved!.sidebarIcons, { map: { icon: 'globe' } });
    menu.change('map', { icon: 'flag', colour: '#00ff00' });
    assert.deepEqual(saved!.sidebarIcons, { map: { icon: 'flag', colour: '#00ff00' } });

    let focused = 0;
    const target = { isConnected: true, focus: () => { focused++; } } as unknown as HTMLElement;
    const body = {} as Element, inside = {} as Element, elsewhere = {} as Element;
    const popup = { contains: (node: Element) => node === inside } as unknown as HTMLElement;
    assert.equal(restoreFocus(target, popup, body, body), true, 'Escape leaves focus on the page');
    assert.equal(restoreFocus(target, popup, inside, body), true, 'the close button keeps focus inside the picker');
    assert.equal(restoreFocus(target, popup, null, body), true);
    assert.equal(restoreFocus(target, popup, elsewhere, body), false, 'focus the user moved elsewhere stays there');
    assert.equal(restoreFocus({ isConnected: false, focus: () => { focused += 10; } } as unknown as HTMLElement, popup, body, body), false, 'a removed target is skipped');
    assert.equal(restoreFocus(null, popup, body, body), false);
    assert.equal(focused, 3);
  } finally { await vite.close(); delete host.__iconMenu; }
});
