import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

const RUNTIME = { clock: { getHour: () => 0 }, tracks: [] } as never;
const drillOf = (open: boolean, close: () => void = () => {}) => ({ open, following: false, outside: null, gate: { stage: { stage: 'open' }, country: null }, follow() {}, pause() {}, close,
  region: { widget: { trackIds: [], time: { mode: 'follow' } }, action: { kind: 'spec' } } }) as never;

test('actual queued drill close restores its own focus and preserves a later widget or button', async () => {
  class SurfaceElement {
    isConnected = true;
    readonly popout: SurfaceElement | null;
    constructor(popout: SurfaceElement | null = null) { this.popout = popout; }
    closest(selector: string) { return selector === '[popover]' || selector === '.drill-popout' ? this.popout : null; }
    contains(_target: unknown) { return false; }
    readonly listeners = new Map<string, (event: unknown) => void>();
    focuses = 0;
    addEventListener(name: string, callback: (event: unknown) => void) { this.listeners.set(name, callback); }
    removeEventListener(name: string) { this.listeners.delete(name); }
    focus() { this.focuses++; }
  }
  const body = new SurfaceElement(), panel = new SurfaceElement();
  const ownControl = new SurfaceElement(panel), anchorControl = new SurfaceElement();
  panel.contains = target => target === ownControl;
  const handlers = new Map<string, (event: unknown) => void>(), queued: FrameRequestCallback[] = [];
  const browser = { body, activeElement: ownControl as SurfaceElement | null,
    addEventListener: (name: string, callback: (event: unknown) => void) => handlers.set(name, callback),
    removeEventListener: (name: string) => handlers.delete(name) };
  const state = { effects: [] as (() => unknown)[], close: undefined as (() => void) | undefined,
    menu: { popup: { current: panel as SurfaceElement | null }, openAt() {} } };
  const host = globalThis as typeof globalThis & { __drillFocus?: typeof state; __drillFocusEffect?: (effect: () => unknown) => void };
  const before = { document: globalThis.document, window: globalThis.window, Element: globalThis.Element,
    requestAnimationFrame: globalThis.requestAnimationFrame, MutationObserver: globalThis.MutationObserver };
  const mutations: (() => void)[] = [];
  globalThis.MutationObserver = class { constructor(callback: () => void) { mutations.push(callback); } observe() {} disconnect() {} } as unknown as typeof MutationObserver;
  host.__drillFocus = state; host.__drillFocusEffect = effect => { state.effects.push(effect); };
  globalThis.document = browser as unknown as Document;
  globalThis.window = browser as unknown as Window & typeof globalThis;
  globalThis.Element = SurfaceElement as unknown as typeof Element;
  globalThis.requestAnimationFrame = callback => { queued.push(callback); return queued.length; };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'drill-focus-surface', enforce: 'pre',
      transform(code, id) {
        if (!id.replaceAll('\\', '/').endsWith('/drill/DrillHost.tsx') && !id.replaceAll('\\', '/').endsWith('/host/DetailWindowHost.tsx')) return;
        return code.replace("from '../../components/Popover'", "from 'drill-focus-surface'")
          .replace(/\buse(?:Layout)?Effect\(/g, 'globalThis.__drillFocusEffect(')
          .replaceAll('onClick={onClose}', 'onClick={(globalThis.__drillFocus.close = onClose)}');
      },
      resolveId(id) { if (id === 'drill-focus-surface') return '\0drill-focus-surface'; },
      load(id) { if (id === '\0drill-focus-surface') return `import { createElement } from 'react';
        export const usePopover = () => globalThis.__drillFocus.menu;
        export const Popover = props => createElement('div', null, props.children);`; },
    }] });
  try {
    const { default: DrillHost } = await vite.ssrLoadModule('/ui/src/charts/drill/DrillHost.tsx') as typeof import('./DrillHost');
    let closes = 0;
    const cleanups: (() => void)[] = [];
    const target = { isConnected: true, contains: (focused: unknown) => focused === anchorControl } as unknown as HTMLElement;
    const anchor = { current: target as HTMLElement | null };
    state.effects = []; browser.activeElement = anchorControl;
    const html = renderToStaticMarkup(createElement(DrillHost, { drill: drillOf(true), runtime: RUNTIME, title: 'Recorded parts', day: 'Recorded day',
      canFollow: false, anchor, children: null, onClose() { closes++; } }));
    assert.match(html, /Back/);
    try {
      for (const effect of state.effects) { const cleanup = effect(); if (typeof cleanup === 'function') cleanups.push(cleanup as () => void); }
      handlers.get('keydown')!({ key: 'Escape' });
      assert.equal(closes, 1);
      state.close!();
      assert.equal(closes, 2);
    } finally { for (const cleanup of cleanups.reverse()) cleanup(); }

    const { DetailWindowHost } = await vite.ssrLoadModule('/ui/src/families/host/DetailWindowHost.tsx') as typeof import('../../families/host/DetailWindowHost');
    let paused = 0, resumed = 0, closed = 0, windowFocuses = 0;
    const clock = { isPlaying: () => true, pause: () => { paused++; }, play: () => { resumed++; } };
    const openerEl = { isConnected: true, focus: (options: FocusOptions) => { assert.equal(options.preventScroll, true); windowFocuses++; } } as unknown as HTMLElement;
    state.effects = []; state.menu.popup.current = panel; browser.activeElement = ownControl;
    const winHtml = renderToStaticMarkup(createElement(DetailWindowHost, {
      open: true,
      subject: 'battle:99',
      hour: 60_800_000,
      title: 'Battle of Kursk',
      pauseClock: true,
      clock,
      opener: { current: openerEl },
      onClose() { closed++; state.menu.popup.current = null; },
      children: createElement('p', null, 'Battle details'),
    }));
    assert.match(winHtml, /Battle of Kursk/);
    assert.match(winHtml, /Close window/);
    const winCleanups: (() => void)[] = [];
    for (const effect of state.effects) { const c = effect(); if (typeof c === 'function') winCleanups.push(c as () => void); }
    assert.equal(paused, 1, 'pauseClock pauses playing clock on open');
    const focusesBefore = panel.focuses;
    panel.listeners.get('focusin')!({ target: ownControl });
    ownControl.isConnected = false;
    browser.activeElement = body;
    for (const mutation of mutations) mutation();
    assert.equal(panel.focuses, focusesBefore + 1, 'removing the focused control returns focus to the window so Escape still closes it');
    ownControl.isConnected = true;
    browser.activeElement = ownControl;
    handlers.get('keydown')!({ key: 'Escape', preventDefault() {} });
    assert.equal(closed, 1);
    browser.activeElement = body;
    queued.shift()!(0);
    assert.equal(windowFocuses, 1, 'DetailWindowHost restores opener focus on close');
    for (const cleanup of winCleanups.reverse()) cleanup();
    assert.equal(resumed, 1, 'pauseClock resumes clock when closed');
  } finally {
    globalThis.document = before.document; globalThis.window = before.window; globalThis.Element = before.Element;
    globalThis.requestAnimationFrame = before.requestAnimationFrame;
    globalThis.MutationObserver = before.MutationObserver;
    delete host.__drillFocus; delete host.__drillFocusEffect; await vite.close();
  }
});

