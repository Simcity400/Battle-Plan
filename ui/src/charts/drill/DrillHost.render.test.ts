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
  }
  const body = new SurfaceElement(), panel = new SurfaceElement(), otherPanel = new SurfaceElement();
  const ownControl = new SurfaceElement(panel), otherControl = new SurfaceElement(otherPanel), button = new SurfaceElement();
  const anchorControl = new SurfaceElement(), disconnected = new SurfaceElement(); disconnected.isConnected = false;
  panel.contains = target => target === ownControl;
  const handlers = new Map<string, (event: unknown) => void>(), queued: FrameRequestCallback[] = [];
  const browser = { body, activeElement: ownControl as SurfaceElement | null,
    addEventListener: (name: string, callback: (event: unknown) => void) => handlers.set(name, callback),
    removeEventListener: (name: string) => handlers.delete(name) };
  const state = { effects: [] as (() => unknown)[], close: undefined as (() => void) | undefined,
    toggle: undefined as ((event: { newState: string }) => void) | undefined,
    menu: { popup: { current: panel as SurfaceElement | null }, openAt() {} } };
  const host = globalThis as typeof globalThis & { __drillFocus?: typeof state; __drillFocusEffect?: (effect: () => unknown) => void };
  const before = { document: globalThis.document, window: globalThis.window, Element: globalThis.Element,
    requestAnimationFrame: globalThis.requestAnimationFrame };
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
          .replaceAll('onClick={requestClose}', 'onClick={(globalThis.__drillFocus.close = requestClose)}');
      },
      resolveId(id) { if (id === 'drill-focus-surface') return '\0drill-focus-surface'; },
      load(id) { if (id === '\0drill-focus-surface') return `import { createElement } from 'react';
        export const usePopover = () => globalThis.__drillFocus.menu;
        export const Popover = props => { globalThis.__drillFocus.toggle = props.onToggle; return createElement('div', null, props.children); };`; },
    }] });
  try {
    const { default: DrillHost } = await vite.ssrLoadModule('/ui/src/charts/drill/DrillHost.tsx') as typeof import('./DrillHost');
    for (const scenario of [
      { event: 'close', mode: 'popout', next: otherControl, restore: false },
      { event: 'outside', mode: 'popout', next: button, restore: false },
      { event: 'toggle', mode: 'popout', next: body, restore: true },
      { event: 'escape', mode: 'popout', next: ownControl, restore: true },
      { event: 'close', mode: 'popout', next: disconnected, restore: true },
      { event: 'close', mode: 'popout', next: null, restore: true },
      { event: 'close', mode: 'inline', next: anchorControl, restore: true },
      { event: 'close', mode: 'popout', next: body, restore: false, unmounted: true },
      { event: 'close', mode: 'popout', next: body, restore: false, detached: true },
      { event: 'close', mode: 'popout', next: body, restore: false, defaultFocus: true },
      { event: 'escape', mode: 'window', next: ownControl, restore: true },
      { event: 'close', mode: 'window', next: body, restore: true },
    ] as const) {
      let closes = 0, focuses = 0;
      const selected: string[] = [], cleanups: (() => void)[] = [];
      const target = { isConnected: true, contains: (focused: unknown) => focused === anchorControl,
        querySelector: (selector: string) => { selected.push(selector); return { focus: (options: FocusOptions) => {
          assert.equal(options.preventScroll, true); focuses++;
        } }; } } as unknown as HTMLElement;
      const anchor = { current: target as HTMLElement | null };
      state.effects = []; state.menu.popup.current = panel; browser.activeElement = scenario.mode === 'inline' ? anchorControl : ownControl;
      const html = renderToStaticMarkup(createElement(DrillHost, { mode: scenario.mode, drill: drillOf(true), runtime: RUNTIME, title: 'Recorded parts', day: 'Recorded day',
        canFollow: false, anchor, returnFocus: 'defaultFocus' in scenario ? undefined : '.country-race-chart canvas',
        main: null, children: null, onClose() { closes++; state.menu.popup.current = null; } }));
      assert.match(html, scenario.mode === 'inline' ? /Back/ : /Close breakdown/);
      try {
        for (const effect of state.effects) { const cleanup = effect(); if (typeof cleanup === 'function') cleanups.push(cleanup as () => void); }
        if (scenario.mode === 'popout') {
          browser.activeElement = otherControl;
          handlers.get('keydown')!({ key: 'Escape' });
          handlers.get('pointerdown')!({ target: otherControl });
          assert.equal(closes, 0, 'another popover keeps its own Escape and pointer events');
          browser.activeElement = ownControl;
        }
        if (scenario.mode === 'window') {
          assert.equal(handlers.has('pointerdown'), false, 'window mode stays open on outside pointerdown');
        }
        if (scenario.event === 'outside') handlers.get('pointerdown')!({ target: button });
        else if (scenario.event === 'escape') handlers.get('keydown')!({ key: 'Escape' });
        else if (scenario.event === 'toggle') state.toggle!({ newState: 'closed' });
        else state.close!();
        assert.equal(closes, 1);
        assert.equal(state.menu.popup.current, null, 'closing clears the live popup ref before the queued callback');
        if ('defaultFocus' in scenario) { assert.equal(queued.length, 0, 'callers without returnFocus retain their own close behavior'); continue; }
        assert.equal(queued.length, 1);
        if (scenario.mode === 'popout') {
          state.toggle!({ newState: 'closed' });
          assert.equal(closes, 1, 'a programmatic hide cannot close twice before the state render');
          assert.equal(queued.length, 1);
        }
        browser.activeElement = scenario.next;
        if ('unmounted' in scenario) anchor.current = null;
        if ('detached' in scenario) Object.assign(target, { isConnected: false });
        queued.shift()!(0);
        assert.equal(focuses, scenario.restore ? 1 : 0, `${scenario.event}: preserve the focus reached before RAF`);
        assert.deepEqual(selected, scenario.restore ? ['.country-race-chart canvas'] : []);
      } finally { for (const cleanup of cleanups.reverse()) cleanup(); }
    }

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
    delete host.__drillFocus; delete host.__drillFocusEffect; await vite.close();
  }
});

