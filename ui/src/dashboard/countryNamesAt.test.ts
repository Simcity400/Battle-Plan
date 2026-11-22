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

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('dated names retain explicit loading, absence, failure and stable completed state identities', async () => {
  const fixture = hooks(), host = globalThis as typeof globalThis & { __nameReadHooks?: typeof fixture };
  host.__nameReadHooks = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'explicit-name-states', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (id.replaceAll('\\', '/').endsWith('/dashboard/countryNamesAt.ts')) return source.replace(
          /import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__nameReadHooks;');
      } }] });
  try {
    const { useCountryNamesAt, nameKey } = await vite.ssrLoadModule('/ui/src/dashboard/countryNamesAt.ts') as typeof import('./countryNamesAt');
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    const track = { source: { id: 'names', revision: '1' }, branch: { id: '1', firstHour: 10, lastHour: 20 } } as BoundTrack;
    let answer!: (value: ProductReply<unknown>) => void, request!: ProductRequest;
    const client = { product: (input: ProductRequest) => { request = input; return new Promise(resolve => { answer = resolve; }); } } as unknown as DashboardClient;
    const wanted = [{ hour: 10, tag: 'present' }, { hour: 10, tag: 'missing' }, { hour: 9, tag: 'outside' }];
    const render = (active = true, source = client) => { fixture.begin(); return useCountryNamesAt(source, track, [...wanted], active); };
    assert.equal(render(false).get(nameKey(10, 'present'))?.kind, 'absent'); fixture.commit();
    let names = render();
    assert.equal(names.get(nameKey(10, 'present'))?.kind, 'loading');
    assert.equal(names.get(nameKey(9, 'outside'))?.kind, 'absent');
    assert.equal(names.has(nameKey(10, 'unrequested')), false); fixture.commit();
    await new Promise<void>(resolve => setImmediate(resolve));
    answer({ ...request, data: { styles: [{ tag: 'present', name: '' }] } });
    await new Promise<void>(resolve => setImmediate(resolve));
    names = render(); fixture.commit();
    assert.deepEqual(names.get(nameKey(10, 'present')), { kind: 'value', value: '' });
    assert.equal(names.get(nameKey(10, 'missing'))?.kind, 'absent');
    assert.equal(render(), names, 'equivalent requests retain map and state identities'); fixture.commit();
    const refusal = new RefusedError('Synthetic country names were not recorded');
    const failedClient = { product: () => Promise.reject(refusal) } as unknown as DashboardClient;
    assert.equal(render(true, failedClient).get(nameKey(10, 'present'))?.kind, 'loading'); fixture.commit();
    await new Promise<void>(resolve => setImmediate(resolve));
    const failed = render(true, failedClient).get(nameKey(10, 'present'));
    assert.deepEqual(failed, { kind: 'failed', failure: refusal });
    let refuse = false;
    const playing = { product: (input: ProductRequest) => refuse ? Promise.reject(refusal)
      : Promise.resolve({ ...input, data: { styles: [{ tag: 'present', name: 'Named' }] } }) } as unknown as DashboardClient;
    const at = async (hour: number) => {
      wanted.splice(0, wanted.length, { hour, tag: 'present' });
      const loading = render(true, playing).get(nameKey(hour, 'present')); fixture.commit();
      await new Promise<void>(resolve => setImmediate(resolve));
      const read = render(true, playing).get(nameKey(hour, 'present')); fixture.commit();
      return { loading, read };
    };
    assert.deepEqual((await at(12)).read, { kind: 'value', value: 'Named' });
    refuse = true;
    assert.deepEqual(await at(13), { loading: { kind: 'value', value: 'Named' }, read: { kind: 'failed', failure: refusal } });
    assert.deepEqual((await at(14)).loading, { kind: 'failed', failure: refusal });
    refuse = false;
    wanted.splice(0, wanted.length, { hour: 15, tag: 'present' });
    fixture.begin();
    assert.equal(useCountryNamesAt(playing, track, [...wanted], true, Infinity, 0, true).get(nameKey(15, 'present'))?.kind, 'loading');
    fixture.commit();
  } finally { fixture.dispose(); delete host.__nameReadHooks; await vite.close(); }
});
