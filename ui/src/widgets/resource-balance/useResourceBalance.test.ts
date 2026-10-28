import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { countryDrillTagAt } from '../../dashboard/countryIdentity.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import type { CountrySelection } from '../../dashboard/model';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { useResourceCountry } from './useResourceBalance';

// Keep the actual acquisition hook; replace only its transport.
test('resource acquisition uses alias membership and preserves end/ever histories', async () => {
  const catalogue: CountryGroupReply = { branchId: 'b', warnings: [], humans: [
    { tag: 'X', firstHour: 100, lastHour: 120 }, { tag: 'Y', firstHour: 110, lastHour: 112 }],
    factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [
      { tag: 'X', identities: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 }, { firstHour: 110, lastHour: 120, slot: 2, countryId: 20 }] },
      { tag: 'Y', identities: [{ firstHour: 110, lastHour: 120, slot: 1, countryId: 10 }] },
    ] };
  const globals = globalThis as typeof globalThis & { __resourceGroups?: CountryGroupReply; __resourceSource?: unknown; __resourceRequests?: unknown[][]; __resourceReads?: unknown[] };
  globals.__resourceGroups = catalogue;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'resource-alias-membership', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/useCountryGroups.ts')) return 'export const useCountryGroups = () => ({ data: globalThis.__resourceGroups, retry() {} });';
        // The viewport read lands at once; exact pages keep their batch.
        if (path.endsWith('/dashboard/viewRead.ts')) return `export const useViewRead = (lane, view, _viewKey, active, read) => { let data;
          if (active && lane != null && view != null) void read(view, new AbortController().signal, value => { data = value; });
          return { data, retry() {} }; };`;
        if (path.endsWith('/dashboard/products.ts')) return `export const useProduct = () => ({ retry() {} });
          export const loadProduct = (_client, request, _signal, validate, source, _restart, publish) => { globalThis.__resourceSource = source; globalThis.__resourceReads.push(request);
            publish?.(validate({ branchId: 'b', tag: request.params.tag, decimals: 5, warnings: [], resources: [], series: [], trades: [], dealHours: [] })); return new Promise(() => {}); };
          export const useProducts = (_client, requests, _active, validate, source) => { globalThis.__resourceSource = source; globalThis.__resourceRequests.push(requests); return ({ data: requests.map(request => validate({
            branchId: 'b', tag: request.params.tag, decimals: 5, warnings: [], resources: [], series: [], trades: [], dealHours: [] })), retry() {} }); };
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });
export const useFollowedProducts = (client, requests, _moving, ...rest) => ({ ...useProducts(client, requests, ...rest), requests });`;
      },
    }] });
  try {
    const { useResourceCountry: acquire, mergePreparedResource } = await vite.ssrLoadModule('/ui/src/widgets/resource-balance/useResourceBalance.ts');
    const track = { id: 't', source: { id: 's', revision: '1', metadata: { campaignKey: 'c' } },
      branch: { id: 'b', firstHour: 100, lastHour: 120 } } as BoundTrack;
    const runtime = { client: { preparedQuery() {} } } as unknown as WidgetRuntime;
    const selection = { campaign: 'c', branch: 'b', tag: 'X', originOffset: 5 };
    let output: ReturnType<typeof useResourceCountry>;
    function Probe({ group }: { group: CountrySelection }) { output = acquire(selection, runtime, track, true, group, { from: 100.5, to: 119.5, width: 20, hours: [105] }, 113); return null; }
    const render = (membership: 'hour' | 'end' | 'ever') => { globals.__resourceRequests = []; globals.__resourceReads = []; return renderToStaticMarkup(createElement(Probe, {
      group: { kind: 'group', group: { type: 'humans' }, aggregate: false, membership } })); };
    render('hour');
    assert.equal(globals.__resourceSource, track.source, 'alias requests retain the prepared source');
    const reads = globals.__resourceReads as { params: unknown }[], batches = globals.__resourceRequests as { params: unknown }[][];
    const membership = (tag: string) => tag === 'X' ? [{ firstHour: 100, lastHour: 109 }] : [{ firstHour: 110, lastHour: 112 }];
    assert.deepEqual(reads.map(request => request.params), ['X', 'Y'].map(tag => ({ branchId: 'b', tag, from: 100.5, to: 119.5, width: 20, within: membership(tag), hours: [119] })));
    assert.deepEqual(batches[0].map(request => request.params), ['X', 'Y'].map(tag => ({ branchId: 'b', tag, from: 105, to: 105, width: undefined, within: membership(tag), hours: [105, 113] })));
    assert.equal(countryDrillTagAt(output!.spans!, 111), 'Y');
    assert.equal(countryDrillTagAt(output!.spans!, 113), null);
    for (const membership of ['end', 'ever'] as const) {
      render(membership); assert.equal(countryDrillTagAt(output!.spans!, 118), 'Y');
    }
    globals.__resourceGroups = { ...catalogue, humans: [], unobserved: { ...catalogue.unobserved, humans: [{ firstHour: 100, lastHour: 120 }] } };
    render('hour'); assert.equal(output!.spans!.size, 0); assert.match(output!.reason!, /Group membership is incomplete/);
    const a = { branchId: 'b', tag: 'X', resources: [], series: [{ id: 7, runs: [{ firstHour: 100, lastHour: 109, producedRaw: '9007199254740993' }] }],
      trades: [], dealHours: [{ firstHour: 100, lastHour: 109 }], summaries: [], exactHours: [105] };
    const b = { ...a, series: [{ id: 7, runs: [{ firstHour: 111, lastHour: 120, producedRaw: '-7' }] }],
      dealHours: [{ firstHour: 111, lastHour: 120 }], exactHours: [113] };
    const merged = mergePreparedResource(a, b);
    assert.equal(merged.summaries, a.summaries, 'point reads retain accepted geometry');
    assert.deepEqual(merged.series[0].runs.map((run: { producedRaw: string }) => run.producedRaw), ['9007199254740993', '-7']);
    assert.deepEqual(merged.exactHours, [105, 113]);
    assert.equal(merged.dealHours.length, 2, 'missing coverage stays missing');
    assert.equal(a.series[0].runs.length, 1, 'joining does not mutate accepted viewport data');
    const point = { ...b, series: [{ id: 7, runs: [{ firstHour: 105, lastHour: 105, producedRaw: '-9' }] },
      { id: 8, runs: [{ firstHour: 105, lastHour: 105, producedRaw: '3' }] }], resources: [{ id: 8, token: 'mod/new' }] };
    const expanded = mergePreparedResource(a, point);
    assert.deepEqual(expanded.series.map((row: { id: number }) => row.id), [7, 8], 'exact reads can discover new native resource keys');
    assert.deepEqual(expanded.series[0].runs.map((run: { firstHour: number; lastHour: number; producedRaw: string }) => [run.firstHour, run.lastHour, run.producedRaw]),
      [[100, 104, '9007199254740993'], [105, 105, '-9'], [106, 109, '9007199254740993']]);
    assert.equal(expanded.resources[0].token, 'mod/new');
    const deal = { deal: '51', resource: 7, direction: 'import', partner: 'Y', deliveredRaw: '4', factories: 1, convoysAssigned: 0, convoysRequired: 0, startsSegment: true };
    const reached = mergePreparedResource({ ...a, trades: [{ ...deal, firstHour: 100, lastHour: 120 }], dealHours: [{ firstHour: 100, lastHour: 120 }] },
      { ...b, trades: [{ ...deal, firstHour: 100, lastHour: 125 }], dealHours: [{ firstHour: 100, lastHour: 125 }] });
    assert.deepEqual(reached.trades.map((run: { firstHour: number; lastHour: number }) => [run.firstHour, run.lastHour]), [[100, 125]], 'a deal read at a later frontier is listed once');
    assert.deepEqual(reached.dealHours, [{ firstHour: 100, lastHour: 125 }]);
    globals.__resourceGroups = { ...catalogue, countryIdentities: [...catalogue.countryIdentities, ...Array.from({ length: 4998 }, (_, index) => ({
      tag: `mod country ${index}`, identities: [{ firstHour: 100, lastHour: 120, slot: index + 100, countryId: index + 1000 }] }))] };
    render('ever');
    const scaleReads = globals.__resourceReads as unknown[], scaleBatches = globals.__resourceRequests as unknown[][];
    assert.equal(globals.__resourceGroups.countryIdentities.length, 5000);
    assert.equal(scaleReads.length + scaleBatches.flat().length, 4, 'five thousand native holders still request only this holder’s two aliases in the viewport and exact page');
    assert.ok(scaleReads.length <= 2 && scaleBatches.length <= 1 && scaleBatches.flat().length <= 2, 'alias acquisition has a bounded request count');
  } finally { delete globals.__resourceGroups; delete globals.__resourceSource; delete globals.__resourceRequests; delete globals.__resourceReads; await vite.close(); }
});
