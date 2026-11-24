import { gatedHour } from '../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { ContentRevision, contentRevision } from './contentRevision.ts';
import { loadProduct, productKey } from './products.ts';
import type { DashboardClient, ProductRequest } from './client';
import { CONTENT_PRODUCTS } from '../generated/dashboard.ts';
import { productRequest } from './request.ts';

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

test('a later presentation generation invalidates content; the first seen, repeats and older ones do not', () => {
  const revision = new ContentRevision();
  let changes = 0;
  revision.subscribe(() => changes++);
  revision.observePresentation(3);
  assert.equal(changes, 0);
  revision.observePresentation(3);
  revision.observePresentation(2);
  assert.equal(changes, 0);
  revision.observePresentation(4);
  assert.equal(changes, 1);
  assert.equal(revision.snapshot(), 1);
  revision.observe({ game: 'game', user: 'user', contentReady: true });
  revision.observePresentation(4);
  assert.equal(changes, 2);
});

test('a content reply that answers after the visual mods changed never fills the new presentation\'s cache', async () => {
  const answers: (() => void)[] = [];
  let calls = 0;
  const client = { product: async (request: ProductRequest) => {
    const style = ++calls === 1 ? 'old style' : 'new style';
    await new Promise<void>(resolve => answers.push(resolve));
    return { ...request, data: { style } };
  } } as unknown as DashboardClient;
  const request = productRequest({ sourceId: 'same', revision: 'same', product: 'sprite-artwork', params: { sprites: [{ key: 'GFX_icon', frame: 0 }] } });
  const signal = () => new AbortController().signal;
  const revision = contentRevision(client);
  revision.observePresentation(1);
  const late = loadProduct(client, request, signal());
  await Promise.resolve();
  revision.observePresentation(2);
  const current = loadProduct(client, request, signal());
  await Promise.resolve();
  for (const answer of answers) answer();
  assert.deepEqual(await late, { style: 'old style' });
  assert.deepEqual(await current, { style: 'new style' });
  assert.deepEqual(await loadProduct(client, request, signal()), { style: 'new style' });
  assert.equal(calls, 2);
});

test('only products declared to read content change their request identity', () => {
  const recorded = productRequest({ sourceId: 'same', revision: 'same', product: 'country-measure', params: {} });
  assert.equal(productKey(recorded, 1), productKey(recorded, 2));
  assert.ok(!(CONTENT_PRODUCTS as readonly string[]).includes(recorded.product));
  for (const product of CONTENT_PRODUCTS) {
    const request = productRequest({ sourceId: 'same', revision: 'same', product, params: {} });
    assert.notEqual(productKey(request, 1), productKey(request, 2));
  }
  assert.ok(CONTENT_PRODUCTS.includes('country-styles'));
  assert.ok(CONTENT_PRODUCTS.includes('country-groups'));
});

test('a recording-only product keeps its settled cache and pending load across content changes', async () => {
  let calls = 0;
  let aborted = false;
  let finish = () => {};
  const response = new Promise<void>(resolve => { finish = resolve; });
  const client = { product: async (request: ProductRequest, signal: AbortSignal) => {
    calls++;
    signal.addEventListener('abort', () => { aborted = true; });
    await response;
    return { ...request, data: { recorded: '12' } };
  } } as unknown as DashboardClient;
  const request = productRequest({ sourceId: 'same', revision: 'same', product: 'country-measure', params: { branchId: '1', tag: 'ABC' } });
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
  const request = productRequest({ sourceId: 'same', revision: 'same', product: 'country-styles', params: { branchId: '1', hour: gatedHour(200), tags: ['ABC'] } });
  const signal = () => new AbortController().signal;
  const revision = contentRevision(client);
  revision.observe({ game: '', user: '', contentReady: false });
  assert.deepEqual(await loadProduct(client, request, signal()), { name: null });
  assert.deepEqual(await loadProduct(client, request, signal()), { name: null });
  assert.equal(calls, 1);
  ready = true;
  revision.observe({ game: 'game', user: 'user', contentReady: true });
  assert.deepEqual(await loadProduct(client, request, signal()), { name: 'Current country' });
  assert.equal(calls, 2);
  assert.equal(contentRevision({} as DashboardClient).snapshot(), 0);
});

