import assert from 'node:assert/strict';
import test from 'node:test';
import type { DashboardClient, ProductRequest } from './client';
import type { CountryFlagParams } from '../generated/dashboard';
import { FLAG_BATCH, FLAG_STORE_BYTES, FlagStore, flagStore, loadListFlags, storedFlag } from './flagCache.ts';
import { forgetProducts, loadProduct } from './products.ts';

const source = { id: 's', revision: 'v' };
const signal = () => new AbortController().signal;

function fakeClient() {
  const requests: ProductRequest<CountryFlagParams>[] = [];
  let productCalls = 0;
  const client = {
    product: async (request: ProductRequest<CountryFlagParams>) => {
      if (request.product !== 'country-flags') { productCalls++; return { ...request, data: { calls: productCalls } }; }
      requests.push(request);
      const flags = request.params.tags.filter(tag => tag !== 'NOFLAG')
        .map(tag => ({ tag, asset: `gfx/flags/medium/${tag}.tga`, width: 1, height: 1, pixels: [0, 0, 0, 255] }));
      return { ...request, data: { branchId: request.params.branchId, hour: request.params.hour, size: request.params.size, flags, warnings: [] } };
    },
  } as unknown as DashboardClient;
  return { client, requests, productCalls: () => productCalls };
}

test('list flags load in bounded batches per observation hour and are never requested twice', async () => {
  const { client, requests } = fakeClient();
  const targets = Array.from({ length: 40 }, (_, index) => ({ tag: `T${index}`, branchId: '1', hour: index < 35 ? 100 : 200 }));
  targets.push({ tag: 'NOFLAG', branchId: '1', hour: 100 });
  await loadListFlags(client, source, targets, signal());
  assert.deepEqual(requests.map(request => [request.params.hour, request.params.tags.length]), [[100, FLAG_BATCH], [100, 4], [200, 5]]);
  assert.ok(requests.every(request => request.params.size === 'shield'));
  const store = flagStore(client);
  assert.equal(store.get(FlagStore.key('s', 'v', '1', 100, 'T0'))?.tag, 'T0');
  assert.equal(store.get(FlagStore.key('s', 'v', '1', 100, 'NOFLAG')), null);
  assert.equal(store.get(FlagStore.key('s', 'v', '1', 200, 'T0')), undefined);
  await loadListFlags(client, source, targets, signal());
  assert.equal(requests.length, 3);
  forgetProducts(client, 's');
  assert.equal(store.get(FlagStore.key('s', 'v', '1', 100, 'T0')), undefined);
});

test('scrolling through thousands of flags never evicts widget products', async () => {
  const { client, productCalls } = fakeClient();
  const widget: ProductRequest = { sourceId: 's', revision: 'v', product: 'national-focus', params: { branchId: '1', tag: 'GER' } };
  await loadProduct(client, widget, signal());
  for (let start = 0; start < 5_000; start += 20) {
    const visible = Array.from({ length: 20 }, (_, offset) => ({ tag: `T${start + offset}`, branchId: '1', hour: 100 }));
    await loadListFlags(client, source, visible, signal());
  }
  await loadProduct(client, widget, signal());
  assert.equal(productCalls(), 1);
});

test('the flag store is bounded by bytes and keeps pixels as bytes', () => {
  const flag = (tag: string) => storedFlag({ tag, asset: `gfx/flags/medium/${tag}.tga`, width: 41, height: 26, pixels: Array.from({ length: 41 * 26 * 4 }, () => 255) });
  assert.ok(flag('A').pixels instanceof Uint8ClampedArray);
  assert.equal(flag('A').pixels.byteLength, 41 * 26 * 4);
  const key = (tag: string, sourceId = 's') => FlagStore.key(sourceId, 'v', '1', 100, tag);
  // Room for three medium flags, not four: the least recently used one goes.
  const budget = 3.5 * (41 * 26 * 4 + 300);
  const store = new FlagStore(budget);
  for (const tag of ['A', 'B', 'C']) store.set(key(tag), flag(tag));
  store.get(key('A'));
  store.set(key('D'), flag('D'));
  assert.equal(store.size, 3);
  assert.equal(store.get(key('B')), undefined);
  assert.equal(store.get(key('A'))?.tag, 'A');
  assert.ok(store.bytes <= budget);
  // Replacing and forgetting entries keeps the byte count exact.
  const full = store.bytes;
  store.set(key('A'), null);
  assert.ok(store.bytes < full - 41 * 26 * 4 + 1);
  store.set(key('Z', 'other'), null);
  const withOther = store.bytes;
  store.deleteSource('other');
  assert.ok(store.bytes < withOther && store.get(key('Z', 'other')) === undefined);
  // A flag larger than the whole budget is still kept alone rather than dropped.
  const tiny = new FlagStore(10);
  tiny.set(key('X'), flag('X'));
  assert.equal(tiny.get(key('X'))?.tag, 'X');
  assert.equal(tiny.size, 1);
});

test('five thousand medium flags stay within the default flag budget', () => {
  const store = new FlagStore(FLAG_STORE_BYTES);
  const pixels = Array.from({ length: 41 * 26 * 4 }, () => 7);
  for (let index = 0; index < 5_000; index++) {
    store.set(FlagStore.key('s', 'v', '1', 100, `T${index}`), storedFlag({ tag: `T${index}`, asset: 'a', width: 41, height: 26, pixels }));
  }
  assert.equal(store.size, 5_000);
  assert.ok(store.bytes < FLAG_STORE_BYTES);
});
