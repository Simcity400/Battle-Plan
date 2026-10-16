import assert from 'node:assert/strict';
import test from 'node:test';
import { loadProduct } from '../../dashboard/products.ts';
import type { DashboardClient, DashboardSource, ProductRequest } from '../../dashboard/client';

test('leaders use shared prepared routing, retain 5000 selections, and share identical projections', async () => {
  const tags = Array.from({ length: 5000 }, (_, index) => `opaque / country ${index}`);
  const source = { id: 'leaders-source', revision: '1' } as DashboardSource;
  const request: ProductRequest = { sourceId: source.id, revision: source.revision, product: 'unit-leaders',
    params: { branchId: '1', selection: { kind: 'countries', tags }, view: 'choices', leaders: 'all' } };
  let scans = 0, pages = 0;
  const data = { selectionTags: tags };
  const client = {
    product: async (input: ProductRequest) => {
      assert.ok(++scans <= 1, 'The explicit parity request is the only recording request.');
      return { ...input, data: { oracle: true } };
    },
    preparedQuery: async (input: DashboardSource, product: string, params: unknown) => {
      assert.ok(++pages <= 1, '5000-country projections share one prepared request.');
      assert.equal(input, source); assert.equal(product, 'unit-leaders'); assert.deepEqual(params, request.params);
      return data;
    },
  } as unknown as DashboardClient;
  const signal = new AbortController().signal;
  const values = await Promise.all(Array.from({ length: 4 }, () => loadProduct(client, request, signal, undefined, source)));
  assert.equal(scans, 0); assert.equal(pages, 1);
  assert.ok(values.every(value => value === data));
  assert.deepEqual(await loadProduct(client, request, signal), { oracle: true });
  assert.equal(scans, 1);
  await assert.rejects(loadProduct(client, request, signal, undefined, { ...source, revision: '2' }), /source does not match/);
  let fallbackScans = 0;
  const unavailable = { product: async (input: ProductRequest) => {
    assert.ok(++fallbackScans <= 1, 'A client without the prepared API uses one cached recording request.');
    return { ...input, data: { fallback: true } };
  } } as unknown as DashboardClient;
  assert.deepEqual(await loadProduct(unavailable, request, signal, undefined, source), { fallback: true });
  assert.equal(fallbackScans, 1, 'Current routing uses the recording product when the prepared API is absent.');
  assert.equal(scans, 1, 'The prepared-capable client retains its explicit parity request.');
  assert.equal(pages, 1, 'Source rejection and recording fallback cannot repeat a prepared request.');
});
