import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { ProductRequest } from '../../dashboard/client';
import { artHandleKey } from './types.ts';

function widgetHooks() {
  const slots: any[] = [];
  let cursor = 0;
  let effects: (() => void)[] = [];
  const same = (a: any[], b: any[]) => a.length === b.length && a.every((value, index) => Object.is(value, b[index]));
  const useMemo = (factory: () => any, deps: any[]) => {
    const index = cursor++;
    const before = slots[index];
    if (!before || !same(deps, before.deps)) slots[index] = { deps, value: factory() };
    return slots[index].value;
  };
  return {
    useState(initial: any) {
      const index = cursor++;
      if (!(index in slots)) slots[index] = typeof initial === 'function' ? initial() : initial;
      return [slots[index], (value: any) => { slots[index] = typeof value === 'function' ? value(slots[index]) : value; }];
    },
    useRef(initial: any) { const index = cursor++; return slots[index] ??= { current: initial }; },
    useMemo,
    useCallback(fn: any, deps: any[]) { return useMemo(() => fn, deps); },
    useEffect(effect: () => any, deps: any[]) {
      const index = cursor++;
      const before = slots[index];
      if (!before || !same(deps, before.deps)) {
        const slot: { deps: any[]; cleanup?: () => void } = { deps };
        slots[index] = slot;
        effects.push(() => { before?.cleanup?.(); slot.cleanup = effect(); });
      }
    },
    render<T>(draw: () => T): T { cursor = 0; effects = []; return draw(); },
    commit() { for (const effect of effects.splice(0)) effect(); },
  };
}

type Hooks = ReturnType<typeof widgetHooks>;

const readySprite = (key: string) => ({ status: 'ready', assetId: key, url: `art://${key}`, origin: { kind: 'baseGame' }, file: '',
  frame: { frameIndex: 0, frameCount: 1, frameWidth: 1, frameHeight: 1, textureWidth: 1, textureHeight: 1, outputWidth: 1, outputHeight: 1 },
  provenance: { fromCurrentInstall: true, label: 'Current installation', scopeKey: 's' } });

interface Kit {
  readonly useArtHandleResolutions: any;
  readonly useArtResolutions: any;
  readonly use: (at: number) => Hooks;
  readonly requests: string[][];
  readonly release: () => void;
  readonly hold: (held: boolean) => void;
}

async function withArtWidgets(count: number, run: (kit: Kit) => Promise<void>) {
  const requests: string[][] = [];
  const hooks = Array.from({ length: count }, widgetHooks);
  let current = hooks[0]!;
  let held = false;
  const waiting: (() => void)[] = [];
  const fixture = {
    useState: (initial: any) => current.useState(initial), useRef: (initial: any) => current.useRef(initial),
    useMemo: (factory: () => any, deps: any[]) => current.useMemo(factory, deps), useCallback: (fn: any, deps: any[]) => current.useCallback(fn, deps),
    useEffect: (effect: () => any, deps: any[]) => current.useEffect(effect, deps),
    async product(_client: unknown, request: ProductRequest) {
      const params = request.params as { sprites?: { key: string; frame: number; text?: boolean }[]; references?: string[] };
      if (request.product === 'unit-leader-portraits') {
        return { portraits: (params.references ?? []).map(reference => ({ reference, image: null, missing: null, resolution: readySprite(reference) })), warnings: [] };
      }
      const sprites = params.sprites ?? [];
      requests.push(sprites.map(sprite => sprite.key));
      if (held) await new Promise<void>(resolve => waiting.push(resolve));
      else await new Promise(resolve => setImmediate(resolve));
      return { items: sprites.map(sprite => ({ ...sprite, icon: sprite.key.startsWith('fail_') ? null : `art://${sprite.key}`, resolution: sprite.key.startsWith('fail_') ? { status: 'failed', reason: 'the read was refused', retryable: false } : readySprite(sprite.key) })), warnings: [] };
    },
  };
  const host = globalThis as typeof globalThis & { __artWidgets?: typeof fixture };
  host.__artWidgets = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'art-widgets-fixture', enforce: 'pre',
      transform(source, id) {
        const file = id.replaceAll('\\', '/');
        if (file.endsWith('/families/art/react.ts')) return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__artWidgets;');
        if (file.endsWith('/families/identity/react.ts')) return 'export const useIdentityScope = () => null;';
        if (file.endsWith('/dashboard/products.ts')) return 'export const loadProduct = (...args) => globalThis.__artWidgets.product(...args); export const loadTransientProduct = (c, r) => globalThis.__artWidgets.product(c, r);';
        if (file.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (file.endsWith('/dashboard/frameUpdates.ts')) return 'export const inFrame = run => queueMicrotask(run);';
      },
    }] });
  try {
    const { useArtHandleResolutions, useArtResolutions } = await vite.ssrLoadModule('/ui/src/families/art/react.ts');
    await run({
      useArtHandleResolutions,
      useArtResolutions,
      use: at => (current = hooks[at]!),
      requests,
      release: () => { for (const resolve of waiting.splice(0)) resolve(); },
      hold: value => { held = value; },
    });
  } finally {
    await vite.close();
    delete host.__artWidgets;
  }
}

const sprite = (key: string) => ({ kind: 'sprite', key, frame: 0, text: true });
const tick = () => new Promise(resolve => setImmediate(resolve));

test('two widgets on one page each draw their own new sprites through the one shared art cache', () => withArtWidgets(2, async ({ useArtHandleResolutions, use, requests }) => {
  const runtime = { client: {} };
  const track = { source: { id: 's', revision: 'r' }, branch: { id: '1' } };
  const asks = [['icons_mods', 'pp_a', 'pp_b'].map(sprite), ['icons_mods', 'cp_a', 'cp_b', 'cp_c'].map(sprite)];
  const shown: any[] = [undefined, undefined];
  const pass = (at: number) => {
    const hooks = use(at);
    shown[at] = hooks.render(() => useArtHandleResolutions(runtime, track, true));
    hooks.commit();
  };
  pass(0); pass(1);
  shown[0].onArtDemands(asks[0]);
  shown[1].onArtDemands(asks[1]);
  pass(0); pass(1);
  shown[1].onArtDemands(asks[1]!);
  for (let round = 0; round < 8; round++) {
    pass(0); pass(1);
    await tick();
  }
  const statuses = (at: number) => Object.values(shown[at].resolutions ?? {}).map((art: any) => art.status);
  assert.deepEqual(statuses(0), ['ready', 'ready', 'ready'], `first widget: ${JSON.stringify(requests)}`);
  assert.deepEqual(statuses(1), ['ready', 'ready', 'ready', 'ready'], `the second widget's own sprites load too: ${JSON.stringify(requests)}`);
}));

test('a shared request that settles between a widget\'s render and its subscription is still seen', () => withArtWidgets(2, async ({ useArtResolutions, use, release, hold }) => {
  const runtime = { client: {} };
  const track = { source: { id: 's', revision: 'r' }, branch: { id: '1' } };
  const handles = [sprite('shared')];
  const resolve: any[] = [undefined, undefined];
  const render = (at: number) => { const hooks = use(at); resolve[at] = hooks.render(() => useArtResolutions(runtime, track, handles, true)); return hooks; };
  hold(true);
  render(0).commit();
  await tick();
  const second = render(1);
  const unsettled = resolve[1];
  release();
  for (let i = 0; i < 10; i++) await tick();
  second.commit();
  for (let i = 0; i < 4; i++) { render(1).commit(); await tick(); }
  assert.notEqual(resolve[1], unsettled, 'subscribing after the reply asks for a new render, so what was memoised on the old answer is redrawn');
  assert.equal(resolve[1](handles[0]).status, 'ready');
}));

test('a widget keeps exactly the art its projection read: 513 handles keep the first, a later projection replaces the set without rotating, another branch starts empty', () => withArtWidgets(1, async ({ useArtHandleResolutions, use }) => {
  const runtime = { client: {} };
  let track = { source: { id: 's', revision: 'r' }, branch: { id: '1' } };
  let shown: any;
  const pass = () => { const hooks = use(0); shown = hooks.render(() => useArtHandleResolutions(runtime, track, true)); hooks.commit(); };
  const keyOf = (handle: { kind: 'portrait'; leader: string; hour: number }) => artHandleKey(handle);
  pass();
  const read = Array.from({ length: 513 }, (_, at) => ({ kind: 'portrait' as const, leader: String(at), hour: 100 }));
  shown.onArtDemands(read);
  for (let i = 0; i < 8; i++) { pass(); await tick(); }
  assert.equal(Object.keys(shown.resolutions).length, 513, 'every handle the projection read is kept');
  assert.ok(keyOf(read[0]!) in shown.resolutions, 'the first handle, which may be the painted row, is never dropped');
  const later = read.slice(400);
  shown.onArtDemands(later);
  for (let i = 0; i < 6; i++) { pass(); await tick(); }
  assert.deepEqual(Object.keys(shown.resolutions).sort(), later.map(keyOf).sort(), 'a later projection replaces the set, so nothing rotates out of view');
  const before = shown.resolutions;
  pass();
  assert.strictEqual(shown.resolutions, before, 'a render with no change keeps the record, so the lane is not rebound');
  track = { ...track, branch: { id: '2' } };
  pass();
  assert.equal(shown.resolutions, undefined, 'another branch starts with no demand');
}));

test('a settled failure keeps its object, so another widget settling does not change this widget\'s record', () => withArtWidgets(2, async ({ useArtHandleResolutions, use }) => {
  const runtime = { client: {} };
  const track = { source: { id: 's', revision: 'r' }, branch: { id: '1' } };
  const shown: any[] = [undefined, undefined];
  const pass = (at: number) => { const hooks = use(at); shown[at] = hooks.render(() => useArtHandleResolutions(runtime, track, true)); hooks.commit(); };
  pass(0); pass(1);
  shown[0].onArtDemands([sprite('fail_a'), sprite('ok_a')]);
  for (let i = 0; i < 8; i++) { pass(0); pass(1); await tick(); }
  const record = shown[0].resolutions;
  assert.deepEqual(Object.values(record).map((art: any) => art.status).sort(), ['failed', 'ready']);
  shown[1].onArtDemands([sprite('ok_b')]);
  for (let i = 0; i < 8; i++) { pass(0); pass(1); await tick(); }
  assert.strictEqual(shown[0].resolutions, record, 'an unrelated settlement leaves the record, so the lane is not rebound');
}));

test('useArtResolutions keeps NUL-delimited handle keys unambiguous across pending demand switches', () => withArtWidgets(1, async ({ useArtResolutions, use, requests, hold, release }) => {
  const runtime = { client: {} };
  const track = { source: { id: 's', revision: 'r' }, branch: { id: '1' } };
  const second = { kind: 'sprite' as const, key: 'b', frame: 0 };
  const pair = [{ kind: 'sprite' as const, key: 'a', frame: 0 }, second];
  const merged = [{ kind: 'sprite' as const, key: `a\0${artHandleKey(second)}`, frame: 0 }];
  hold(true);
  const hooks = use(0);
  hooks.render(() => useArtResolutions(runtime, track, pair, true));
  hooks.commit();
  await tick();
  hooks.render(() => useArtResolutions(runtime, track, merged, true));
  hooks.commit();
  await tick();
  release();
  for (let i = 0; i < 6; i++) await tick();
  assert.ok(requests.some(keys => keys.includes(merged[0]!.key)), 'switching to a NUL-colliding sprite key while pending still fetches it');
}));
