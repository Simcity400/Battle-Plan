import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { FlowDiagramProps, FlowNode } from '../../charts/flow/FlowDiagram';
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
    assert.doesNotMatch(render(), /Selected countries are unavailable in this campaign/);
    assert.ok(state.flow, 'known renamed holder retains the complete numeric trade view');
    props.widget.countries = { kind: 'tags', tags: ['unknown'] };
    assert.match(render(), /Selected countries are unavailable in this campaign/);
    assert.match(state.flow!.message!, /Selected countries are unavailable in this campaign/);
    props.widget.countries = { kind: 'tags', tags: ['X'] };
    state.data = { ...data, countryIdentities: [] };
    assert.match(render(), /Selected countries are unavailable in this campaign/, 'missing native evidence cannot become a literal known-empty choice');

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

test('a country’s flag or name opens its whole trade, reading its own reply only when the shown one lacks it', async () => {
  const data: TradeFlowsReply = { branchId: 'branch', decimals: 0, resources: [], countries: ['X', 'Y'], reads: [100],
    known: [{ firstHour: 100, lastHour: 160 }], partial: [], warnings: [], flows: [], countryIdentities: [
      { tag: 'X', runs: [{ firstHour: 100, lastHour: 160, slot: 1, countryId: 10 }] },
      { tag: 'Y', runs: [{ firstHour: 100, lastHour: 119, slot: 2, countryId: 99 }] },
    ] };
  type Item = { label: string; run: () => void };
  const state = { slot: 0, states: [] as unknown[], drill: CLOSED as DrillState, requests: [] as unknown[], reply: undefined as TradeFlowsReply | undefined,
    flow: undefined as FlowDiagramProps | undefined, host: undefined as ComponentProps<typeof DrillHost> | undefined,
    country: undefined as { holder: string; trade: { reply: TradeFlowsReply } } | undefined, menu: [] as Item[] };
  const globals = globalThis as typeof globalThis & { __tradeCountry?: typeof state };
  globals.__tradeCountry = state;
  const replacements: Record<string, string> = {
    '/widgets/trade-flows/useTradeFlows.ts': `import { resolveCountries } from '../../dashboard/groups.ts';
      import { decodeTrade, tradeScope } from './tradeData.ts';
      export const tradeFlowsRequest = (track, tags) => ({ product: 'trade-flows', params: { branchId: track.branch.id, tags } });
      export const useTradeFlows = (widget, runtime) => {
        const track = runtime.tracks[0], data = ${JSON.stringify(data)};
        const resolved = resolveCountries(widget.countries, ['X'], undefined, track.branch.lastHour, Number.MAX_SAFE_INTEGER);
        return { track, all: false, resolved, groups: {}, data, retry() {},
          scope: tradeScope(decodeTrade(data), widget.countries, resolved, undefined, track.branch.lastHour, false) };
      };`,
    '/dashboard/products.ts': `export const useProduct = (client, request) => { const s = globalThis.__tradeCountry;
      if (request) s.requests.push(request); return { data: request ? s.reply : undefined, retry() {} }; };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryFlagsAt.ts': `export const flagReadSpec = () => ({}); export const flagReadMetadata = () => ({});
      export const useCountryLabelsAt = () => ({ labelAt: hour => tag => ({ name: tag + ' at ' + hour }) });`,
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsRead = () => new Map();`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456'], surface: '#000000', text: '#ffffff', muted: '#777777', countryColours: 'raw' });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open(x, y, items) { globalThis.__tradeCountry.menu = items; } });',
    '/charts/drill/DrillHost.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__tradeCountry.host = props; return createElement('div', null, props.main, props.open ? props.children : null); };`,
    '/widgets/trade-flows/TradeDrillView.tsx': 'export default () => null;',
    '/widgets/trade-flows/TradeCountryView.tsx': `import { createElement } from 'react';
      export const resourceLegendItems = () => [];
      export default props => { globalThis.__tradeCountry.country = props; return createElement('i', null, 'country view'); };`,
    '/charts/flow/FlowDiagram.tsx': 'export default props => { globalThis.__tradeCountry.flow = props; return "Trade diagram"; };',
  };
  // Hooks keep their state between renders by call order, as React does.
  const stateful = '((initial) => { const s = globalThis.__tradeCountry, i = s.slot++; if (!(i in s.states)) s.states[i] = initial;'
    + ' return [s.states[i], value => { s.states[i] = typeof value === "function" ? value(s.states[i]) : value; }]; })(';
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'trade-country-caller', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/widgets/trade-flows/TradeFlowsWidget.tsx')) return source
          .replace(/\buseMemo(?:<[^;\n]*>)?\(/g, '((factory) => factory())(')
          .replace(/\buseCallback\(/g, '((callback) => callback)(')
          .replace(/\buseEffect\(/g, '(() => {})(')
          .replace(/\buseReducer\(/g, '((reduce) => [globalThis.__tradeCountry.drill, action => { globalThis.__tradeCountry.drill = reduce(globalThis.__tradeCountry.drill, action); }])(')
          .replace(/\buseState(?:<[^;\n]*>)?\(/g, stateful)
          .replace(/\buseRef(?:<[^;\n]*>)?\(/g, '((initial) => ({ current: initial }))(');
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/TradeFlowsWidget.tsx');
    const track = { id: 'main', selectedTag: 'X', axisOffset: 0, branch: { id: 'branch', firstHour: 100, lastHour: 160 },
      source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', countries: [] } } };
    const props = { widget: { id: 'trade', kind: 'trade-flows', title: 'Trade flows', configVersion: 1, trackIds: ['main'],
      countries: { kind: 'tags', tags: ['X'] }, time: { mode: 'campaign' } }, runtime: { client: {}, tracks: [track], requestSeek() {},
      clock: { getHour: () => 0, getSnapshot: () => ({ position: 0, playing: false, hoursPerSecond: 0 }) } },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { state.slot = 0; return renderToStaticMarkup(createElement(Widget, props)); };
    const country = (holder: string, label: string) => ({ id: `t:${holder}`, side: 'target', label, color: '#000', order: '0',
      trade: { selection: { kind: 'node', side: 'target', holders: [holder] }, node: { identity: holder }, links: [], hour: 160, from: 100 } }) as unknown as FlowNode;
    render();
    assert.ok(state.flow!.onLabelPick, 'the diagram’s names and flags open something');
    assert.equal(state.host!.open, false);

    // Y was not asked for: its own reply is read before its view shows.
    state.flow!.onLabelPick!(country('[2,99]', 'Y'), 160);
    let markup = render();
    assert.equal(state.host!.open, true);
    assert.deepEqual(state.requests.at(-1), { product: 'trade-flows', params: { branchId: 'branch', tags: ['Y'] } });
    assert.doesNotMatch(markup, /country view/);
    state.reply = { ...data, flows: [] };
    markup = render();
    assert.match(markup, /country view/);
    assert.equal(state.country!.holder, '[2,99]');
    assert.equal(state.country!.trade.reply, state.reply);
    assert.equal(state.host!.title, 'Y at 119: imports and exports', 'named as it was last recorded');

    // X was asked for: the shown reply already has its whole trade.
    state.host!.onClose();
    state.requests = [];
    render();
    state.flow!.onContextMenu!({ kind: 'node', node: country('[1,10]', 'X') }, 0, 0);
    assert.equal(state.menu[0].label, 'Imports and exports of X', 'the menu, and so the keyboard, reaches the view too');
    state.menu[0].run();
    render();
    assert.equal(state.country!.holder, '[1,10]');
    assert.deepEqual(state.country!.trade.reply, data);
    assert.deepEqual(state.requests, []);
    assert.equal(state.host!.title, 'X at 160: imports and exports');
  } finally { await vite.close(); delete globals.__tradeCountry; }
});
