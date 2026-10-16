import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('trade viewport, followed drill and pointer hours acquire independent prepared pages', async () => {
  const widget = { trackIds: ['t'], countries: { kind: 'all' }, time: { mode: 'campaign' }, options: { measure: 'current' } };
  const source = { id: 's', revision: '1', metadata: { campaignKey: 'c', countries: [] } };
  const track = { id: 't', source, branch: { id: '1', firstHour: 0, lastHour: 100, countries: [] }, axisOffset: 0 };
  const runtime = { client: { preparedQuery() {} }, tracks: [track], clock: { getHour: () => 100 } };
  const key = JSON.stringify(['s', '1', '1', null, widget.countries, undefined, 'current']);
  const state = { viewport: { key, from: 20, to: 80, width: 60 }, point: { key, hour: 50 }, detail: { key, hour: 70 },
    requests: [] as { params: { width?: number; from?: number; hours?: number[] } }[], sources: [] as unknown[] };
  const globals = globalThis as typeof globalThis & { __tradeAcquisition?: typeof state }; globals.__tradeAcquisition = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'trade-exact-demand', enforce: 'pre', transform(code, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/dashboard/useCountryGroups.ts')) return 'export const useCountryGroups = () => ({});';
      if (path.endsWith('/dashboard/products.ts')) return 'export const useProduct = (_client, request, _active, _validate, source) => { if (request) { globalThis.__tradeAcquisition.requests.push(request); globalThis.__tradeAcquisition.sources.push(source); } return { retry() {} }; };';
      if (path.endsWith('/widgets/trade-flows/useTradeFlows.ts')) return code.replace(/const \[(viewport|point|detail), (\w+)\] = useState<.*>\(\);/g,
        (_whole, name, setter) => `const [${name}, ${setter}] = [globalThis.__tradeAcquisition.${name}, () => {}];`);
    } }] });
  try {
    const { useTradeFlows } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/useTradeFlows.ts');
    function Probe() { useTradeFlows(widget, runtime, true); return null; }
    const render = () => { state.requests = []; state.sources = []; renderToStaticMarkup(createElement(Probe)); return state.requests.map(request => request.params); };
    const first = render();
    assert.equal(first.length, 4);
    assert.deepEqual(first[0].hours, [100]);
    assert.deepEqual(first[2].hours, [100, 100, 70]);
    assert.deepEqual(first[3].hours, [50, 100]);
    assert.ok(state.sources.every(actual => actual === source));
    state.point.hour = 60;
    const hovered = render();
    assert.deepEqual(hovered[0], first[0]);
    assert.deepEqual(hovered[2], first[2], 'hover never replaces the followed drill demand');
    assert.deepEqual(hovered[3].hours, [60, 100]);
    state.detail.hour = 80;
    const followed = render();
    assert.deepEqual(followed[0], first[0], 'Follow never reloads viewport geometry');
    assert.deepEqual(followed[2].hours, [100, 100, 80]);
    assert.deepEqual(followed[3], hovered[3], 'Follow leaves the pointer demand intact');
    source.metadata.countries = Array.from({ length: 5000 }, (_, index) => ({ tag: `mod-${index}` })) as never[];
    const large = render();
    assert.equal(large.length, 4, 'five thousand countries share one viewport, scale and two exact-page requests');
    assert.ok(large.length <= 4, 'acquisition is bounded independently of country count');
  } finally { await vite.close(); delete globals.__tradeAcquisition; }
});
