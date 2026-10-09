import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { FlowDiagramProps } from '../../charts/flow/FlowDiagram';
import type { TradeFlowsReply } from '../../generated/dashboard';
import type { ReactElement } from 'react';
import type { DrillState } from '../../charts/drill/drillState';
import type DrillHost from '../../charts/drill/DrillHost';
import type { ComponentProps } from 'react';
import { CLOSED } from '../../charts/drill/drillState.ts';
import { selectionToken } from './tradeDrill.ts';

test('actual Trade flows selection reasons use its native census without fetching a manual group catalogue', async () => {
  const data: TradeFlowsReply = { branchId: 'branch', decimals: 0, resources: [], countries: ['X', 'Y'], reads: [100],
    known: [{ firstHour: 100, lastHour: 160 }], partial: [], warnings: [], flows: [], countryIdentities: [
      { tag: 'X', runs: [{ firstHour: 100, lastHour: 109, slot: 1, countryId: 10 }] },
      { tag: 'Y', runs: [{ firstHour: 100, lastHour: 119, slot: 2, countryId: 99 }, { firstHour: 120, lastHour: 160, slot: 1, countryId: 10 }] },
    ] };
  const state: { data: TradeFlowsReply; flow?: FlowDiagramProps; hour: number; drill: DrillState;
    host?: ComponentProps<typeof DrillHost>; chip?: ReactElement<{ hour: number; label: { name: string; flag: string } | null }>;
    detail?: { hour: number; onPick: (hour: number) => void } } = { data, hour: 125, drill: CLOSED };
  const globals = globalThis as typeof globalThis & { __tradeHolderReason?: typeof state };
  globals.__tradeHolderReason = state;
  const replacements: Record<string, string> = {
    '/widgets/trade-flows/useTradeFlows.ts': `import { resolveCountries } from '../../dashboard/groups.ts';
      import { decodeTrade, tradeScope } from './tradeData.ts';
      export const useTradeFlows = (widget, runtime) => {
        const track = runtime.tracks[0], data = globalThis.__tradeHolderReason.data;
        const resolved = resolveCountries(widget.countries, ['X'], undefined, track.branch.lastHour, Number.MAX_SAFE_INTEGER);
        return { track, all: false, resolved, groups: {}, data, retry() {},
          scope: tradeScope(decodeTrade(data), widget.countries, resolved, undefined, track.branch.lastHour, false) };
      };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__tradeHolderReason.hour;',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryFlagsAt.ts': `export const flagReadSpec = () => ({}); export const flagReadMetadata = () => ({});
      export const useCountryLabelsAt = () => ({ labelAt: hour => tag => ({ name: tag + ' at ' + hour, known: true, flag: tag + '@' + hour }) });`,
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsRead = () => new Map();`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456'], surface: '#000000', text: '#ffffff', muted: '#777777', countryColours: 'raw' });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__tradeHolderReason.host = props; return createElement('div', null, props.main, props.open ? props.children : null); };`,
    '/widgets/trade-flows/TradeDrillView.tsx': 'export default props => { globalThis.__tradeHolderReason.detail = props; return null; };',
    '/charts/flow/FlowDiagram.tsx': 'export default props => { globalThis.__tradeHolderReason.flow = props; return props.message ?? "Trade diagram"; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'trade-native-reason-caller', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/widgets/trade-flows/TradeFlowsWidget.tsx')) return source
          .replace(/\buseMemo(?:<[^;\n]*>)?\(/g, '((factory) => factory())(')
          .replace(/\buseCallback\(/g, '((callback) => callback)(')
          .replace(/\buseEffect\(/g, '(() => {})(')
          .replace(/\buseReducer\(/g, '((reduce) => [globalThis.__tradeHolderReason.drill, action => { globalThis.__tradeHolderReason.drill = reduce(globalThis.__tradeHolderReason.drill, action); }])(')
          .replace(/\buseState(?:<[^;\n]*>)?\(/g, '((initial) => [initial, () => {}])(')
          .replace(/\buseRef(?:<[^;\n]*>)?\(/g, '((initial) => ({ current: initial }))(')
          .replace(/\bcreatePortal\(/g, '((chip) => { globalThis.__tradeHolderReason.chip = chip; return null; })(');
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/TradeFlowsWidget.tsx');
    const track = { id: 'main', selectedTag: 'X', axisOffset: 0, branch: { id: 'branch', firstHour: 100, lastHour: 160 },
      source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', countries: [] } } };
    const props = { widget: { id: 'trade', kind: 'trade-flows', title: 'Trade flows', configVersion: 1, trackIds: ['main'],
      countries: { kind: 'tags', tags: ['X'] }, time: { mode: 'follow' } }, runtime: { client: {}, tracks: [track],
      clock: { getHour: () => state.hour, getSnapshot: () => ({ position: state.hour, playing: false, hoursPerSecond: 0 }) } },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { state.flow = undefined; return renderToStaticMarkup(createElement(Widget, props)); };
    assert.doesNotMatch(render(), /no recorded native identity/);
    assert.ok(state.flow, 'known renamed holder retains the complete numeric trade view');
    props.widget.countries = { kind: 'tags', tags: ['unknown'] };
    assert.match(render(), /no recorded native identity/);
    assert.match(state.flow!.message!, /no recorded native identity/);
    props.widget.countries = { kind: 'tags', tags: ['X'] };
    state.data = { ...data, countryIdentities: [] };
    assert.match(render(), /no recorded native identity/, 'missing native evidence cannot become a literal known-empty choice');

    state.data = { ...data, countryIdentities: [{ ...data.countryIdentities[0], runs: [...data.countryIdentities[0].runs,
      { firstHour: 110, lastHour: 160, slot: 3, countryId: 30 }] }, data.countryIdentities[1]] };
    props.widget.time = { mode: 'pin', windows: { main: { firstOffset: 5, lastOffset: 5 } } };
    props.widget.options = { show: 'contributions', tradeSelection: selectionToken({ kind: 'node', side: 'source', holders: ['[1,10]'] },
      { campaignKey: 'campaign', branchId: 'branch' }) };
    props.countrySlot = {} as HTMLElement;
    render();
    assert.equal(state.detail!.hour, 105);
    assert.equal(state.chip!.props.label!.name, 'X at 105');
    assert.equal(state.chip!.props.label!.flag, 'X@105');
    state.detail!.onPick(125);
    render();
    assert.equal(state.chip!.props.label!.name, 'Y at 125');
    state.hour = 130;
    state.host!.onFollow!();
    render();
    assert.equal(state.detail!.hour, 130);
    assert.equal(state.chip!.props.label!.flag, 'Y@130');
    state.hour = 115;
    render();
    assert.equal(state.chip!.props.label, null, 'an identity gap cannot label the body with the reused tag');
    assert.deepEqual(props.widget.time, { mode: 'pin', windows: { main: { firstOffset: 5, lastOffset: 5 } } });
  } finally { await vite.close(); delete globals.__tradeHolderReason; }
});
