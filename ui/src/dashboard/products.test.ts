import assert from 'node:assert/strict';
import test from 'node:test';
import { forgetProducts, loadProduct, productKey, replyLengths } from './products.ts';
import type { DashboardClient, ProductRequest } from './client';

const request: ProductRequest = { sourceId: 's', revision: 'v', product: 'national-focus', params: { branchId: '1', tag: 'GER' } };

test('identical product requests share one native round trip and forgetting a source reloads', async () => {
  let calls = 0;
  const client = { product: async (input: ProductRequest) => { calls++; return { sourceId: input.sourceId, revision: input.revision, product: input.product, data: { calls } }; } } as unknown as DashboardClient;
  const results = await Promise.all(Array.from({ length: 24 }, () => loadProduct<{ calls: number }>(client, request, new AbortController().signal)));
  assert.equal(calls, 1);
  assert.ok(results.every(value => value === results[0]));
  await loadProduct(client, { ...request, params: { branchId: '1', tag: 'ENG' } }, new AbortController().signal);
  assert.equal(calls, 2);
  await loadProduct(client, request, new AbortController().signal);
  assert.equal(calls, 2);
  forgetProducts(client, 's');
  await loadProduct(client, request, new AbortController().signal);
  assert.equal(calls, 3);
});

test('replies for another recording or product are rejected and payload validators run once per load', async () => {
  const wrong = { product: async () => ({ sourceId: 'other', revision: 'v', product: 'national-focus', data: {} }) } as unknown as DashboardClient;
  await assert.rejects(loadProduct(wrong, request, new AbortController().signal), /does not match/);
  let validated = 0;
  const good = { product: async (input: ProductRequest) => ({ ...input, data: { n: 1 } }) } as unknown as DashboardClient;
  const validate = (data: unknown) => { validated++; return data as { n: number }; };
  const [first, second] = await Promise.all([
    loadProduct(good, request, new AbortController().signal, validate),
    loadProduct(good, request, new AbortController().signal, validate),
  ]);
  assert.equal(validated, 1);
  assert.equal(first, second);
  assert.equal(productKey(request), productKey({ ...request, params: { branchId: '1', tag: 'GER' } }));
  assert.notEqual(productKey(request), productKey({ ...request, revision: 'v2' }));
});

test('the cache sizes a reply by the length its transport received, validated or not', async () => {
  for (const validate of [undefined, (data: unknown) => data]) {
    let calls = 0;
    const client = { product: async (input: ProductRequest) => {
      calls++;
      const reply = { ...input, data: { small: true } };
      // Received at the whole budget: once counted, it cannot be kept.
      replyLengths.set(reply, 64 * 1024 * 1024);
      return reply;
    } } as unknown as DashboardClient;
    await loadProduct(client, request, new AbortController().signal, validate);
    await loadProduct(client, request, new AbortController().signal, validate);
    assert.equal(calls, 2);
  }
});
