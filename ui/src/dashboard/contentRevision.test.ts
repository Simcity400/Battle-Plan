import assert from 'node:assert/strict';
import test from 'node:test';
import { ContentRevision, contentRevision } from './contentRevision.ts';
import { loadProduct, productKey } from './products.ts';
import { FlagStore, flagStore } from './flagCache.ts';
import type { DashboardClient, ProductRequest } from './client';
import { CONTENT_PRODUCTS } from '../generated/dashboard.ts';

test('folder and readiness changes invalidate content; repeated native status updates do not', () => {
  const revision = new ContentRevision();
  let changes = 0;
  const stop = revision.subscribe(() => changes++);
  const folders = { game: 'game', user: 'user', contentReady: false };
  revision.observe(null);
  assert.equal(revision.snapshot(), 0);
  revision.observe(folders);
  revision.observe({ ...folders });
  assert.equal(changes, 1);
  revision.observe({ ...folders, contentReady: true });
  revision.observe({ ...folders, contentReady: true, game: 'other' });
  revision.observe({ ...folders, contentReady: true, user: 'other' });
  assert.equal(changes, 4);
  assert.equal(revision.snapshot(), 4);
  stop();
  revision.observe(folders);
  assert.equal(changes, 4);
});

test('only products declared to read content change their request identity', () => {
  for (const product of ['country-fuel', 'country-measure']) {
    const request = { sourceId: 'same', revision: 'same', product, params: {} };
    assert.equal(productKey(request, 1), productKey(request, 2));
    assert.ok(!(CONTENT_PRODUCTS as readonly string[]).includes(product));
  }
  for (const product of CONTENT_PRODUCTS) {
    const request = { sourceId: 'same', revision: 'same', product, params: {} };
    assert.notEqual(productKey(request, 1), productKey(request, 2));
  }
  // Counts are recorded, but this reply also names content building definitions.
  assert.ok(CONTENT_PRODUCTS.includes('country-factories'));
});

test('a recording-only product keeps its settled cache and pending load across content changes', async () => {
  let calls = 0;
  let aborted = false;
  let finish = () => {};
  const client = { product: async (request: ProductRequest, signal: AbortSignal) => {
    calls++;
    signal.addEventListener('abort', () => { aborted = true; });
    await new Promise<void>(resolve => { finish = resolve; });
    return { ...request, data: { recorded: '12' } };
  } } as unknown as DashboardClient;
  const request = { sourceId: 'same', revision: 'same', product: 'country-fuel', params: { branchId: '1', tag: 'ABC' } };
  const signal = () => new AbortController().signal;
  const pending = loadProduct(client, request, signal());
  await Promise.resolve();
  contentRevision(client).observe({ game: 'game', user: 'user', contentReady: false });
  const shared = loadProduct(client, request, signal());
  finish();
  assert.deepEqual(await pending, { recorded: '12' });
  assert.deepEqual(await shared, { recorded: '12' });
  contentRevision(client).observe({ game: 'game', user: 'user', contentReady: true });
  assert.deepEqual(await loadProduct(client, request, signal()), { recorded: '12' });
  assert.equal(calls, 1);
  assert.equal(aborted, false);
});

test('content becoming available re-requests successful but empty replies at the same source and hour', async () => {
  let calls = 0;
  let ready = false;
  const client = { product: async (request: ProductRequest) => {
    calls++;
    return { ...request, data: { name: ready ? 'Current country' : null } };
  } } as unknown as DashboardClient;
  const request = { sourceId: 'same', revision: 'same', product: 'country-styles', params: { branchId: '1', hour: 200, tags: ['ABC'] } };
  const signal = () => new AbortController().signal;
  const revision = contentRevision(client);
  revision.observe({ game: '', user: '', contentReady: false });
  assert.deepEqual(await loadProduct(client, request, signal()), { name: null });
  assert.deepEqual(await loadProduct(client, request, signal()), { name: null });
  assert.equal(calls, 1);
  // The flag list's answered empty result depends on this same configuration.
  const oldFlags = flagStore(client);
  const key = FlagStore.key('same', 'same', '1', 200, 'ABC');
  oldFlags.set(key, null);
  ready = true;
  revision.observe({ game: 'game', user: 'user', contentReady: true });
  assert.deepEqual(await loadProduct(client, request, signal()), { name: 'Current country' });
  assert.equal(calls, 2);
  assert.equal(flagStore(client).get(key), undefined);
  assert.notEqual(flagStore(client), oldFlags);
  // Another client has an independent content scope.
  assert.equal(contentRevision({} as DashboardClient).snapshot(), 0);
});
