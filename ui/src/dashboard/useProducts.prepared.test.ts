import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('the real batch hook routes resource alias requests through their bound prepared source', async () => {
  const state = { pending: [] as Promise<unknown>[], track<T>(load: Promise<T>) { this.pending.push(load); return load; } };
  const globals = globalThis as typeof globalThis & { __resourceAliasBatch?: typeof state }; globals.__resourceAliasBatch = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'resource-alias-batch-effect', enforce: 'pre', transform(source, id) {
      if (id.replaceAll('\\', '/').endsWith('/dashboard/products.ts')) return source
        .replace('useEffect(() => {', '((effect) => effect())(() => {')
        .replace('void loadProduct<T>(', 'globalThis.__resourceAliasBatch.track(loadProduct<T>(')
        .replace(', attempt > 0, publish).then(publish,', ', attempt > 0, publish)).then(publish,');
    } }] });
  try {
    const { useProducts } = await vite.ssrLoadModule('/ui/src/dashboard/products.ts');
    const source = { id: 'aliases', revision: '1', metadata: { campaignKey: 'mod/campaign' } };
    const calls: string[] = [];
    const client = { product() { throw new Error('An alias request reached the recording oracle.'); },
      preparedQuery: async (actual: unknown, product: string, params: { tag: string }) => {
        assert.equal(actual, source); assert.equal(product, 'resource-balance');
        assert.ok(calls.length < 2, 'one prepared request per selected alias'); calls.push(params.tag); return { tag: params.tag };
      }, preparationStatus() { throw new Error('A completed progressive reply cannot start a poll loop.'); } };
    function Probe() { useProducts(client, ['Old', 'New'].map(tag => ({ sourceId: source.id, revision: source.revision,
      product: 'resource-balance', params: { branchId: '1', tag } })), true, undefined, source); return null; }
    renderToStaticMarkup(createElement(Probe));
    assert.equal(state.pending.length, 2); await Promise.all(state.pending);
    assert.deepEqual(calls, ['Old', 'New']);
  } finally { await vite.close(); delete globals.__resourceAliasBatch; }
});
