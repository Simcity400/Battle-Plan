import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import type { DrillState } from '../../charts/drill/drillState';
import { CLOSED } from '../../charts/drill/drillState.ts';
import { readSelection, tradeContext } from './tradeDrill.ts';

test('market moment respects pin/campaign/range while closed and pins/restores each selected row', async () => {
  const state = { drill: CLOSED as DrillState, selected: null as string | null, override: undefined as number | undefined,
    hour: 45, shownHour: 0, pins: [] as WidgetSpec[], views: [] as { data: { hour: number }; selected?: string; onPick?: (key: string) => void }[],
    host: null as null | { onPin?: () => void; onClose: () => void; onFollow?: () => void; open: boolean } };
  const globals = globalThis as typeof globalThis & { __marketWidget?: typeof state }; globals.__marketWidget = state;
  const replacements: Record<string, string> = {
    '/widgets/trade-flows/useTradeFlows.ts': `import { resolveWindow } from '../../dashboard/model.ts';
      export const tradeFlowsRequest = () => { throw Error('resource request in market view'); };
      export const useTradeFlows = (widget, runtime, _active, hour) => {
        const state = globalThis.__marketWidget, track = runtime.tracks[0]; state.override = hour;
        const at = hour ?? Math.floor(resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()).lastHour); state.shownHour = at;
        return { track, all: true, resolved: null, groups: {}, scope: null, retry() {}, market: { data: { branchId: '1', hour: at, offers: [], contracts: [], agreements: [], deliveries: [], warnings: [] } } };
      };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__marketWidget.hour;',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryFlagsAt.ts': 'export const flagReadSpec = () => ({}); export const flagReadMetadata = () => ({}); export const useCountryLabelsAt = () => ({ labelAt: () => () => ({}) });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = () => ""; export const useCountryFactsRead = () => new Map();',
    '/dashboard/products.ts': 'export const useProduct = () => ({ retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: [], surface: "#000000", text: "#ffffff", countryColours: "raw" }); export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => { globalThis.__marketWidget.host = props; return createElement('div', null, props.open ? props.children : props.main); };`,
    '/widgets/trade-flows/TradeMarketView.tsx': 'export default props => { globalThis.__marketWidget.views.push(props); return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'market-widget-moment', enforce: 'pre', transform(code, id) {
      const path = id.replaceAll('\\', '/');
      const found = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
      if (found) return found[1];
      if (path.endsWith('/widgets/trade-flows/TradeFlowsWidget.tsx')) return code
        .replace(/const \[marketSelection, setMarketSelection\] = useState<.*>\(null\);/, 'const [marketSelection, setMarketSelection] = [globalThis.__marketWidget.selected, key => { globalThis.__marketWidget.selected = key; }];')
        .replace(/\buseReducer\(/g, '((reduce) => [globalThis.__marketWidget.drill, action => { globalThis.__marketWidget.drill = reduce(globalThis.__marketWidget.drill, action); }])(')
        .replace(/\buseEffect\(/g, '(() => {})(');
    } }] });
  try {
    const { registerWidgets } = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    const { definition } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/definition.ts');
    registerWidgets([definition]);
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/TradeFlowsWidget.tsx');
    const track = { id: 't', selectedTag: 'AAA', axisOffset: 0, branch: { id: '1', firstHour: 0, lastHour: 100 }, source: { id: 'source', revision: 'r', metadata: { campaignKey: 'campaign', countries: [] } } };
    const props = { widget: { id: 'market', kind: 'trade-flows', title: 'Market', configVersion: 1, trackIds: ['t'], countries: { kind: 'all' }, options: { view: 'market' }, time: { mode: 'pin', windows: { t: { firstOffset: 17, lastOffset: 17 } } } },
      runtime: { client: {}, tracks: [track], clock: { getHour: () => state.hour, getSnapshot: () => ({ position: state.hour, playing: false, hoursPerSecond: 0 }) }, addWidgetBelow: (_id: string, pin: WidgetSpec) => state.pins.push(pin) }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { state.views = []; renderToStaticMarkup(createElement(Widget, props)); };
    render(); assert.equal(state.override, undefined); assert.equal(state.shownHour, 17);
    props.widget.time = { mode: 'campaign' }; render(); assert.equal(state.shownHour, 100);
    props.widget.time = { mode: 'range', firstOffset: 20, lastOffset: 30 }; render(); assert.equal(state.shownHour, 30);
    for (const key of ['offer:5000', 'contract:false:0011', 'agreement:0012']) {
      state.drill = CLOSED; state.selected = null;
      props.widget.options = { view: 'market' }; props.widget.time = { mode: 'pin', windows: { t: { firstOffset: 17, lastOffset: 17 } } };
      render(); state.views[0].onPick!(key); render();
      assert.equal(state.override, 17); assert.equal(state.views[0].selected, key);
      state.host!.onPin!();
      const pin = state.pins.at(-1)!;
      assert.equal(pin.kind, 'trade-flows'); assert.equal(pin.options!.view, 'market'); assert.equal(pin.options!.show, 'contributions');
      assert.deepEqual(readSelection(pin.options!.tradeSelection, tradeContext(track as never)), { kind: 'market', key });
      props.widget = JSON.parse(JSON.stringify(pin)); state.drill = CLOSED; state.selected = null; render();
      assert.equal(state.override, undefined); assert.equal(state.shownHour, 17); assert.equal(state.views[0].selected, key);
      state.host!.onFollow!(); render(); assert.equal(state.shownHour, 45); assert.equal(state.views[0].selected, key);
    }
  } finally { await vite.close(); delete globals.__marketWidget; }
});
