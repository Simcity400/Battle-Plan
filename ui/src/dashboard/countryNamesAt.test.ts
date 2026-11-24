import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient, ProductReply, ProductRequest } from './client';
import type { BoundTrack } from './model';

type Effect = { deps?: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
function hooks() {
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  let cursor = 0;
  const same = (a?: readonly unknown[], b?: readonly unknown[]) => a && b && a.length === b.length && a.every((item, i) => Object.is(item, b[i]));
  const state = {
    renders: 0,
    begin() { cursor = 0; },
    useRef(initial: unknown) { const index = cursor++; if (!slots.has(index)) slots.set(index, { current: initial }); return slots.get(index); },
    useState(initial: unknown) {
      const index = cursor++; if (!slots.has(index)) slots.set(index, initial);
      return [slots.get(index), (next: unknown) => { slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next); state.renders++; }];
    },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, before = slots.get(index) as { deps: readonly unknown[]; value: unknown } | undefined;
      if (!before || !same(before.deps, deps)) slots.set(index, { deps, value: make() });
      return (slots.get(index) as { value: unknown }).value;
    },
    useEffect(run: Effect['run'], deps?: readonly unknown[]) {
      const index = cursor++; if (!same(effects.get(index)?.deps, deps)) pending.set(index, { run, deps });
    },
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

test('dated fact hooks own independent reads and mask switched clients immediately', async () => {
  const host = globalThis as typeof globalThis & { __datedFactHooks?: ReturnType<typeof hooks> };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'dated-fact-owner-fixture', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (id.replaceAll('\\', '/').endsWith('/dashboard/countryNamesAt.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, (_match, names: string) => names.split(',').map(name =>
            `const ${name.trim()} = (...args) => globalThis.__datedFactHooks.${name.trim()}(...args);`).join('\n'));
      } }] });
  try {
    const { useCountryFactsAt, useCountryFactsRead, nameKey } = await vite.ssrLoadModule('/ui/src/dashboard/countryNamesAt.ts') as typeof import('./countryNamesAt.ts');
    const track = { source: { id: 'synthetic', revision: '1' }, branch: { id: '1' } } as BoundTrack;
    for (const hook of [useCountryFactsAt, useCountryFactsRead]) {
      const a = hooks(), b = hooks();
      const reads: { signal: AbortSignal; answer: () => void }[] = [];
      const client = { product: (request: ProductRequest, signal: AbortSignal) => new Promise<ProductReply<unknown>>(resolve => {
        reads.push({ signal, answer: () => resolve({ ...request, data: {} }) });
      }) } as unknown as DashboardClient;
      const wanted = [{ hour: 100, tag: 'A' }];
      const spec = { product: 'country-facts' as const, perRequest: 64, params: () => ({}), extract: () => new Map([['A', 'Recorded name']]) };
      host.__datedFactHooks = a;
      a.begin(); assert.equal(hook(client, track, wanted, true, spec).size, 0); a.commit();
      host.__datedFactHooks = b;
      b.begin(); assert.equal(hook(client, track, wanted, true, spec).size, 0); b.commit();
      assert.equal(reads.length, 2, 'each hook owns its request');
      a.dispose();
      assert.equal(reads[0].signal.aborted, true);
      assert.equal(reads[1].signal.aborted, false, 'departing hooks cannot cancel another owner');
      reads[1].answer(); await new Promise<void>(resolve => setImmediate(resolve));
      assert.ok(b.renders > 0);
      b.begin(); assert.equal(hook(client, track, wanted, true, spec).get(nameKey(100, 'A')), 'Recorded name'); b.commit();
      const nextClient = { product: (request: ProductRequest) => Promise.resolve({ ...request, data: {} }) } as unknown as DashboardClient;
      host.__datedFactHooks = b;
      b.begin(); assert.equal(hook(nextClient, track, wanted, true, spec).size, 0, 'client departure masks settled facts before effects commit');
      b.commit(); await new Promise<void>(resolve => setImmediate(resolve));
      b.begin(); assert.equal(hook(nextClient, track, wanted, true, spec).get(nameKey(100, 'A')), 'Recorded name'); b.commit();
      b.dispose();
    }
  } finally { delete host.__datedFactHooks; await vite.close(); }
});

test('useCountryNamesAt never borrows another hour’s name while a renamed or reused tag’s hour is still loading', async () => {
  const host = globalThis as typeof globalThis & { __datedFactHooks?: ReturnType<typeof hooks> };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'dated-names-no-borrow', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (path.endsWith('/dashboard/countrySlots.ts')) return 'export const useSlotTags = () => undefined;';
        if (path.endsWith('/dashboard/countryNamesAt.ts'))
          return source.replace(/import \{ ([^}]+) \} from 'react';/, (_match, names: string) => names.split(',').map(name =>
            `const ${name.trim()} = (...args) => globalThis.__datedFactHooks.${name.trim()}(...args);`).join('\n'));
      } }] });
  try {
    const { useCountryNamesAt, nameKey } = await vite.ssrLoadModule('/ui/src/dashboard/countryNamesAt.ts') as typeof import('./countryNamesAt.ts');
    const state = hooks();
    host.__datedFactHooks = state;
    const track = { source: { id: 'rename-source', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 300, gaps: [] } } as unknown as BoundTrack;
    const pending = new Map<number, () => void>();
    const client = {
      product: (request: ProductRequest) => {
        const hour = (request.params as { hour?: number }).hour;
        if (request.product === 'country-styles' && hour === 200) {
          return Promise.resolve({ ...request, data: { styles: [{ tag: 'GER', name: 'Later Name', color: null }], warnings: [] } });
        }
        return new Promise<ProductReply<unknown>>(resolve => {
          if (hour !== undefined) pending.set(hour, () => resolve({ ...request, data: { styles: [{ tag: 'GER', name: 'Earlier Name', color: null }], warnings: [] } }));
        });
      },
    } as unknown as DashboardClient;

    state.begin(); useCountryNamesAt(client, track, [{ hour: 200, tag: 'GER' }], true); state.commit();
    await new Promise<void>(resolve => setImmediate(resolve));
    state.begin();
    const at200 = useCountryNamesAt(client, track, [{ hour: 200, tag: 'GER' }], true);
    state.commit();
    assert.deepEqual(at200.get(nameKey(200, 'GER')), { kind: 'value', value: 'Later Name' });

    state.begin();
    const at100Pending = useCountryNamesAt(client, track, [{ hour: 100, tag: 'GER' }], true);
    state.commit();
    assert.deepEqual(at100Pending.get(nameKey(100, 'GER')), { kind: 'loading' }, 'must not borrow hour 200 name for pending hour 100');

    pending.get(100)?.();
    await new Promise<void>(resolve => setImmediate(resolve));
    state.begin();
    const at100Settled = useCountryNamesAt(client, track, [{ hour: 100, tag: 'GER' }], true);
    state.commit();
    assert.deepEqual(at100Settled.get(nameKey(100, 'GER')), { kind: 'value', value: 'Earlier Name' });
    state.dispose();
  } finally { delete host.__datedFactHooks; await vite.close(); }
});

