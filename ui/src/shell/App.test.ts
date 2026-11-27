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

const scenarios = {
  peek: 'sidebar peek cuts its hole in both native-map paths until the sidebar is kept open',
  storage: 'page-save warnings survive view changes and reset until saving succeeds',
  retry: 'Try again on the map sends each failed map operation again and shows its failure until that operation succeeds',
  switch: 'a switch to the map that cannot open its recording keeps the view shown, which offers the whole switch again',
  narrow: 'a window too narrow for the workspace keeps the sidebar at its minimum width, and its chosen width once widened again',
} as const;
for (const scenario of Object.keys(scenarios) as (keyof typeof scenarios)[]) test(scenarios[scenario], async () => {
  const slots: any[] = [];
  let cursor = 0;
  let layouts: (() => void)[] = [];
  let mapVisible = false;
  let overlayOpen = false;
  let quota = true;
  const values = new Map([['battleplan.sidebar.v1', 'hidden']]);
  const fixture = {
    useState(initial: any) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (next: any) => { slots[index] = typeof next === 'function' ? next(slots[index]) : next; }];
    },
    useRef(initial: any) { const index = cursor++; return slots[index] ??= { current: initial }; },
    useCallback(callback: any) { return callback; },
    useMemo(factory: () => unknown) { const index = cursor++; return (slots[index] ??= { value: factory() }).value; },
    useEffect() {},
    useSyncExternalStore(_subscribe: unknown, snapshot: () => unknown) { return snapshot(); },
    useLayoutEffect(effect: () => (() => void) | undefined, deps: unknown[]) {
      const index = cursor++;
      const previous = slots[index];
      if (previous && deps.every((value, index) => Object.is(value, previous.deps[index]))) return;
      layouts.push(() => { previous?.cleanup?.(); slots[index] = { deps, cleanup: effect() }; });
    },
    mapSurface(_element: unknown, visible: boolean) { mapVisible = visible; return null; },
    viewerPath: 'synthetic',
    openReplay: async (_path: string) => {},
    selectNativeView: async (_dashboard: boolean) => {},
  };
  const host = globalThis as typeof globalThis & { __shellFixture?: typeof fixture };
  host.__shellFixture = fixture;
  const globals = ['window', 'document', 'localStorage'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { innerWidth: 1200 } });
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { querySelector: () => overlayOpen ? {} : null } });
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { if (quota && key === 'battleplan.analytics-pages.v1') throw new Error('quota'); values.set(key, value); },
    removeItem: (key: string) => { values.delete(key); },
  } });
  const replacements: Record<string, string> = {
    '/shell/api.ts': `export const desktopAvailable = true; export const errorMessage = String;
      export const mapAction = async () => {};
      export const selectNativeView = dashboard => globalThis.__shellFixture.selectNativeView(dashboard);
      export const openReplay = path => globalThis.__shellFixture.openReplay(path);`,
    '/shell/useNativeViewer.ts': `export const useDesktopState = () => ({ state: { ready: true, running: true, viewer: { path: globalThis.__shellFixture.viewerPath, hour: 0 } }, error: null, launchRecording: null });
      export const useMapSurface = (...args) => globalThis.__shellFixture.mapSurface(...args);`,
    '/shell/appearance/useAppearance.ts': 'export const useAppearance = () => ({ settings: { backdrop: { kind: "none" }, workspaceBackdrop: { kind: "same" } } });',
    '/shell/appearance/SidebarIconMenu.tsx': 'export const useSidebarIconMenu = () => ({ menu: null, icon: () => undefined, custom: () => undefined });',
    '/shell/appearance/WidgetIconMenu.tsx': `export const useWidgetIconMenu = () => ({ icons: null, menu: null });
      export const WidgetIconsContext = { Provider: ({ children }) => children };`,
    '/shell/appearance/Backdrop.tsx': 'export default () => null;',
    '/shell/Settings.tsx': `export default function Settings() {} export function SettingsNav() {}
      export const settingsSections = [{ id: 'folders', label: 'Folders' }];`,
    '/dashboard/nativeClient.ts': 'export const nativeDashboardClient = {};',
    '/dashboard/browserClient.ts': 'export const browserDashboardClient = {};',
    '/dashboard/replayLibrary.ts': 'export const useReplayLibrary = () => ({});',
    '/dashboard/contentRevision.ts': 'export const contentRevision = () => ({ observe() {} });',
    '/shell/diagnosticsSetting.ts': 'export const useDiagnosticsShortcut = () => {};',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'shell-interaction-fixture', enforce: 'pre',
      transform(source, id) {
        const name = id.replaceAll('\\', '/');
        if (name.endsWith('/shell/App.tsx')) return source.replace(/import \{[^\n]+\} from 'react';/,
          `const { useState, useRef, useCallback, useMemo, useEffect, useLayoutEffect, useSyncExternalStore } = globalThis.__shellFixture;
           const memo = value => value;`);
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
      const showSidebar = () => nodes!.find(node => node.props['aria-label'] === 'Show sidebar');
      assert.ok(showSidebar(), 'a hidden sidebar can be shown again without the shortcut');
      find(node => node.props.className === 'sidebar-edge').props.onPointerEnter(); render();
      assert.equal(mapVisible, true, 'the campaign map stays shown beside the peeking sidebar');
      assert.equal(hasMapOverlay(), true, 'the peek overlay is the popover whose hole the sidebar shows through');
      find(node => node.props['aria-label'] === 'Keep sidebar open').props.onClick(); render();
      assert.equal(mapVisible, true);
      assert.equal(hasMapOverlay(), false, 'keeping the sidebar open removes its overlay');
      assert.equal(showSidebar(), undefined);

    } else if (scenario === 'retry') {
      const settle = () => new Promise(resolve => setImmediate(resolve));
      const retry = () => nodes.find(node => node.type === 'button' && [node.props.children].flat().includes('Try again'));
      const copy = () => nodes.find(node => node.props.className === 'error-copy');
      const shown = () => copy()?.props.children;
      const title = () => nodes.find(node => typeof node.props.title === 'string' && node.props.alert)?.props.title;
      fixture.openReplay = () => Promise.reject('Synthetic open refusal');
      dashboard().props.onPrimaryPath('elsewhere'); await settle(); render();
      assert.deepEqual([title(), shown(), copy()?.props['data-detail']], ['The map couldn’t open this campaign', 'Couldn’t open the recording.', 'Synthetic open refusal'],
        'the failure is named by what failed, its raw reason kept as detail');
      assert.equal(mapVisible, false, 'the recovery panel shows instead of the native map');
      dashboard().props.onMapError('seek', 'Synthetic seek refusal'); render();
      const opened: string[] = [];
      fixture.openReplay = async (path: string) => { opened.push(path); };
      retry()!.props.onClick(); render();
      assert.equal(shown(), 'Couldn’t open the recording.', 'a failure stays shown while its operation is sent again');
      assert.equal(dashboard().props.mapRetry, 1, 'the dashboard is asked to send its failed commands again');
      await settle(); render();
      assert.deepEqual(opened, ['elsewhere']);
      assert.deepEqual([title(), shown(), copy()?.props['data-detail']], ['The map stopped', 'Couldn’t move the map to this hour.', 'Synthetic seek refusal'],
        'a failed seek is the dashboard\'s to clear once its repeat succeeds, and a map that had been running says it stopped');
      dashboard().props.onMapError('seek', null); dashboard().props.onMapError('apply', 'The map stopped: synthetic panic'); render();
      assert.deepEqual([title(), shown(), copy()?.props['data-detail']], ['The map stopped', 'Couldn’t apply the last map change.', 'The map stopped: synthetic panic'],
        'a reason that only repeats the title gives way to the action’s own sentence');
      fixture.viewerPath = 'elsewhere';
      dashboard().props.onMapError('apply', null); render();
      assert.equal(retry(), undefined);
      assert.equal(mapVisible, true);
    } else if (scenario === 'switch') {
      const settle = () => new Promise(resolve => setImmediate(resolve));
      const shownTab = () => find(node => node.props.className === 'app-shell').props['data-tab'];
      const failure = () => nodes.find(node => node.props.className === 'shell-switch-failure')?.props;
      const views: boolean[] = [];
      fixture.selectNativeView = async (dashboard: boolean) => { views.push(dashboard); };
      navigation().props.onChoose(navigation().props.pages[0].id); await settle(); render();
      dashboard().props.onPrimaryPath('elsewhere'); render();
      assert.equal(shownTab(), 'analytics');
      fixture.openReplay = () => Promise.reject('Synthetic open refusal');
      find(node => node.props.id === 'tab-map').props.onClick(); await settle(); render();
      assert.equal(shownTab(), 'analytics');
      assert.deepEqual(views, [true, false, true], 'Analytics, still shown, takes its native view back from the map');
      assert.deepEqual([failure()?.message, failure()?.detail], ['Couldn’t show the campaign map.', 'Synthetic open refusal'], 'the view still shown says why, its reason kept as detail');
      const opened: string[] = [];
      fixture.openReplay = async (path: string) => { opened.push(path); };
      failure()!.onRetry(); render();
      assert.ok(failure(), 'the failure stays shown while the switch is made again');
      await settle(); render();
      assert.deepEqual([views.slice(3), opened, shownTab()], [[false], ['elsewhere'], 'map'], 'the whole switch is made again: the map, then its recording');
      assert.equal(failure(), undefined);
    } else if (scenario === 'narrow') {
      const viewport = globalThis.window as unknown as { innerWidth: number };
      const widths = () => find(node => node.props.className === 'app-shell').props.style;
      const rail = () => find(node => node.props.className === 'sidebar-rail');
      find(node => node.props['aria-label'] === 'Keep sidebar open').props.onClick(); render();
      rail().props.onKeyDown({ key: 'ArrowRight', shiftKey: true, preventDefault() {} }); render();
      assert.equal(widths()['--sidebar-width'], '280px');
      viewport.innerWidth = 600; render();
      assert.deepEqual([widths()['--sidebar-width'], widths()['--sidebar-peek-width']], ['200px', '200px'], 'the sidebar never narrows past its minimum');
      assert.equal(rail().props['aria-valuemax'], 200);
      viewport.innerWidth = 1200; render();
      assert.equal(widths()['--sidebar-width'], '280px', 'a wider window shows the chosen width again');
    } else {
      navigation().props.onPages([...navigation().props.pages, { id: 'page-added', label: 'Added' }]); render();
      assert.equal(dashboard().props.pages, navigation().props.pages, 'the dashboard knows which pages exist, so a deleted page drops its kept draft');
      const notice = warning();
      assert.ok(notice, 'failed saves warn in the active view');
      assert.equal(notice.props.role, 'status');
      const noticeText = (node: any) => node.props.children[1].props.children;
      assert.equal(notice.props.children[0].props.name, 'alert', 'the notice leads with the alert icon');
      assert.match(noticeText(notice), /Couldn’t save your pages/);
      assert.ok(navigation().props.pages.some((entry: any) => entry.id === 'page-added'), 'session changes still apply');
      navigation().props.onPages(navigation().props.pages.map((entry: any) => entry.id === 'page-added' ? { ...entry, label: 'Renamed' } : entry)); render();
      assert.equal(noticeText(warning()), noticeText(notice), 'failed rename saves also warn');
      find(node => node.props.id === 'tab-settings').props.onClick();
      await new Promise(resolve => setImmediate(resolve)); render();
      assert.equal(find(node => node.props.className === 'app-shell').props['data-tab'], 'settings');
      assert.equal(find(node => 'onResetPages' in node.props).props.onResetPages, null);
      await vite.ssrLoadModule('/ui/src/widgets/definitions.ts'); render();
      assert.equal(typeof find(node => 'onResetPages' in node.props).props.onResetPages, 'function', 'an added page differs from the defaults');
      assert.equal(noticeText(warning()), noticeText(notice), 'changing tabs cannot clear the storage warning');
      find(node => 'onResetPages' in node.props).props.onResetPages(); render();
      assert.equal(noticeText(warning()), noticeText(notice), 'failed reset saves also warn');
      assert.equal(find(node => 'onResetPages' in node.props).props.onResetPages, undefined, 'default pages and layouts offer no reset');
      dashboard().props.onUnsaved(['overview']); render();
      assert.equal(typeof find(node => 'onResetPages' in node.props).props.onResetPages, 'function', 'an unsaved layout edit is not the default');
      dashboard().props.onUnsaved([]); render();
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
