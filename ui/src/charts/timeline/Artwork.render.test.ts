import test from 'node:test';
import assert from 'node:assert/strict';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';
import type { TimelineProps } from './Timeline';

test('timeline draws neutral artwork in dated slots with scroll offsets and plot clipping', async () => {
  const state = { cursor: 0, slots: [] as unknown[], effects: [] as (() => unknown)[],
    frame: undefined as ((info: { now: number; dt: number }) => boolean) | undefined,
    viewport: { visible: { top: 200, height: 120 }, offset: 200 }, loaded: false,
    tokens: { ...tokensFromValues(() => '#777777', 'sans-serif'), reducedMotion: true }, texts: [] as string[], dashes: [] as number[][] };
  const hooks = {
    useId: () => 'timeline-art',
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
  const host = globalThis as typeof globalThis & { __timelineArt?: typeof state; __timelineArtHooks?: typeof hooks; __timelineArtScheduler?: typeof scheduler };
  host.__timelineArt = state; host.__timelineArtHooks = hooks; host.__timelineArtScheduler = scheduler;
  const globals = ['window', 'document', 'ResizeObserver', 'Image'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { innerHeight: 120, devicePixelRatio: 1, addEventListener() {}, removeEventListener() {} } });
  Object.defineProperty(globalThis, 'document', { configurable: true, value: { addEventListener() {}, removeEventListener() {} } });
  Object.defineProperty(globalThis, 'ResizeObserver', { configurable: true, value: class { observe() {} disconnect() {} } });
  Object.defineProperty(globalThis, 'Image', { configurable: true, value: class { src = ''; get complete() { return state.loaded; }
    get naturalWidth() { return state.loaded ? 20 : 0; } get naturalHeight() { return state.loaded ? 20 : 0; } } });
  const replacements: Record<string, string> = {
    '/charts/timeline/viewport.ts': `export * from '/ui/src/charts/timeline/viewport.ts?actual'; export const timelineViewport = () => globalThis.__timelineArt.viewport;`,
    '/charts/liveMotion.ts': 'export const useMotionNow = () => given => given;',
    '/charts/images.ts': `export const canvasImageRead = () => source => source && globalThis.__timelineArt.loaded ? {naturalWidth: 20, naturalHeight: 20} : null;`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'timeline-art-frame', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/timeline/Timeline.tsx')) return source
          .replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__timelineArtHooks;')
          .replace("import { scheduler } from '../scheduler';", 'const scheduler = globalThis.__timelineArtScheduler;')
          .replace("import { readTokens, subscribeTokens } from '../tokens';", 'const readTokens = () => globalThis.__timelineArt.tokens; const subscribeTokens = () => () => {};');
      },
    }] });
  const cleanups: (() => void)[] = [];
  try {
    const { default: Timeline } = await vite.ssrLoadModule('/ui/src/charts/timeline/Timeline.tsx') as typeof import('./Timeline.tsx');
    const context = new Proxy({ measureText: (text: string) => ({ width: text.length * 6 }),
      fillText: (text: string) => state.texts.push(text), setLineDash: (dash: number[]) => state.dashes.push(dash) },
    { get: (target, key) => key in target ? target[key as keyof typeof target] : () => {} });
    const element = { style: {}, parentElement: null, clientWidth: 600, clientHeight: 120, width: 600, height: 120,
      getContext: () => context, getBoundingClientRect: () => ({ left: 0, top: 0, width: 600, height: 120 }), addEventListener() {}, removeEventListener() {} };
    const props: TimelineProps = { label: 'Synthetic history', extent: [0, 10], playhead: 5, active: true, revision: 0, showIcons: true, showNames: true,
      lanes: Array.from({ length: 20 }, (_, index) => ({ id: `lane-${index}`, label: `Lane ${index}`, singleLine: true,
        ...(index === 4 ? { identity: { label: 'Project', tile: 'blueprint' as const, icon: 'synthetic.png' } } : {}),
        spans: index === 5 ? [{ id: 'mark', label: 'Event', firstHour: 0, lastHour: 0, marker: true }] : [] })) };
    const render = () => { state.cursor = 0; state.effects = []; return Timeline(props); };
    render();
    for (const index of [0, 1, 2]) (state.slots[index] as { current: unknown }).current = element;
    for (const effect of state.effects) { const cleanup = effect(); if (typeof cleanup === 'function') cleanups.push(cleanup as () => void); }
    state.frame!({ now: 0, dt: 500 });
    const html = renderToStaticMarkup(render());
    assert.match(html, /art-fallback/); assert.doesNotMatch(html, /\?/);
    type Tile = { props: { style: { top: number; left: number; clipPath: string }; children: { props: { icon: string } } } };
    const tiles = (render().props.children[0].props.children[1] as Tile[]);
    const blueprint = tiles.find(tile => tile.props.children.props.icon === 'flask')!;
    const mark = tiles.find(tile => tile.props.children.props.icon === 'sparkle')!;
    assert.ok(blueprint); assert.ok(mark);
    assert.ok(blueprint.props.style.top < 100, 'scrolled artwork is positioned within the sticky view');
    assert.match(mark.props.style.clipPath, / [1-9]\d*(?:\.\d+)?px\)$/, 'a marker centred at the plot edge clips its left half');
    assert.ok(state.texts.includes('Empty')); assert.equal(state.texts.includes('?'), false);
    assert.equal(state.dashes.some(dash => dash.length === 2 && dash[0] === 3 && dash[1] === 3), false, 'empty lanes have no dashed box');
    const oldTop = blueprint.props.style.top;
    state.viewport = { visible: { top: 220, height: 120 }, offset: 220 };
    state.frame!({ now: 500, dt: 500 });
    const scrolled = render().props.children[0].props.children[1] as Tile[];
    assert.equal(scrolled.find(tile => tile.props.children.props.icon === 'flask')!.props.style.top, oldTop - 20);
    state.loaded = true; state.frame!({ now: 1000, dt: 500 });
    const loaded = render().props.children[0].props.children[1] as Tile[];
    assert.equal(loaded.some(tile => tile.props.children.props.icon === 'flask'), false, 'loaded artwork replaces its fallback');
  } finally {
    for (const cleanup of cleanups.reverse()) cleanup();
    await vite.close(); delete host.__timelineArt; delete host.__timelineArtHooks; delete host.__timelineArtScheduler;
    for (const [key, descriptor] of globals) { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); }
  }
});
