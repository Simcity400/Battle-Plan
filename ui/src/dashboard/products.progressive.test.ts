import assert from 'node:assert/strict';
import test from 'node:test';
import * as React from 'react';
import { forgetProducts, loadProduct, useProduct, useProducts } from './products.ts';
import { contentRevision } from './contentRevision.ts';
import { HeldFailures } from './heldFailures.ts';
import { failureText, queryFailure } from './queryError.ts';
import { registerPageJoin } from './pageJoins.ts';
import type { DashboardClient, DashboardSource, ProductRequest } from './client.ts';

const source = { id: 'progressive', revision: '1' } as DashboardSource;
const request = { sourceId: source.id, revision: source.revision, product: 'party-history', params: { branchId: '1' } };
const complete = { rows: ['whole'] };

type Page = { rows: string[]; cursor: string | null };
registerPageJoin(['synthetic-pages'], pages => ({ rows: (pages as Page[]).flatMap(page => page.rows), cursor: null }));
const paged = (params: Record<string, unknown> = {}): ProductRequest => ({ sourceId: source.id, revision: source.revision, product: 'series-summary',
  params: { branchId: '1', family: { kind: 'synthetic-pages' }, ...params } });
const first: Page = { rows: ['first'], cursor: 'second' };
const second: Page = { rows: ['second'], cursor: null };
const cursorOf = (asked: ProductRequest) => (asked.params as { cursor?: string }).cursor;

const lost = 'BattlePlan couldn’t reach its desktop host for dashboard_product: the IPC request failed (TypeError: Failed to fetch).';
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

test('a mounted paged read failing after its first page keeps that page shown, says the failure, and keeps nothing that failed', async () => {
  const asked: (string | undefined)[] = [];
  let failing = true;
  const client = { product: async (at: ProductRequest) => {
    const cursor = cursorOf(at);
    asked.push(cursor);
    if (cursor === undefined) return { ...at, data: first };
    if (failing) throw new Error('synthetic page failure');
    return { ...at, data: second };
  } } as unknown as DashboardClient;
  const hook = mount(() => useProduct(client, paged(), true, undefined, source));
  try {
    hook.render(); await flush();
    const failed = hook.render();
    assert.equal(failed.data, first, 'the first page stays shown');
    assert.equal(failed.error?.message, 'synthetic page failure');
    failing = false;
    assert.deepEqual(await loadProduct(client, paged(), new AbortController().signal, undefined, source), { rows: ['first', 'second'], cursor: null });
    assert.deepEqual(asked, [undefined, 'second', 'second'], 'the first page is held; the page that failed is read again');
  } finally { hook.unmount(); }
});

test('in a widget frame a failure after a publication keeps the reply, said once by the frame by its name with its Retry', async () => {
  let reads = 0;
  const client = { product: async (at: ProductRequest) => {
    reads++;
    if (cursorOf(at) === undefined) return { ...at, data: first };
    throw queryFailure(lost);
  } } as unknown as DashboardClient;
  const frame = new HeldFailures();
  let heard = 0;
  const unsubscribe = frame.subscribe(() => { heard++; });
  const hook = mount(() => useProduct(client, paged({ framed: true }), true, undefined, source), frame);
  try {
    hook.render(); await flush();
    const failed = hook.render();
    assert.equal(failed.data, first, 'the published reply stays');
    assert.equal(failed.error, undefined, 'the body says an error only when nothing was published');
    assert.deepEqual(frame.read().map(error => error.message), [lost], 'the frame says the failure');
    assert.equal(failureText(frame.read()[0], 'Couldn’t load.'), 'BattlePlan couldn’t reach its desktop host for dashboard_product.', 'by its name');
    hook.render();
    assert.equal(heard, 1, 'the same failure is said once');
    const asked = reads;
    frame.retry(frame.read()[0]!);
    assert.equal(hook.render().data, undefined, 'Retry asks again');
    assert.deepEqual(frame.read().map(error => error.message), [], 'the notice leaves while the reply is asked again');
    await flush();
    assert.ok(reads > asked);
    assert.equal(hook.render().data, first);
    assert.deepEqual(frame.read().map(error => error.message), [lost]);
  } finally { hook.unmount(); unsubscribe(); }
  assert.deepEqual(frame.read().map(error => error.message), [], 'a reader that leaves takes its notice with it');
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

test('a request of several that failed stays said while another publishes again, until Retry', async () => {
  const accepted = gate<void>();
  const client = {
    product: async (at: ProductRequest) => {
      const tag = (at.params as { tag: string }).tag;
      if (cursorOf(at) === undefined) return { ...at, data: { rows: [`${tag} first`], cursor: 'second' } };
      if (tag === 'A') throw new Error('A stopped');
      await accepted.promise;
      return { ...at, data: { rows: [`${tag} second`], cursor: null } };
    },
  } as unknown as DashboardClient;
  const requests = ['A', 'B'].map(tag => paged({ tag }));
  const rows = (state: { data?: unknown[] }) => state.data?.map(reply => (reply as Page).rows.join(' + '));
  const frame = new HeldFailures();
  const hook = mount(() => useProducts(client, requests, true, undefined, source), frame);
  try {
    hook.render(); await flush();
    const stopped = hook.render();
    assert.deepEqual(rows(stopped), ['A first', 'B first'], 'both first pages were published');
    assert.equal(stopped.error, undefined);
    assert.deepEqual(frame.read().map(error => error.message), ['A stopped'], 'A stopped after publishing');
    accepted.resolve(); await flush();
    const later = hook.render();
    assert.deepEqual(rows(later), ['A first', 'B first + B second'], 'B publishes again');
    assert.equal(later.error, undefined);
    assert.deepEqual(frame.read().map(error => error.message), ['A stopped'], 'A is still stopped, and still said with its Retry');
    frame.retry(new Error('A stopped'));
    hook.render();
    assert.deepEqual(frame.read().map(error => error.message), [], 'Retry asks again');
  } finally { hook.unmount(); accepted.resolve(); }
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
  let reads = 0, pageSignal!: AbortSignal;
  const client = { product: async (at: ProductRequest, signal: AbortSignal) => {
    reads++;
    if (cursorOf(at) === undefined) return { ...at, data: first };
    pageSignal = signal;
    return new Promise((_resolve, reject) => signal.addEventListener('abort',
      () => reject(new DOMException('source released', 'AbortError')), { once: true }));
  } } as unknown as DashboardClient;
  const hook = mount(() => useProduct(client, paged({ released: true }), true, undefined, source));
  try {
    hook.render(); await flush(); assert.equal(hook.render().data, first);
    forgetProducts(client, source.id);
    assert.equal(pageSignal.aborted, true);
    await flush();
    const released = hook.render();
    assert.equal(released.data, first, 'source teardown does not invent or cache a final reply');
    assert.equal(released.error?.message, 'source released');
    assert.equal(reads, 2);
  } finally { hook.unmount(); }
});
