import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import type { ReactElement } from 'react';

type Node = ReactElement<Record<string, any>>;
function elements(value: any): Node[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  if (!value || typeof value !== 'object' || !('props' in value)) return [];
  return [value, ...elements(value.props.children)];
}

for (const scenario of ['peek', 'storage'] as const) test(scenario === 'peek'
  ? 'sidebar peek occludes both native-map paths until the sidebar is kept open'
  : 'page-save warnings survive view changes and reset until saving succeeds', async () => {
  const slots: any[] = [];
  let cursor = 0;
  let layouts: (() => void)[] = [];
  let mapVisible = false;
  let overlayOpen = false;
  let quota = true;
  const values = new Map([['hoi4-replay.sidebar.v1', 'hidden']]);
  const fixture = {
    useState(initial: any) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (next: any) => { slots[index] = typeof next === 'function' ? next(slots[index]) : next; }];
    },
    useRef(initial: any) { const index = cursor++; return slots[index] ??= { current: initial }; },
    useCallback(callback: any) { return callback; },
    useEffect() {},
    useLayoutEffect(effect: () => (() => void) | undefined, deps: unknown[]) {
      const index = cursor++;
      const previous = slots[index];
      if (previous && deps.every((value, index) => Object.is(value, previous.deps[index]))) return;
      layouts.push(() => { previous?.cleanup?.(); slots[index] = { deps, cleanup: effect() }; });
    },
    mapSurface(_element: unknown, visible: boolean) { mapVisible = visible; return null; },
  };
  const host = globalThis as typeof globalThis & { __shellFixture?: typeof fixture };
  host.__shellFixture = fixture;
  const globals = ['window', 'document', 'localStorage'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { innerWidth: 1200 } });
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { querySelector: () => overlayOpen ? {} : null } });
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { if (quota && key === 'hoi4-replay.analytics-pages.v1') throw new Error('quota'); values.set(key, value); },
  } });
  const replacements: Record<string, string> = {
    '/shell/api.ts': `export const desktopAvailable = true; export const errorMessage = String;
      export const selectNativeView = async () => {}; export const openReplay = async () => {}; export const mapAction = async () => {};`,
    '/shell/useNativeViewer.ts': `export const useDesktopState = () => ({ state: { ready: true, running: true, viewer: { path: 'synthetic' } }, error: null });
      export const useMapSurface = (...args) => globalThis.__shellFixture.mapSurface(...args);`,
    '/shell/appearance/useAppearance.ts': 'export const useAppearance = () => ({ settings: { backdrop: { kind: "none" } } });',
    '/shell/appearance/SidebarIconMenu.tsx': 'export const useSidebarIconMenu = () => ({ menu: null });',
    '/shell/appearance/Backdrop.tsx': 'export default () => null;',
    '/shell/Settings.tsx': `export default function Settings() {} export function SettingsNav() {}
      export const settingsSections = [{ id: 'folders', label: 'Folders' }];`,
    '/dashboard/nativeClient.ts': 'export const nativeDashboardClient = {};',
    '/dashboard/browserClient.ts': 'export const browserDashboardClient = {};',
    '/dashboard/replayLibrary.ts': 'export const useReplayLibrary = () => ({});',
    '/dashboard/storage.ts': 'export const clearLayouts = () => {};',
    '/dashboard/contentRevision.ts': 'export const contentRevision = () => ({ observe() {} });',
    '/shell/diagnosticsSetting.ts': 'export const useDiagnosticsShortcut = () => {};',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'shell-interaction-fixture', enforce: 'pre',
      transform(source, id) {
        const name = id.replaceAll('\\', '/');
        if (name.endsWith('/shell/App.tsx')) return source.replace(/import \{[^\n]+\} from 'react';/,
          `import { Suspense } from 'react';
           const { useState, useRef, useCallback, useEffect, useLayoutEffect } = globalThis.__shellFixture;
           const memo = value => value; const lazy = () => function Dashboard() {};`);
        return Object.entries(replacements).find(([suffix]) => name.endsWith(suffix))?.[1];
      },
    }] });
  try {
    const { default: App } = await vite.ssrLoadModule('/ui/src/shell/App.tsx');
    const { hasMapOverlay } = await vite.ssrLoadModule('/ui/src/shell/mapOverlays.ts');
    const overlay = { showPopover() { overlayOpen = true; }, hidePopover() { overlayOpen = false; } };
    let nodes: Node[];
    const render = () => {
      cursor = 0; layouts = [];
      nodes = elements(App());
      const sentinel = nodes.find(node => node.props.className === 'sidebar-peek-overlay');
      if (sentinel) { assert.equal(sentinel.props.popover, 'manual'); sentinel.props.ref.current = overlay; }
      for (const effect of layouts) effect();
    };
    const find = (predicate: (node: Node) => boolean) => { const node = nodes.find(predicate); assert.ok(node); return node; };
    const dashboard = () => find(node => 'onPrimaryPath' in node.props);
    const navigation = () => find(node => 'onPages' in node.props);
    const warning = () => find(node => node.props.className === 'shell-header').props.children
      .find((child: any) => child?.props?.className === 'notice warning shell-page-storage-warning');
    render();
    dashboard().props.onPrimaryPath('synthetic'); render();
    assert.equal(mapVisible, true);
    assert.equal(hasMapOverlay(), false);
    if (scenario === 'peek') {
      find(node => node.props.className === 'sidebar-edge').props.onPointerEnter(); render();
      assert.equal(mapVisible, false, 'standalone native map hides while peeking');
      assert.equal(hasMapOverlay(), true, 'dashboard native-map guard sees peek through the shared menu mechanism');
      find(node => node.props['aria-label'] === 'Keep sidebar open').props.onClick(); render();
      assert.equal(mapVisible, true);
      assert.equal(hasMapOverlay(), false, 'keeping the sidebar open removes its overlay');

    } else {
      navigation().props.onPages([...navigation().props.pages, { id: 'page-added', label: 'Added' }]); render();
      const notice = warning();
      assert.ok(notice, 'failed saves warn in the active view');
      assert.equal(notice.props.role, 'status');
      assert.match(notice.props.children, /could not save/);
      assert.ok(navigation().props.pages.some((entry: any) => entry.id === 'page-added'), 'session changes still apply');
      navigation().props.onPages(navigation().props.pages.map((entry: any) => entry.id === 'page-added' ? { ...entry, label: 'Renamed' } : entry)); render();
      assert.equal(warning().props.children, notice.props.children, 'failed rename saves also warn');
      find(node => node.props.id === 'tab-settings').props.onClick();
      await new Promise(resolve => setImmediate(resolve)); render();
      assert.equal(find(node => node.props.className === 'app-shell').props['data-tab'], 'settings');
      assert.equal(warning().props.children, notice.props.children, 'changing tabs cannot clear the storage warning');
      find(node => 'onResetPages' in node.props).props.onResetPages(); render();
      assert.equal(warning().props.children, notice.props.children, 'failed reset saves also warn');
      find(node => 'onBack' in node.props).props.onBack();
      await new Promise(resolve => setImmediate(resolve)); render();
      assert.ok(warning());
      quota = false;
      navigation().props.onPages(navigation().props.pages); render();
      assert.equal(warning(), undefined, 'only a successful page save clears the warning');
    }
  } finally {
    await vite.close(); delete host.__shellFixture;
    for (const [key, descriptor] of globals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
  }
});
