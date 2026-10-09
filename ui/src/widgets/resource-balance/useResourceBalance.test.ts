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
  const globals = globalThis as typeof globalThis & { __resourceGroups?: CountryGroupReply };
  globals.__resourceGroups = catalogue;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'resource-alias-membership', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/useCountryGroups.ts')) return 'export const useCountryGroups = () => ({ data: globalThis.__resourceGroups, retry() {} });';
        if (path.endsWith('/dashboard/products.ts')) return `export const useProduct = () => ({ retry() {} });
          export const useProducts = (_client, requests, _active, validate) => ({ data: requests.map(request => validate({
            branchId: 'b', tag: request.params.tag, decimals: 5, warnings: [], resources: [], series: [], trades: [], dealHours: [] })), retry() {} });`;
      },
    }] });
  try {
    const { useResourceCountry: acquire } = await vite.ssrLoadModule('/ui/src/widgets/resource-balance/useResourceBalance.ts');
    const track = { id: 't', source: { id: 's', revision: '1', metadata: { campaignKey: 'c' } },
      branch: { id: 'b', firstHour: 100, lastHour: 120 } } as BoundTrack;
    const runtime = { client: {} } as WidgetRuntime;
    const selection = { campaign: 'c', branch: 'b', tag: 'X', originOffset: 5 };
    let output: ReturnType<typeof useResourceCountry>;
    function Probe({ group }: { group: CountrySelection }) { output = acquire(selection, runtime, track, true, group); return null; }
    const render = (membership: 'hour' | 'end' | 'ever') => renderToStaticMarkup(createElement(Probe, {
      group: { kind: 'group', group: { type: 'humans' }, aggregate: false, membership } }));
    render('hour');
    assert.equal(countryDrillTagAt(output!.spans!, 111), 'Y');
    assert.equal(countryDrillTagAt(output!.spans!, 113), null);
    for (const membership of ['end', 'ever'] as const) {
      render(membership); assert.equal(countryDrillTagAt(output!.spans!, 118), 'Y');
    }
    globals.__resourceGroups = { ...catalogue, humans: [], unobserved: { ...catalogue.unobserved, humans: [{ firstHour: 100, lastHour: 120 }] } };
    render('hour'); assert.equal(output!.spans!.size, 0); assert.match(output!.reason!, /not recorded/);
  } finally { delete globals.__resourceGroups; await vite.close(); }
});
