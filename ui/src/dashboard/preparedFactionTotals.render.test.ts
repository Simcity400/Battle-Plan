import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { BoundTrack } from './model';
import type { DashboardClient } from './client';
import type { FactionTotalsParams } from '../generated/dashboard';

test('faction preparation requests only reached hours and bounded geometry, retaining the native origin', async () => {
  const state = { requests: [] as { product: string; params: { reads?: { hour: number }[]; width?: number; family: { kind: string; selection: FactionTotalsParams } } }[],
    prepared: undefined as { exactLineTags: (hour: number) => Promise<readonly string[] | null>; lineSummaries: ReadonlyMap<string, unknown> } | undefined };
  const host = globalThis as typeof globalThis & { __preparedFactionHook?: typeof state }; host.__preparedFactionHook = state;
  const data = { branchId: '1', membershipMode: 'end', recorded: [], disjoint: [], unit: 'states', unavailableReason: null, warnings: [],
    series: [{ id: '401', runs: [{ firstHour: 77, lastHour: 77, value: 0, startsSegment: false }], names: [], members: [], countedMembers: [],
      membership: [{ firstHour: 0, lastHour: 1000000 }], dissolutionHour: null, unavailableReason: null }] };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'prepared-faction-request-fixture', enforce: 'pre', transform(_source, id) {
      if (id.replaceAll('\\', '/').endsWith('/dashboard/products.ts')) return `const data=${JSON.stringify(data)};
        export const useProduct=(_client,request,_active,validate)=>{if(request)globalThis.__preparedFactionHook.requests.push(request);
          return {data:request && request.product==='series-state' ? validate(data) : undefined,retry(){}};};
        export const useFollowedProduct=(client,request,_moving,active,validate)=>useProduct(client,request,active,validate);`;
    } }] });
  try {
    const { usePreparedFactionTotals } = await vite.ssrLoadModule('/ui/src/dashboard/preparedFactionTotals.ts');
    const selection: FactionTotalsParams = { branchId: '1', source: { kind: 'territory', measure: 'states', rule: 'owned' }, membership: 'end', top: 1, factionId: '401', parts: true };
    const track = { branch: { id: '1', firstHour: 0, lastHour: 1000000 }, source: { id: 'source', revision: 'revision' } } as BoundTrack;
    const client = { preparedQuery: async (_source: unknown, product: string, params: unknown) => {
      state.requests.push({ product, params: params as typeof state.requests[number]['params'] }); return data;
    } } as unknown as DashboardClient;
    function Probe() {
      state.prepared = usePreparedFactionTotals(client, track, selection, 77, true, [0, 1000000], 40);
      return null;
    }
    renderToStaticMarkup(createElement(Probe));
    assert.deepEqual(state.requests.map(request => request.product), ['series-state', 'series-state', 'series-summary']);
    assert.deepEqual(state.requests.filter(request => request.product === 'series-state').flatMap(request => request.params.reads!.map(read => read.hour)), [77, 40]);
    assert.equal(state.requests.find(request => request.product === 'series-summary')!.params.width, 1);
    assert.ok(state.requests.every(request => request.params.family.kind === 'faction-totals' && request.params.family.selection === selection));
    assert.deepEqual(await state.prepared!.exactLineTags(77), ['401'], 'known zero remains eligible for an exact faction pick');
    assert.deepEqual(state.requests.at(-1)!.params.reads, [{ hour: 77 }]);
    assert.equal(state.prepared!.lineSummaries.size, 0, 'reached facts do not invent chart geometry');
  } finally { await vite.close(); delete host.__preparedFactionHook; }
});
