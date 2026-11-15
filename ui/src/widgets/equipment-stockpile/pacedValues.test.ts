import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { Live } from '../../charts/live';

type Effect = { deps?: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
/** React's hooks for one component, rendered by hand: `renders` counts the renders its state asks for. */
function hooks() {
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  let cursor = 0;
  const same = (a?: readonly unknown[], b?: readonly unknown[]) => a && b && a.length === b.length && a.every((item, i) => Object.is(item, b[i]));
  const state = {
    renders: 0,
    begin() { cursor = 0; },
    useRef(initial: unknown) { const index = cursor++; if (!slots.has(index)) slots.set(index, { current: initial }); return slots.get(index); },
    useState(initial: unknown) {
      const index = cursor++; if (!slots.has(index)) slots.set(index, typeof initial === 'function' ? (initial as () => unknown)() : initial);
      return [slots.get(index), (next: unknown) => { slots.set(index, typeof next === 'function' ? (next as (value: unknown) => unknown)(slots.get(index)) : next); state.renders++; }];
    },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, before = slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (!before || !same(before.deps, deps)) slots.set(index, { deps, value: make() });
      return (slots.get(index) as { value: unknown }).value;
    },
    useEffect(run: Effect['run'], deps?: readonly unknown[]) { const index = cursor++; if (!same(effects.get(index)?.deps, deps)) pending.set(index, { run, deps }); },
    commit() {
      for (const [index, effect] of pending) {
        effects.get(index)?.cleanup?.(); const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined; effects.set(index, effect);
      }
      pending.clear();
    },
    dispose() { for (const effect of effects.values()) effect.cleanup?.(); },
  };
  return state;
}

test('paced values: a read giving a fresh but equal value renders nothing and hands every render the same value; a change renders once', async () => {
  const fixture = hooks(), host = globalThis as typeof globalThis & { __pacedHooks?: typeof fixture };
  host.__pacedHooks = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'paced-values-hooks', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/widgets/equipment-stockpile/useEquipmentStockpile.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__pacedHooks;');
      } }] });
  try {
    const { usePacedValues } = await vite.ssrLoadModule('/ui/src/widgets/equipment-stockpile/useEquipmentStockpile.ts') as typeof import('./useEquipmentStockpile');
    // Each read builds a new object holding the same values, as an empty reply's cells did.
    let content = 'none', tell = () => {};
    const live: Live<{ content: string }> = { read: () => ({ content }), subscribe: changed => { tell = changed; return () => { tell = () => {}; }; } };
    const render = () => { fixture.begin(); const value = usePacedValues(live, true); fixture.commit(); return value; };
    const first = render();
    assert.equal(render(), first, 'a render with nothing changed hands the same value');
    assert.equal(render(), first);
    assert.equal(fixture.renders, 0, 'reading equal values asks for no render');
    content = 'coal';
    tell();
    assert.equal(fixture.renders, 1, 'a change the source tells renders once');
    const changed = render();
    assert.deepEqual(changed, { content: 'coal' });
    assert.equal(render(), changed, 'and holds the new value after');
    assert.equal(fixture.renders, 1);
  } finally { fixture.dispose(); delete host.__pacedHooks; await vite.close(); }
});
