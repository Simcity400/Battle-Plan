import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { LogListProps } from './LogList';

test('log windows settle when an overscanned row adds a horizontal scrollbar', async () => {
  type Effect = { deps: readonly unknown[] | undefined; run: () => void | (() => void); cleanup?: () => void };
  const slots = new Map<number, unknown>(), layouts = new Map<number, Effect>(), effects = new Map<number, Effect>();
  const pendingLayouts = new Map<number, Effect>(), pendingEffects = new Map<number, Effect>();
  let cursor = 0, dirty = true;
  const changed = (before: readonly unknown[] | undefined, after: readonly unknown[] | undefined) => !before || !after
    || before.length !== after.length || after.some((value, index) => !Object.is(value, before[index]));
  const fixture = {
    useState(initial: unknown) {
      const index = cursor++;
      let state = slots.get(index) as { value: unknown; set: (next: unknown) => void } | undefined;
      if (!state) {
        state = { value: typeof initial === 'function' ? initial() : initial, set: next => {
          const held = slots.get(index) as { value: unknown };
          const value = typeof next === 'function' ? next(held.value) : next;
          if (!Object.is(value, held.value)) { held.value = value; dirty = true; }
        } };
        slots.set(index, state);
      }
      return [state.value, state.set];
    },
    useRef(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, { current: initial });
      return slots.get(index) as { current: unknown };
    },
    useId() { return fixture.useRef(`log-${cursor}`).current; },
    useMemo(build: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, previous = slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (changed(previous?.deps, deps)) slots.set(index, { deps: [...deps], value: build() });
      return (slots.get(index) as { value: unknown }).value;
    },
    useCallback(callback: unknown, deps: readonly unknown[]) { return fixture.useMemo(() => callback, deps); },
    useLayoutEffect(run: Effect['run'], deps?: readonly unknown[]) {
      const index = cursor++;
      if (changed(layouts.get(index)?.deps, deps)) pendingLayouts.set(index, { deps: deps && [...deps], run });
    },
    useEffect(run: Effect['run'], deps?: readonly unknown[]) {
      const index = cursor++;
      if (changed(effects.get(index)?.deps, deps)) pendingEffects.set(index, { deps: deps && [...deps], run });
    },
  };
  const commit = (held: Map<number, Effect>, pending: Map<number, Effect>) => {
    const next = [...pending]; pending.clear();
    for (const [index] of next) held.get(index)?.cleanup?.();
    for (const [index, effect] of next) {
      const cleanup = effect.run();
      held.set(index, { ...effect, cleanup: typeof cleanup === 'function' ? cleanup : undefined });
    }
  };
  type Node = { props: Record<string, unknown> };
  const descendants = (value: unknown): Node[] => Array.isArray(value) ? value.flatMap(descendants)
    : value && typeof value === 'object' && 'props' in value ? [value as Node, ...descendants((value as Node).props.children)] : [];
  let nodes: Node[] = [], outerHeight = 120;
  const scroller = { scrollTop: 0, clientWidth: 390,
    get clientHeight() { return outerHeight - (nodes.some(node => node.props['data-log-key'] === '9') ? 10 : 0); },
    get offsetHeight() { return outerHeight; },
    addEventListener() {}, removeEventListener() {}, querySelector() { return null; },
  };
  const observers = new Set<Observer>();
  let observedHeight = -1;
  class Observer {
    readonly update: () => void;
    constructor(update: () => void) { this.update = update; }
    observe() { observers.add(this); }
    disconnect() { observers.delete(this); }
  }
  const host = globalThis as typeof globalThis & { __logMotionHooks?: typeof fixture };
  host.__logMotionHooks = fixture;
  const globals = ['window', 'ResizeObserver', 'requestAnimationFrame', 'cancelAnimationFrame'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { setTimeout: () => 0, clearTimeout() {} } });
  Object.defineProperty(globalThis, 'ResizeObserver', { configurable: true, value: Observer });
  const frames: (() => void)[] = [];
  Object.defineProperty(globalThis, 'requestAnimationFrame', { configurable: true, value: (frame: () => void) => frames.push(frame) });
  Object.defineProperty(globalThis, 'cancelAnimationFrame', { configurable: true, value: () => {} });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'log-motion-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/log/LogList.tsx') || path.endsWith('/charts/useRowMotion.ts') || path.endsWith('/dashboard/widgetPerf.ts')) {
          return source.replace(/import \{ ([^}]+) \} from 'react';/, "import * as React from 'react'; const { $1 } = { ...React, ...globalThis.__logMotionHooks };");
        }
        if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open() {}, menu: null });';
        if (path.endsWith('/charts/useChartTokens.ts')) return `export const useViewportHeight = () => 1000;
          export const useChartTokens = () => ({ font: 'Synthetic', fontSize: { small: 12, body: 16, title: 14 }, reducedMotion: false });`;
      },
    }] });
  try {
    const { default: LogList } = await vite.ssrLoadModule('/ui/src/charts/log/LogList.tsx') as typeof import('./LogList.tsx');
    let props: LogListProps<string> = { rows: Array.from({ length: 20 }, (_, index) => ({ key: String(index), hour: 100 - index, item: String(index) })),
      label: 'Synthetic recovery log', density: 'comfortable', months: false, animate: true,
      renderRow: row => row.item, describe: row => row.item };
    let attached: unknown;
    const settle = () => {
      for (let commits = 0; commits < 52; commits++) {
        dirty = false; cursor = 0; nodes = descendants(LogList(props));
        const list = nodes.find(node => node.props.role === 'listbox')!;
        if (attached !== list.props.ref) { attached = list.props.ref; (attached as (node: unknown) => void)(scroller); }
        commit(layouts, pendingLayouts); commit(effects, pendingEffects);
        for (const frame of frames.splice(0)) frame();
        if (dirty) continue;
        if (observedHeight !== scroller.clientHeight) {
          observedHeight = scroller.clientHeight;
          for (const observer of observers) observer.update();
          for (const frame of frames.splice(0)) frame();
          if (dirty) continue;
        }
        return;
      }
      assert.fail('LogList viewport effects exceeded the React update-depth limit instead of settling');
    };
    settle();
    assert.equal(scroller.clientHeight, 110);
    assert.equal(nodes.filter(node => node.props.role === 'option').length, 10);
    assert.equal(observers.size, 1, 'the viewport keeps one observer through measured height updates');
    outerHeight = 400;
    for (const observer of observers) observer.update();
    settle();
    assert.equal(nodes.filter(node => node.props.role === 'option').length, 17, 'real outer resizes still update row demand');
    outerHeight = 120;
    for (const observer of observers) observer.update();
    settle();
    const list = nodes.find(node => node.props.role === 'listbox')!;
    (list.props.onKeyDown as (event: unknown) => void)({ key: 'End', preventDefault() {} });
    assert.equal(scroller.scrollTop, 650, 'keyboard navigation uses the usable client height, excluding the scrollbar');
    settle();
    assert.equal(scroller.scrollTop, 760 - scroller.clientHeight, 'the destination remains visible after its scrollbar settles');
    assert.ok(nodes.some(node => node.props['data-log-key'] === '19' && node.props['aria-selected']));
    props = { ...props, rows: [{ key: 'new', hour: 101, item: 'New row' }, ...props.rows!] };
    settle();
    const scrolling = nodes.find(node => node.props.role === 'listbox')!;
    (scrolling.props.onWheel as (event: unknown) => void)({ currentTarget: scroller, deltaY: -760 });
    scroller.scrollTop = 0;
    (scrolling.props.onScroll as (event: unknown) => void)({ currentTarget: scroller });
    settle();
    assert.match(String(nodes.find(node => node.props['data-log-key'] === 'new')?.props.className), /is-entering/);
  } finally {
    for (const effect of [...layouts.values(), ...effects.values()]) effect.cleanup?.();
    for (const frame of frames.splice(0)) frame();
    await vite.close(); delete host.__logMotionHooks;
    for (const [key, descriptor] of globals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
  }
});
