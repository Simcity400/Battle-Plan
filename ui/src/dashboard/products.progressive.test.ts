import assert from 'node:assert/strict';
import test from 'node:test';
import * as React from 'react';
import { forgetProducts, loadProduct, useProduct, useProducts } from './products.ts';
import { contentRevision } from './contentRevision.ts';
import { HeldFailures } from './heldFailures.ts';
import type { DashboardClient, DashboardSource, ProductRequest } from './client.ts';
import { productRequest } from './request.ts';

const source = { id: 'progressive', revision: '1' } as DashboardSource;
const request = productRequest({ sourceId: source.id, revision: source.revision, product: 'party-history', params: { branchId: '1' } });
const complete = { rows: ['whole'] };

const flush = () => new Promise<void>(resolve => setImmediate(resolve));
function gate<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

function mount<T>(read: () => T, frame: HeldFailures | null = null) {
  type Slot = { value?: unknown; initialized?: boolean; deps?: readonly unknown[]; cleanup?: () => void };
  const slots: Slot[] = [];
  let cursor = 0, effects: (() => void)[] = [];
  const internals = (React as unknown as { __CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE: { H: unknown } })
    .__CLIENT_INTERNALS_DO_NOT_USE_OR_WARN_USERS_THEY_CANNOT_UPGRADE;
  const dispatcher = {
    useState(initial: unknown) {
      const slot = slots[cursor++] ??= {};
      if (!slot.initialized) { slot.value = typeof initial === 'function' ? initial() : initial; slot.initialized = true; }
      return [slot.value, (next: unknown) => { slot.value = typeof next === 'function' ? next(slot.value) : next; }];
    },
    useRef(initial: unknown) {
      const slot = slots[cursor++] ??= {};
      if (!slot.initialized) { slot.value = { current: initial }; slot.initialized = true; }
      return slot.value;
    },
    useEffect(effect: () => (() => void) | undefined, deps: readonly unknown[]) {
      const slot = slots[cursor++] ??= {};
      if (!slot.deps || slot.deps.length !== deps.length || deps.some((value, index) => !Object.is(value, slot.deps![index]))) {
        slot.deps = [...deps];
        effects.push(() => { slot.cleanup?.(); slot.cleanup = effect(); });
      }
    },
    useSyncExternalStore(_subscribe: unknown, snapshot: () => unknown) { cursor++; return snapshot(); },
    useContext() { return frame; },
  };
  return {
    render() {
      cursor = 0; effects = [];
      const previous = internals.H;
      internals.H = dispatcher;
      let result: T;
      try { result = read(); } finally { internals.H = previous; }
      for (const effect of effects) effect();
      return result!;
    },
    unmount() { for (const slot of slots) slot?.cleanup?.(); },
  };
}

test('mounted useProduct publishes its reply once, validated once, and the reply is shared without another read', async () => {
  const answer = gate<void>();
  let reads = 0, validations = 0;
  const client = { product: async (asked: ProductRequest) => { reads++; await answer.promise; return { ...asked, data: complete }; } } as unknown as DashboardClient;
  const validate = (data: unknown) => { validations++; return (data as typeof complete).rows; };
  const hook = mount(() => useProduct(client, { ...request, params: { branchId: '1' } }, true, validate, source));
  try {
    hook.render(); await flush();
    assert.equal(hook.render().data, undefined, 'nothing is shown before the reply');
    assert.equal(reads, 1, 'a recreated request with the same key starts no effect cycle');
    answer.resolve(); await flush();
    assert.equal(hook.render().data, complete.rows);
    assert.equal(await loadProduct(client, request, new AbortController().signal, validate, source), complete.rows);
    assert.deepEqual({ reads, validations }, { reads: 1, validations: 1 }, 'the reply is shared, and validated once');
  } finally { hook.unmount(); answer.resolve(); }
});

test('in a widget frame a failure with nothing published stays the body\'s own error', async () => {
  const client = { product: async () => { throw new Error('synthetic query failure'); } } as unknown as DashboardClient;
  const frame = new HeldFailures();
  const hook = mount(() => useProducts(client, [{ ...request, params: { branchId: 'unpublished' } } as ProductRequest], true, undefined, source), frame);
  try {
    hook.render(); await flush();
    const failed = hook.render();
    assert.equal(failed.data, undefined);
    assert.equal(failed.error?.message, 'synthetic query failure');
    assert.deepEqual(frame.read().map(error => error.message), [], 'nothing is said twice');
  } finally { hook.unmount(); }
});

test('mounted product content changes mask the old reply, cancel the read for content gone and never show its late reply', async () => {
  const reads: { signal: AbortSignal; answer: (data: unknown) => void }[] = [];
  const client = { product: (at: ProductRequest, signal: AbortSignal) => new Promise(resolve => {
    reads.push({ signal, answer: data => resolve({ ...at, data }) });
  }) } as unknown as DashboardClient;
  const hook = mount(() => useProduct(client, request, true, undefined, source));
  const shown = { rows: ['old content'] }, stale = { rows: ['content gone'] }, current = { rows: ['new content'] };
  try {
    hook.render(); await flush();
    reads[0].answer(shown); await flush();
    assert.equal(hook.render().data, shown);
    contentRevision(client).observe({ game: 'new content', user: 'recorded stack', contentReady: true });
    assert.equal(hook.render().data, undefined, 'the content owner masks the old reply before new data arrives');
    await flush();
    assert.equal(reads.length, 2);
    contentRevision(client).observe({ game: 'newer content', user: 'recorded stack', contentReady: true });
    hook.render(); await flush();
    assert.equal(reads[1].signal.aborted, true, 'the read for content gone is cancelled');
    reads[1].answer(stale); await flush();
    assert.equal(hook.render().data, undefined, 'its late reply is never shown');
    reads[2].answer(current); await flush();
    assert.equal(hook.render().data, current);
    assert.equal(reads.length, 3);
  } finally { hook.unmount(); }
});

test('source release cancels a mounted read through the shared cache lifetime', async () => {
  let pageSignal!: AbortSignal;
  const client = { product: async (_at: ProductRequest, signal: AbortSignal) => {
    pageSignal = signal;
    return new Promise((_resolve, reject) => signal.addEventListener('abort',
      () => reject(new DOMException('source released', 'AbortError')), { once: true }));
  } } as unknown as DashboardClient;
  const hook = mount(() => useProduct(client, { ...request, params: { branchId: 'released' } }, true, undefined, source));
  try {
    hook.render(); await flush();
    forgetProducts(client, source.id);
    assert.equal(pageSignal.aborted, true);
  } finally { hook.unmount(); }
});
