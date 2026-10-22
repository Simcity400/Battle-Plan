import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { createServer } from 'vite';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { ProductionSelection, ProductionSelectionRequest } from './preparedProducedData';

// Execute the actual hook with controlled React state/effects and transport.
// This isolates page carry/cancellation ownership without certifying DOM scrolling.
test('range loading carries its own numeric page and displayed hour while the next page waits', async () => {
  const states: unknown[] = [];
  let cursor = 0;
  let effect: (() => void | (() => void)) | undefined;
  let cleanup: (() => void) | undefined;
  let previousDeps: readonly unknown[] | undefined;
  const harness = {
    useState(initial: unknown) {
      const index = cursor++;
      if (!(index in states)) states[index] = typeof initial === 'function' ? initial() : initial;
      return [states[index], (next: unknown) => { states[index] = typeof next === 'function' ? next(states[index]) : next; }];
    },
    useEffect(callback: () => void | (() => void), deps: readonly unknown[]) {
      if (!previousDeps || deps.some((value, index) => value !== previousDeps![index])) { effect = callback; previousDeps = deps; }
    },
  };
  const globals = globalThis as typeof globalThis & { __productionHookFixture?: typeof harness };
  globals.__productionHookFixture = harness;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'production-hook-react', enforce: 'pre',
      async load(id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/contentRevision.ts')) return 'export const useContentRevision = () => 0;';
        if (!path.endsWith('/widgets/equipment-produced/usePreparedProduction.ts')) return;
        return (await readFile(id, 'utf8')).replace("import { useEffect, useState } from 'react';",
          'const {useEffect,useState} = globalThis.__productionHookFixture;');
      } }] });
  try {
    const { usePreparedProduction } = await vite.ssrLoadModule('/ui/src/widgets/equipment-produced/usePreparedProduction.ts');
    const source = { id: 'source', revision: 'revision' } as DashboardSource;
    const selection = { branchId: '1', tags: [], top: 5000, measure: 'total', count: 'units', grouping: 'type', included: [], refits: false };
    const request = { hour: 12, selection, offset: '0', limit: 1, ranked: true } as ProductionSelectionRequest;
    const page: ProductionSelection = { schema: 1, branchId: '1', firstHour: 0, lastHour: 23, known: true, observation: null,
      totalCountries: 5000, offset: '0', valueMin: '-7', valueMax: '9', keys: [], types: [], countries: [], warnings: [] };
    let finish!: (page: ProductionSelection) => void;
    let calls = 0;
    const client = { preparedQuery: async () => ++calls === 1 ? page : await new Promise<ProductionSelection>(resolve => { finish = resolve; }) } as unknown as DashboardClient;
    const render = (next = request) => {
      cursor = 0;
      const result = usePreparedProduction(client, source, next, true);
      if (effect) { cleanup?.(); const run = effect; effect = undefined; cleanup = run() || undefined; }
      return result;
    };
    render();
    await new Promise(resolve => setTimeout(resolve, 0));
    assert.equal(render().data, page);
    const next = { ...request, offset: '4999' };
    assert.equal(render(next).data, page);
    const pending = render(next);
    assert.equal(pending.pending, true);
    assert.equal(pending.data.offset, '0');
    assert.equal(pending.hour, 12);
    const last = { ...page, offset: '4999' };
    finish(last);
    await new Promise(resolve => setTimeout(resolve, 0));
    assert.equal(render(next).data, last);
    assert.equal(render(next).pending, false);
    assert.equal(calls, 2);
  } finally { cleanup?.(); await vite.close(); delete globals.__productionHookFixture; }
});
