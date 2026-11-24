import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { ProductRequest } from '../../dashboard/client';

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

test('two widgets on one page each draw their own new sprites through the one shared art cache', async () => {
  const requests: string[][] = [];
  const hooks = [widgetHooks(), widgetHooks()];
  let current = hooks[0]!;
  const fixture = {
    useState: (initial: any) => current.useState(initial), useRef: (initial: any) => current.useRef(initial),
    useMemo: (factory: () => any, deps: any[]) => current.useMemo(factory, deps), useCallback: (fn: any, deps: any[]) => current.useCallback(fn, deps),
    useEffect: (effect: () => any, deps: any[]) => current.useEffect(effect, deps),
    async product(_client: unknown, request: ProductRequest) {
      const sprites = (request.params as { sprites: { key: string; frame: number; text?: boolean }[] }).sprites;
      requests.push(sprites.map(sprite => sprite.key));
      await new Promise(resolve => setImmediate(resolve));
      return { items: sprites.map(sprite => ({ ...sprite, icon: `art://${sprite.key}`, resolution: { status: 'ready', assetId: sprite.key, url: `art://${sprite.key}`,
        origin: { kind: 'baseGame' }, file: '', frame: { frameIndex: 0, frameCount: 1, frameWidth: 1, frameHeight: 1, textureWidth: 1, textureHeight: 1, outputWidth: 1, outputHeight: 1 },
        provenance: { fromCurrentInstall: true, label: 'Current installation', scopeKey: 's' } } })), warnings: [] };
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
    const { useArtHandleResolutions } = await vite.ssrLoadModule('/ui/src/families/art/react.ts');
    const runtime = { client: {} };
    const track = { source: { id: 's', revision: 'r' }, branch: { id: '1' } };
    const sprite = (key: string) => ({ kind: 'sprite', key, frame: 0, text: true });
    const asks = [['icons_mods', 'pp_a', 'pp_b'].map(sprite), ['icons_mods', 'cp_a', 'cp_b', 'cp_c'].map(sprite)];
    const shown: any[] = [undefined, undefined];
    const pass = (at: number) => {
      current = hooks[at]!;
      shown[at] = current.render(() => useArtHandleResolutions(runtime, track, true));
      current.commit();
    };
    pass(0); pass(1);
    shown[0].onArtDemands(asks[0]);
    shown[1].onArtDemands(asks[1]);
    pass(0); pass(1);
    shown[1].onArtDemands(asks[1].slice(1));
    for (let round = 0; round < 8; round++) {
      pass(0); pass(1);
      await new Promise(resolve => setImmediate(resolve));
    }
    const statuses = (at: number) => Object.values(shown[at].resolutions ?? {}).map((art: any) => art.status);
    assert.deepEqual(statuses(0), ['ready', 'ready', 'ready'], `first widget: ${JSON.stringify(requests)}`);
    assert.deepEqual(statuses(1), ['ready', 'ready', 'ready', 'ready'], `the second widget's own sprites load too: ${JSON.stringify(requests)}`);
  } finally {
    await vite.close();
    delete host.__artWidgets;
  }
});
