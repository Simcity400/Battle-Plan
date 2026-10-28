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
      if (path.endsWith('/dashboard/products.ts')) return 'export const useProduct = (_client, request, _active, _validate, source) => { if (request) { globalThis.__tradeAcquisition.requests.push(request); globalThis.__tradeAcquisition.sources.push(source); } return { retry() {} }; }; export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });'
        + ' export const loadProduct = (client, request, _signal, validate, source) => { useProduct(client, request, true, validate, source); return new Promise(() => {}); };';
      // The viewport read asks synchronously, as each render's demand.
      if (path.endsWith('/dashboard/viewRead.ts')) return `export const useViewRead = (lane, view, _viewKey, active, read) => { let data;
        if (active && lane != null && view != null) void read(view, new AbortController().signal, value => { data = value; });
        return { data, retry() {} }; };`;
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

test('market view shares source, native country filter and moment demand without resource requests', async () => {
  const state = { requests: [] as { sourceId: string; revision: string; product: string; params: { hour: number; tags: string[] } }[],
    groups: { branchId: '1', warnings: [], identityChanges: [], warStarts: [], capitulations: [],
      humans: [{ tag: 'AAA', firstHour: 0, lastHour: 29 }, { tag: 'BBB', firstHour: 30, lastHour: 100 }],
      factions: [], wars: [], unobserved: { humans: [] as { firstHour: number; lastHour: number }[], factions: [], wars: [] },
      countryIdentities: [
        { tag: 'AAA', identities: [{ firstHour: 0, lastHour: 29, slot: 1, countryId: 1 }] },
        { tag: 'BBB', identities: [{ firstHour: 30, lastHour: 100, slot: 1, countryId: 1 }] },
      ] }, groupDemand: false, result: null as null | { marketReason: string | null } };
  const globals = globalThis as typeof globalThis & { __marketAcquisition?: typeof state }; globals.__marketAcquisition = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'trade-market-demand', enforce: 'pre', transform(_code, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/dashboard/useCountryGroups.ts')) return 'export const useCountryGroups = (_runtime, _track, active) => { globalThis.__marketAcquisition.groupDemand = active; return { data: globalThis.__marketAcquisition.groups }; };';
      if (path.endsWith('/dashboard/products.ts')) return 'export const useProduct = (_client, request) => { if (request) globalThis.__marketAcquisition.requests.push(request); return { retry() {} }; }; export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });';
    } }] });
  try {
    const { useTradeFlows } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/useTradeFlows.ts');
    const widget = { trackIds: ['t'], countries: { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, time: { mode: 'campaign' }, options: { view: 'market' } };
    const source = { id: 'source', revision: 'revision', metadata: { campaignKey: 'c', countries: [] } };
    const track = { id: 't', source, branch: { id: '1', firstHour: 0, lastHour: 100 }, axisOffset: 0 };
    const runtime = { client: { preparedQuery() {} }, tracks: [track], clock: { getHour: () => 45 } };
    let hour = 15;
    function Probe() { state.result = useTradeFlows(widget, runtime, true, hour); return null; }
    const render = () => { state.requests = []; renderToStaticMarkup(createElement(Probe)); return state.requests; };
    assert.deepEqual(render().map(request => [request.product, request.params.hour, request.params.tags]), [['market-lendlease', 15, ['AAA']]]);
    hour = 45;
    assert.deepEqual(render().map(request => [request.sourceId, request.revision, request.params.hour, request.params.tags]), [['source', 'revision', 45, ['BBB']]]);
    assert.equal(state.groupDemand, true);
    state.groups.unobserved.humans = [{ firstHour: 40, lastHour: 50 }];
    assert.deepEqual(render(), [], 'unknown membership must never become an all-countries request');
    assert.match(state.result!.marketReason!, /unavailable/);
    state.groups.unobserved.humans = [];
    widget.countries = { kind: 'tags', tags: ['AAA'] } as never;
    assert.deepEqual(render()[0].params.tags, ['BBB'], 'manual picks follow their native holder through its renamed tag');
    widget.countries = { kind: 'all' } as never;
    assert.deepEqual(render()[0].params.tags, []);
    assert.equal(state.groupDemand, false);
  } finally { await vite.close(); delete globals.__marketAcquisition; }
});

test('keyboard navigation asks for the drawn flows in the selection shown', async () => {
  type Navigate = (key: string, from: number | null, x: [number, number], until: number | null) => void;
  const state = { navigations: [] as Record<string, unknown>[], result: null as null | { navigateFlows?: (flows: readonly string[], country?: readonly string[]) => Navigate } };
  const globals = globalThis as typeof globalThis & { __tradeNavigation?: typeof state }; globals.__tradeNavigation = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'trade-navigation', enforce: 'pre', transform(_code, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/dashboard/useCountryGroups.ts')) return 'export const useCountryGroups = () => ({});';
      if (path.endsWith('/dashboard/products.ts')) return 'export const useProduct = () => ({ retry() {} }); export const loadProduct = () => new Promise(() => {});';
      if (path.endsWith('/dashboard/viewRead.ts')) return 'export const useViewRead = () => ({ retry() {} });';
      if (path.endsWith('/dashboard/preparedSeries.ts')) return 'export const preparedSeriesQuery = (_client, _source, _product, params) => { globalThis.__tradeNavigation.navigations.push(params); return new Promise(() => {}); };';
    } }] });
  try {
    const { useTradeFlows } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/useTradeFlows.ts');
    const widget = { trackIds: ['t'], countries: { kind: 'tags', tags: ['AAA', 'BBB'] }, time: { mode: 'campaign' }, options: { measure: 'current', between: 'between' } };
    const track = { id: 't', source: { id: 's', revision: '1', metadata: { campaignKey: 'c', countries: [] } }, branch: { id: '1', firstHour: 0, lastHour: 100 }, axisOffset: 0 };
    const runtime = { client: { preparedQuery() {} }, tracks: [track], clock: { getHour: () => 100 } };
    function Probe() { state.result = useTradeFlows(widget, runtime, true); return null; }
    renderToStaticMarkup(createElement(Probe));
    state.result!.navigateFlows!(['ab'])('ArrowRight', 10, [0, 100], null);
    assert.deepEqual(state.navigations[0], { branchId: '1', tags: ['AAA', 'BBB'], between: true, flows: ['ab'],
      op: 'navigate', key: 'ArrowRight', from: 10, firstHour: 0, lastHour: 100 }, 'only trade between them, and only the flows drawn');
    state.result!.navigateFlows!(['ac', 'ab'], ['AAA'])('ArrowLeft', 10, [0, 100], 50);
    assert.deepEqual(state.navigations[1], { branchId: '1', tags: ['AAA'], between: false, flows: ['ac', 'ab'],
      op: 'navigate', key: 'ArrowLeft', from: 10, firstHour: 0, lastHour: 50 }, "one country's view steps through its own whole trade");
  } finally { await vite.close(); delete globals.__tradeNavigation; }
});
