import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { StockpileArtworkReply } from '../../generated/dashboard';

test('stockpile artwork masks another client before effects and keeps cancelled replies out', async () => {
  const state = { value: undefined as unknown, effect: undefined as (() => (() => void) | undefined) | undefined,
    requests: [] as { resolve: (data: StockpileArtworkReply) => void; reject: (error: Error) => void }[] };
  const host = globalThis as typeof globalThis & { __stockpileArtwork?: typeof state };
  host.__stockpileArtwork = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'stockpile-artwork-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/equipment-stockpile/useStockpileArtwork.ts')) return source.replace(
          "import { useEffect, useState } from 'react';",
          `const useState = () => [globalThis.__stockpileArtwork.value, value => { globalThis.__stockpileArtwork.value = value; }];
          const useEffect = effect => { globalThis.__stockpileArtwork.effect = effect; };`);
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (path.endsWith('/dashboard/products.ts')) return `export const loadTransientProduct = () => new Promise((resolve, reject) => {
          globalThis.__stockpileArtwork.requests.push({ resolve, reject }); });`;
      },
    }] });
  try {
    const { useStockpileArtwork } = await vite.ssrLoadModule('/ui/src/widgets/equipment-stockpile/useStockpileArtwork.ts') as typeof import('./useStockpileArtwork');
    const first = { client: {} } as WidgetRuntime, second = { client: {} } as WidgetRuntime;
    const track = { source: { id: 'source', revision: 'revision' }, branch: { id: '1' } } as BoundTrack;
    const render = (runtime: WidgetRuntime) => useStockpileArtwork(runtime, track, 'AAA', 100, [], true);
    const reply: StockpileArtworkReply = { branchId: '1', tag: 'AAA', hour: 100, entries: [], warnings: [] };
    const settle = () => new Promise<void>(resolve => setImmediate(resolve));
    assert.equal(render(first), undefined);
    const cleanupFirst = state.effect!();
    state.requests[0].resolve(reply);
    await settle();
    assert.equal(render(first)?.data, reply);
    assert.equal(render(second), undefined, 'identical request text cannot expose the first client');
    cleanupFirst?.();
    const cleanupSecond = state.effect!();
    assert.equal(render(second), undefined);
    cleanupSecond?.();
    const cleanupReturn = state.effect!();
    state.requests[1].resolve({ ...reply, warnings: ['cancelled'] });
    await settle();
    assert.equal(render(second), undefined);
    state.requests[2].reject(new Error('second client failed'));
    await settle();
    assert.equal(render(second)?.error, 'second client failed');
    assert.equal(render(first), undefined, 'errors also belong to their client');
    cleanupReturn?.();
  } finally {
    await vite.close();
    delete host.__stockpileArtwork;
  }
});
