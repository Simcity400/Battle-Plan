import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { DashboardClient, ProductReply, ProductRequest } from './client';
import type { ProductState } from './products';
import { productRequest } from './request.ts';

type Effect = { deps: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };
type Payload = { owner: string };
const request: ProductRequest = productRequest({ sourceId: 'same-source', revision: 'same-revision', product: 'country-fuel',
  params: { branchId: 'same-branch', tag: 'ONE' } });
const frames = new Map<number, FrameRequestCallback>();
let frameIds = 0;
const settle = async () => {
  await new Promise<void>(resolve => setImmediate(resolve));
  for (const [id, frame] of [...frames]) { frames.delete(id); frame(0); }
};

// quarantine: #3 re-enable in the dashboard rework; the issue names its failure
test.skip('actual product and window hooks scope ready, late and retried results to their client under identical keys', async () => {
  const states = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  const memos = new Map<number, { deps: readonly unknown[]; value: unknown }>();
  let cursor = 0, writes = 0;
  const animation = { request: globalThis.requestAnimationFrame, cancel: globalThis.cancelAnimationFrame };
  globalThis.requestAnimationFrame = callback => { frames.set(++frameIds, callback); return frameIds; };
  globalThis.cancelAnimationFrame = id => { frames.delete(id); };
  const fixture = {
    useState(initial?: unknown) {
      const index = cursor++;
      if (!states.has(index)) states.set(index, initial);
      return [states.get(index), (next: unknown) => { writes++; states.set(index, typeof next === 'function' ? next(states.get(index)) : next); }];
    },
    useRef(initial?: unknown) {
      const index = cursor++;
      if (!states.has(index)) states.set(index, { current: initial });
      return states.get(index);
    },
    useMemo(run: () => unknown, deps: readonly unknown[]) {
      const index = cursor++, before = memos.get(index);
      if (!before || before.deps.length !== deps.length || before.deps.some((value, i) => !Object.is(value, deps[i])))
        memos.set(index, { deps: [...deps], value: run() });
      return memos.get(index)!.value;
    },
    useEffect(run: Effect['run'], deps: readonly unknown[]) {
      const index = cursor++, before = effects.get(index);
      if (!before || before.deps.length !== deps.length || before.deps.some((value, i) => !Object.is(value, deps[i])))
        pending.set(index, { run, deps: [...deps] });
    },
    commit() {
      for (const [index, effect] of pending) {
        effects.get(index)?.cleanup?.();
        const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined;
        effects.set(index, effect);
      }
      pending.clear();
    },
    unmount() {
      for (const effect of effects.values()) effect.cleanup?.();
      effects.clear(); pending.clear(); states.clear(); memos.clear();
    },
  };
  const host = globalThis as typeof globalThis & { __productClientHooks?: typeof fixture };
  host.__productClientHooks = fixture;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'product-hook-client-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/products.ts') || path.endsWith('/dashboard/productWindows.ts')) {
          const hooked = source.replace(/import \{ ([^}]+) \} from 'react';/, 'const { $1 } = globalThis.__productClientHooks;');
          assert.notEqual(hooked, source, `${path} imports its hooks from React`);
          return hooked;
        }
        if (path.endsWith('/dashboard/heldFailures.ts')) return source.replace("import { createContext, useContext, useEffect, useRef, useState } from 'react';",
          "import { createContext } from 'react'; const { useEffect, useRef, useState } = globalThis.__productClientHooks; const useContext = () => null;");
        if (path.endsWith('/dashboard/contentRevision.ts')) return source.replace(
          'return useSyncExternalStore(revision.subscribe, revision.snapshot, revision.snapshot);', 'return revision.snapshot();');
      },
    }] });
  try {
    const hooks = await vite.ssrLoadModule('/ui/src/dashboard/products.ts') as typeof import('./products.ts');
    const windowHooks = await vite.ssrLoadModule('/ui/src/dashboard/productWindows.ts') as typeof import('./productWindows.ts');
    const transport = () => {
      const calls: { signal: AbortSignal; answer: (owner: string) => void; fail: () => void }[] = [];
      const client = { product: (input: ProductRequest, signal: AbortSignal) => new Promise<ProductReply<Payload>>((resolve, reject) => {
        calls.push({ signal, answer: owner => resolve({ ...input, data: { owner } }), fail: () => reject(new Error('Previous client failed')) });
      }) } as unknown as DashboardClient;
      return { client, calls };
    };
    for (const mode of ['single', 'batch', 'windows'] as const) {
      const requests = mode !== 'single' ? [request, { ...request, params: { branchId: 'same-branch', tag: 'TWO' } }] : [request];
      const render = (client: DashboardClient): ProductState<Payload | Payload[]> => {
        cursor = 0;
        if (mode === 'windows') {
          const windows = windowHooks.useProductWindows<Payload>(client, 'same-source-revision-branch', index => requests[index], [0, 1], true);
          return { data: windows.data.size ? [...windows.data.values()] : undefined,
            error: [...windows.errors.values()][0], retry: windows.retry };
        }
        return mode === 'batch' ? hooks.useProducts<Payload>(client, requests, true) : hooks.useProduct<Payload>(client, request, true);
      };
      const absent = (state: ProductState<Payload | Payload[]>) => { assert.equal(state.data, undefined); assert.equal(state.error, undefined); };
      const owners = (state: ProductState<Payload | Payload[]>) => (Array.isArray(state.data) ? state.data : state.data ? [state.data] : []).map(data => data.owner);
      for (const outcome of ['data', 'error']) {
        const a = transport(), b = transport();
        absent(render(a.client)); fixture.commit(); await settle();
        const before = writes;
        for (const call of a.calls) outcome === 'data' ? call.answer('A') : call.fail();
        await settle();
        if (mode === 'windows') assert.equal(writes - before, 1, 'windows landing in one frame commit together');
        const ready = render(a.client);
        if (outcome === 'data') assert.deepEqual(owners(ready), requests.map(() => 'A'));
        else assert.equal(ready.error?.message, 'Previous client failed');
        absent(render(b.client));
        fixture.commit(); await settle();
        assert.equal(b.calls.length, requests.length);
        absent(render(b.client));
        for (const call of b.calls) call.answer('B');
        await settle();
        assert.deepEqual(owners(render(b.client)), requests.map(() => 'B'));
        if (mode === 'windows') {
          ready.retry();
          assert.deepEqual(owners(render(b.client)), requests.map(() => 'B'));
          fixture.commit(); await settle();
          assert.equal(b.calls.length, requests.length);
        }
        fixture.unmount(); await settle();
      }
      for (const phase of ['before return', 'after return']) for (const outcome of ['data', 'error']) {
        const a = transport(), b = transport();
        const firstA = render(a.client); absent(firstA); fixture.commit(); await settle();
        absent(render(b.client)); fixture.commit(); await settle();
        assert.equal(a.calls.length, requests.length);
        assert.ok(a.calls.every(call => call.signal.aborted));
        const oldA = a.calls.slice();
        const deliverOldA = () => { for (const call of oldA) outcome === 'data' ? call.answer('Old A') : call.fail(); };
        if (phase === 'before return') { deliverOldA(); await settle(); absent(render(b.client)); }
        absent(render(a.client)); fixture.commit(); await settle();
        assert.equal(a.calls.length, requests.length * 2, 'A → B → A starts fresh work after cancellation');
        if (phase === 'after return') { deliverOldA(); await settle(); absent(render(a.client)); }
        for (const call of b.calls) call.fail();
        await settle(); absent(render(a.client));
        if (mode === 'windows') {
          firstA.retry();
          absent(render(a.client)); fixture.commit(); await settle();
          assert.equal(a.calls.length, requests.length * 2);
          assert.ok(a.calls.slice(requests.length).every(call => !call.signal.aborted));
        }
        for (const call of a.calls.slice(requests.length)) call.answer('Current A');
        await settle();
        assert.deepEqual(owners(render(a.client)), requests.map(() => 'Current A'));
        fixture.unmount(); await settle();
      }
    }
    const retrying = transport();
    cursor = 0;
    const retryWindows = () => {
      cursor = 0;
      return windowHooks.useProductWindows<Payload>(retrying.client, 'same-source-revision-branch', () => request, [0], true);
    };
    retryWindows(); fixture.commit(); await settle(); retrying.calls[0].fail(); await settle();
    const failed = retryWindows(); assert.equal(failed.errors.get(0)?.message, 'Previous client failed');
    failed.retry(); assert.equal(retryWindows().errors.size, 0); fixture.commit(); await settle();
    assert.equal(retrying.calls.length, 2, 'the current owner can retry its failed window');
    retrying.calls[1].answer('Retried'); await settle();
    assert.equal(retryWindows().data.get(0)?.owner, 'Retried'); fixture.unmount();
    const a = transport(), b = transport(), data = { owner: 'A' };
    const display = (client: DashboardClient, value: Payload | undefined, hour: number) => {
      cursor = 0;
      return windowHooks.useWindowDisplay('same-source-revision-branch', value, hour, client);
    };
    const first = display(a.client, data, 100); fixture.commit();
    assert.equal(display(a.client, undefined, 124), first, 'pending same-client data keeps its supplying hour');
    assert.equal(display(b.client, undefined, 124), null, 'another client never borrows the retained display'); fixture.commit();
    assert.equal(display(a.client, undefined, 124), null, 'returning to A does not restore A first-visit rows'); fixture.commit();
    assert.equal(display(a.client, { owner: 'Current A' }, 125)?.hour, 125);
  } finally {
    fixture.unmount(); await vite.close(); delete host.__productClientHooks;
    globalThis.requestAnimationFrame = animation.request; globalThis.cancelAnimationFrame = animation.cancel;
  }
});
