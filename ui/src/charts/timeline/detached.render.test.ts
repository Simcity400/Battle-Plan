import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';
import type { TimelineProps } from './Timeline';

test('a timeline unmounting answers resizes, scrolls and frames between its refs clearing and its cleanup', async () => {
  const state = { cursor: 0, slots: [] as unknown[], effects: [] as (() => unknown)[],
    frame: undefined as ((info: { now: number; dt: number }) => boolean) | undefined,
    viewport: { visible: { top: 0, height: 120 }, offset: 0 },
    tokens: { ...tokensFromValues(() => '#777777', 'sans-serif'), reducedMotion: true } };
  const hooks = {
    useId: () => 'timeline-detached',
    useMemo(build: () => unknown, deps: readonly unknown[]) {
      const index = state.cursor++, before = state.slots[index] as { deps: readonly unknown[]; value: unknown } | undefined;
      if (before && before.deps.length === deps.length && deps.every((value, i) => Object.is(value, before.deps[i]))) return before.value;
      const value = build(); state.slots[index] = { deps, value }; return value;
    },
    useRef(initial: unknown) { const index = state.cursor++; return state.slots[index] ??= { current: initial }; },
    useState(initial: unknown) { const index = state.cursor++; if (!(index in state.slots)) state.slots[index] = initial;
      return [state.slots[index], (next: unknown) => { state.slots[index] = typeof next === 'function' ? next(state.slots[index]) : next; }]; },
    useEffect(effect: () => unknown, deps?: readonly unknown[]) { hooks.useMemo(() => state.effects.push(effect), deps ?? [Symbol()]); },
  };
  const scheduler = { add: (task: { frame: typeof state.frame }) => { state.frame = task.frame; return () => {}; }, wake() {}, setVisible() {} };
  const host = globalThis as typeof globalThis & { __timelineDetached?: typeof state; __timelineDetachedHooks?: typeof hooks; __timelineDetachedScheduler?: typeof scheduler };
  host.__timelineDetached = state; host.__timelineDetachedHooks = hooks; host.__timelineDetachedScheduler = scheduler;
  const resized: (() => void)[] = [], scrolled: (() => void)[] = [];
  const globals = ['window', 'document', 'ResizeObserver'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { innerHeight: 120, devicePixelRatio: 1, addEventListener() {}, removeEventListener() {} } });
  Object.defineProperty(globalThis, 'document', { configurable: true, value: {
    addEventListener: (kind: string, listener: () => void) => { if (kind === 'scroll') scrolled.push(listener); }, removeEventListener() {} } });
  Object.defineProperty(globalThis, 'ResizeObserver', { configurable: true, value: class {
    constructor(changed: () => void) { resized.push(changed); } observe() {} disconnect() {} } });
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'timeline-detached', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        // The viewport is read from the elements it is given, never from cleared refs.
        if (path.endsWith('/charts/timeline/viewport.ts')) return `export * from '/ui/src/charts/timeline/viewport.ts?actual';
          export const timelineViewport = (spacer, scrolling) => { if (!spacer || !scrolling) throw new Error('read a cleared ref'); return globalThis.__timelineDetached.viewport; };`;
        if (path.endsWith('/charts/liveMotion.ts')) return 'export const useMotionNow = () => given => given;';
        if (path.endsWith('/charts/images.ts')) return 'export const canvasImageRead = () => () => null;';
        if (path.endsWith('/timeline/Timeline.tsx')) return source
          .replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__timelineDetachedHooks;')
          .replace("import { scheduler } from '../scheduler';", 'const scheduler = globalThis.__timelineDetachedScheduler;')
          .replace("import { readTokens, subscribeTokens } from '../tokens';", 'const readTokens = () => globalThis.__timelineDetached.tokens; const subscribeTokens = () => () => {};');
      },
    }] });
  const cleanups: (() => void)[] = [];
  try {
    const { default: Timeline } = await vite.ssrLoadModule('/ui/src/charts/timeline/Timeline.tsx') as typeof import('./Timeline.tsx');
    const context = new Proxy({ measureText: (text: string) => ({ width: text.length * 6 }) },
      { get: (target, key) => key in target ? target[key as keyof typeof target] : () => {} });
    const element = { style: {}, parentElement: null, isConnected: true, clientWidth: 600, clientHeight: 120, width: 600, height: 120,
      getContext: () => context, getBoundingClientRect: () => ({ left: 0, top: 0, width: 600, height: 120 }), addEventListener() {}, removeEventListener() {} };
    const props: TimelineProps = { label: 'Synthetic history', extent: [0, 10], playhead: 5, active: true, revision: 0, showIcons: false, showNames: true,
      lanes: [{ id: 'lane', label: 'Lane', spans: [{ id: 'span', label: 'Span', firstHour: 0, lastHour: 10 }] }] };
    state.cursor = 0; state.effects = [];
    Timeline(props);
    const refs = [0, 1, 2].map(index => state.slots[index] as { current: unknown });
    for (const ref of refs) ref.current = element;
    for (const effect of state.effects) { const cleanup = effect(); if (typeof cleanup === 'function') cleanups.push(cleanup as () => void); }
    assert.ok(resized.length && scrolled.length && state.frame, 'the effect observes, listens and schedules');
    // Unmounting: React clears the refs and detaches the nodes before this effect's cleanup runs.
    for (const ref of refs) ref.current = null;
    element.isConnected = false;
    for (const changed of resized) assert.doesNotThrow(changed, 'a resize');
    for (const listener of scrolled) assert.doesNotThrow(listener, 'a scroll');
    assert.doesNotThrow(() => state.frame!({ now: 0, dt: 16 }), 'a frame');
  } finally {
    for (const cleanup of cleanups.reverse()) cleanup();
    await vite.close(); delete host.__timelineDetached; delete host.__timelineDetachedHooks; delete host.__timelineDetachedScheduler;
    for (const [key, descriptor] of globals) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
  }
});
