import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, Fragment } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

interface Counts { requests: number; asks: number; lookups: number; hours: number[] }

const replacements: Record<string, string> = {
  '/dashboard/products.ts': `export const useProduct = (_client, request) => {
    if (request) globalThis.__regionCounts.requests += 1;
    return { data: { regions: [{ id: 16, sea: true }, { id: 19, sea: false }] }, error: undefined, retry() {} };
  };`,
  '/families/identity/react.ts': `export const regionKey = id => 'region:' + id;
    export const useIdentityScope = (_client, recording, branchId, active = true) => (recording && branchId && active ? { branchId } : null);
    export const useIdentities = (_client, _recording, _scope, asks) => {
      globalThis.__regionCounts.asks += asks.length;
      globalThis.__regionCounts.hours.push(...asks.map(ask => ask.hour));
      return { resolve: () => { globalThis.__regionCounts.lookups += 1; return { kind: 'known', name: { kind: 'known', value: 'North Sea' } }; } };
    };`,
};

test('closed region choosers on a playing page read and name nothing; an opened one reads its list once', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'region-list-counts', enforce: 'pre',
      transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
  try {
    const { useRegionList } = await vite.ssrLoadModule('/ui/src/families/regions/useRegionList.ts') as typeof import('./useRegionList.ts');
    const counts: Counts = { requests: 0, asks: 0, lookups: 0, hours: [] };
    (globalThis as Record<string, unknown>).__regionCounts = counts;
    const track = { source: { id: 's', revision: '1' }, branch: { id: '1', lastHour: 100 } } as never;
    const states: string[] = [];
    const Widget = ({ open }: { open: boolean }) => {
      states.push(useRegionList({} as never, track, open, 40).kind);
      return null;
    };
    const page = (open: boolean) => createElement(Fragment, null, ...Array.from({ length: 6 }, (_, at) => createElement(Widget, { key: at, open })));
    for (let hour = 0; hour < 24; hour++) renderToStaticMarkup(page(false));
    assert.deepEqual(counts, { requests: 0, asks: 0, lookups: 0, hours: [] }, 'six closed choosers over 24 played hours do no region-list work');
    assert.ok(states.every(kind => kind === 'loading'));
    renderToStaticMarkup(createElement(Widget, { open: true }));
    assert.equal(counts.requests, 1, 'an opened chooser asks for the list');
    assert.equal(counts.lookups, 2, 'and names each listed region once');
    assert.deepEqual([...new Set(counts.hours)], [40], 'at the hour the widget shows, never the branch’s last hour');
  } finally {
    delete (globalThis as Record<string, unknown>).__regionCounts;
    await vite.close();
  }
});
