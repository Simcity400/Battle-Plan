import assert from 'node:assert/strict';
import test from 'node:test';
import * as React from 'react';
import { forgetProducts, loadProduct, useProduct, useProducts } from './products.ts';
import { contentRevision } from './contentRevision.ts';
import type { DashboardClient, DashboardSource, ProductRequest } from './client.ts';

const source = { id: 'progressive', revision: '1' } as DashboardSource;
const request = { sourceId: source.id, revision: source.revision, product: 'party-history', params: { branchId: '1' } };
const partial = { coverage: { firstHour: 0, lastHour: 9, complete: false }, rows: ['prefix'] };
const complete = { coverage: { firstHour: 0, lastHour: 19, complete: true }, approximate: false, rows: ['prefix', 'tail'] };
const flush = () => new Promise<void>(resolve => setImmediate(resolve));
function gate<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

/** A small hook renderer installs React's dispatcher, while exercising the real
 * useProduct, cache and query code. No transport or polling module is replaced. */
function mount<T>(read: () => T) {
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
    useEffect(effect: () => (() => void) | undefined, deps: readonly unknown[]) {
      const slot = slots[cursor++] ??= {};
      if (!slot.deps || slot.deps.length !== deps.length || deps.some((value, index) => !Object.is(value, slot.deps![index]))) {
        slot.deps = [...deps];
        effects.push(() => { slot.cleanup?.(); slot.cleanup = effect(); });
      }
    },
    useSyncExternalStore(_subscribe: unknown, snapshot: () => unknown) { cursor++; return snapshot(); },
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

test('mounted prepared useProduct publishes a prefix then replaces it once and caches only completion', { timeout: 2000 }, async () => {
  const status = gate<void>();
  let queries = 0, statuses = 0, validations = 0;
  const client = {
    preparedQuery: async () => { assert.ok(++queries <= 2, 'one initial query and one coverage update'); return queries === 1 ? partial : complete; },
    preparationStatus: async (input: object) => {
      assert.equal(++statuses, 1); await status.promise;
      return { ...input, state: 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null, coverage: { '1': complete.coverage } };
    },
    product: async () => { throw new Error('legacy product route'); },
  } as unknown as DashboardClient;
  const validate = (data: unknown) => { validations++; return (data as typeof partial).rows; };
  const hook = mount(() => useProduct(client, { ...request, params: { branchId: '1' } }, true, validate, source));
  try {
    hook.render(); await flush();
    assert.equal(hook.render().data, partial.rows, 'partial data renders while Status is pending');
    assert.equal(queries, 1, 'a recreated request with the same key starts no effect cycle');
    status.resolve(); await flush();
    assert.equal(hook.render().data, complete.rows);
    assert.equal(validations, 2, 'each publication is validated once');
    assert.equal(await loadProduct(client, request, new AbortController().signal, validate, source), complete.rows);
    assert.deepEqual({ queries, statuses }, { queries: 2, statuses: 1 }, 'final data is shared without more requests');
  } finally { hook.unmount(); status.resolve(); }
});

test('one-shot prepared loads return the first reply but never retain incomplete or empty coverage', async () => {
  for (const coverage of [partial.coverage, { firstHour: 0, lastHour: -1, complete: false }]) {
    let queries = 0;
    const client = { preparedQuery: async () => {
      assert.ok(++queries <= 3, 'two partial one-shot reads and one final read');
      return queries < 3 ? { ...partial, coverage, rows: [String(queries)] } : complete;
    }, preparationStatus: async () => { throw new Error('one-shot partial data must not poll'); } } as unknown as DashboardClient;
    // The validator deliberately removes coverage: retention must inspect the native reply.
    const validate = (data: unknown) => (data as typeof partial).rows;
    assert.deepEqual(await loadProduct(client, request, new AbortController().signal, validate, source), ['1']);
    assert.deepEqual(await loadProduct(client, request, new AbortController().signal, validate, source), ['2']);
    assert.equal(await loadProduct(client, request, new AbortController().signal, validate, source), complete.rows);
    assert.equal(await loadProduct(client, request, new AbortController().signal, validate, source), complete.rows);
    assert.equal(queries, 3);
  }
});

test('mounted preparation failure preserves its partial publication without caching it', async () => {
  let queries = 0;
  const client = { preparedQuery: async () => { assert.ok(++queries <= 3); return queries < 3 ? partial : complete; },
    preparationStatus: async (input: object) => ({ ...input, state: 'failed', sections: [], backgroundEnabled: true, bytes: '0', error: 'synthetic preparation failure' }),
  } as unknown as DashboardClient;
  const hook = mount(() => useProduct(client, request, true, undefined, source));
  try {
    hook.render(); await flush();
    assert.equal(hook.render().data, partial);
    assert.equal(hook.render().error, 'synthetic preparation failure');
    assert.equal(await loadProduct(client, request, new AbortController().signal, undefined, source), complete);
    assert.equal(queries, 3, 'failed partial was never installed in the cache');
  } finally { hook.unmount(); }
});

test('mounted product content changes cancel old coverage polling and reject late publications', async () => {
  const status = gate<void>();
  let queries = 0, statuses = 0, oldSignal!: AbortSignal;
  const client = { preparedQuery: async (_source: unknown, _product: string, _params: unknown, options: { signal: AbortSignal }) => {
    assert.ok(++queries <= 2);
    if (queries === 1) oldSignal = options.signal;
    return queries === 1 ? partial : complete;
  }, preparationStatus: async (input: object) => {
    assert.equal(++statuses, 1); await status.promise;
    return { ...input, state: 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null, coverage: { '1': complete.coverage } };
  } } as unknown as DashboardClient;
  const hook = mount(() => useProduct(client, request, true, undefined, source));
  try {
    hook.render(); await flush(); assert.equal(hook.render().data, partial);
    contentRevision(client).observe({ game: 'new content', user: 'recorded stack', contentReady: true });
    assert.equal(hook.render().data, undefined, 'content owner masks the old reply before new data arrives');
    await flush(); assert.equal(oldSignal.aborted, true);
    assert.equal(hook.render().data, complete);
    status.resolve(); await flush(); assert.equal(hook.render().data, complete);
    assert.deepEqual({ queries, statuses }, { queries: 2, statuses: 1 });
  } finally { hook.unmount(); status.resolve(); }
});

test('mounted useProducts publishes all initial countries before replacing a prepared extent', async () => {
  const status = gate<void>();
  let queries = 0, statuses = 0;
  const client = { preparedQuery: async () => { assert.ok(++queries <= 2); return queries === 1 ? partial : complete; },
    preparationStatus: async (input: object) => {
      assert.equal(++statuses, 1); await status.promise;
      return { ...input, state: 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null, coverage: { '1': complete.coverage } };
    }, product: async () => { throw new Error('prepared requests require their source'); },
  } as unknown as DashboardClient;
  const hook = mount(() => useProducts(client, [{ ...request } as ProductRequest], true, undefined, source));
  try {
    hook.render(); await flush(); assert.deepEqual(hook.render().data, [partial]);
    status.resolve(); await flush(); assert.deepEqual(hook.render().data, [complete]);
    assert.deepEqual({ queries, statuses }, { queries: 2, statuses: 1 });
  } finally { hook.unmount(); status.resolve(); }
});


test('source release cancels a mounted prepared read through the shared cache lifetime', async () => {
  let queries = 0, statuses = 0, nativeSignal!: AbortSignal;
  const client = { preparedQuery: async (_source: unknown, _product: string, _params: unknown, options: { signal: AbortSignal }) => {
    assert.equal(++queries, 1); nativeSignal = options.signal; return partial;
  }, preparationStatus: async (_input: object, signal: AbortSignal) => {
    assert.equal(++statuses, 1);
    return new Promise((_resolve, reject) => signal.addEventListener('abort',
      () => reject(new DOMException('source released', 'AbortError')), { once: true }));
  } } as unknown as DashboardClient;
  const hook = mount(() => useProduct(client, request, true, undefined, source));
  try {
    hook.render(); await flush(); assert.equal(hook.render().data, partial);
    forgetProducts(client, source.id);
    assert.equal(nativeSignal.aborted, true);
    await flush();
    assert.equal(hook.render().data, partial, 'source teardown does not invent or cache a final reply');
    assert.equal(hook.render().error, 'source released');
    assert.deepEqual({ queries, statuses }, { queries: 1, statuses: 1 });
  } finally { hook.unmount(); }
});

test('a mounted partial reply stays private while one-shot readers reach newer data independently', async () => {
  const status = gate<void>();
  const newer = { ...partial, rows: ['new one-shot prefix'] };
  let queries = 0, statuses = 0;
  const client = { preparedQuery: async () => {
    assert.ok(++queries <= 3, 'one mounted initial read, one one-shot read, one mounted coverage update');
    return queries === 1 ? partial : queries === 2 ? newer : complete;
  }, preparationStatus: async (input: object) => {
    assert.equal(++statuses, 1); await status.promise;
    return { ...input, state: 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null, coverage: { '1': complete.coverage } };
  } } as unknown as DashboardClient;
  const hook = mount(() => useProduct(client, request, true, undefined, source));
  try {
    hook.render(); await flush(); assert.equal(hook.render().data, partial);
    assert.equal(await loadProduct(client, request, new AbortController().signal, undefined, source), newer,
      'the mounted prefix is not a cached result for other readers');
    assert.equal(hook.render().data, partial, 'one-shot data cannot replace the mounted owner');
    status.resolve(); await flush(); assert.equal(hook.render().data, complete);
    assert.equal(await loadProduct(client, request, new AbortController().signal, undefined, source), complete);
    assert.deepEqual({ queries, statuses }, { queries: 3, statuses: 1 });
  } finally { hook.unmount(); status.resolve(); }
});


test('caching mounted completion keeps concurrent one-shot work reachable by source cancellation', async () => {
  const status = gate<void>();
  let queries = 0, statuses = 0, pendingSignal!: AbortSignal;
  const client = { preparedQuery: async (_source: unknown, _product: string, _params: unknown, options: { signal: AbortSignal }) => {
    assert.ok(++queries <= 3);
    if (queries === 2) {
      pendingSignal = options.signal;
      return new Promise((_resolve, reject) => pendingSignal.addEventListener('abort',
        () => reject(new DOMException('source released', 'AbortError')), { once: true }));
    }
    return queries === 1 ? partial : complete;
  }, preparationStatus: async (input: object) => {
    assert.equal(++statuses, 1); await status.promise;
    return { ...input, state: 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null, coverage: { '1': complete.coverage } };
  } } as unknown as DashboardClient;
  const hook = mount(() => useProduct(client, request, true, undefined, source));
  try {
    hook.render(); await flush(); assert.equal(hook.render().data, partial);
    const pending = loadProduct(client, request, new AbortController().signal, undefined, source);
    const stopped = assert.rejects(pending, { name: 'AbortError' });
    await flush();
    status.resolve(); await flush(); assert.equal(hook.render().data, complete);
    assert.equal(await loadProduct(client, request, new AbortController().signal, undefined, source), complete);
    forgetProducts(client, source.id); assert.equal(pendingSignal.aborted, true);
    await stopped;
    assert.deepEqual({ queries, statuses }, { queries: 3, statuses: 1 });
  } finally { hook.unmount(); status.resolve(); forgetProducts(client, source.id); }
});

const unfinishedReplies = [
  { coverage: null },
  { coverage: null, pending: [{ firstHour: 0, lastHour: 19 }] },
  { pending: [{ firstHour: 0, lastHour: 19 }] },
  { approximate: [{ firstHour: 0, lastHour: 19 }] },
];

test('one-shot replies with null coverage or unfinished spans never enter the final cache', async () => {
  for (const metadata of unfinishedReplies) {
    let reads = 0;
    const client = { preparedQuery: async () => ++reads < 3 ? { ...metadata, rows: [String(reads)] } : complete } as unknown as DashboardClient;
    const validate = (reply: unknown) => (reply as typeof partial).rows;
    for (const expected of [['1'], ['2'], complete.rows, complete.rows])
      assert.deepEqual(await loadProduct(client, request, new AbortController().signal, validate, source), expected);
    assert.equal(reads, 3);
  }
});

test('mounted replies with null coverage or unfinished spans poll through completion', async () => {
  for (const metadata of unfinishedReplies) {
    let reads = 0;
    const publications: unknown[] = [];
    const unfinished = { ...metadata, rows: ['unprepared'] };
    const client = { preparedQuery: async () => ++reads === 1 ? unfinished : complete,
      preparationStatus: async (input: object) => ({ ...input, state: 'ready', sections: [], backgroundEnabled: true,
        bytes: '0', error: null, coverage: { '1': complete.coverage } }),
    } as unknown as DashboardClient;
    assert.equal(await loadProduct(client, request, new AbortController().signal, undefined, source, false,
      data => publications.push(data)), complete);
    assert.deepEqual(publications, [unfinished, complete]);
    assert.equal(reads, 2);
  }
});
