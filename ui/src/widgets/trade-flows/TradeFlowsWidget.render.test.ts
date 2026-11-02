import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { dayLabel } from '../../charts/draw.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply, TradeFlowsReply } from '../../generated/dashboard';
import { BRANCH, humansBy, KNOWN_HOUR, mountModule, PENDING_HOUR, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('trade of a group counted at an hour still being prepared is loading, not no trade', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  // Trade was recorded throughout, and GER, the human player, never traded.
  const data: TradeFlowsReply = { branchId: BRANCH.id, decimals: 0, resources: [], countries: ['GER', 'ENG'], reads: [0], known: [span], partial: [], warnings: [],
    flows: [], countryIdentities: ['GER', 'ENG'].map((tag, index) => ({ tag, runs: [{ ...span, slot: index + 1, countryId: index + 101 }] })) };
  const state = { data, groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __tradePending?: typeof state }; host.__tradePending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/trade-flows/TradeFlowsWidget.tsx', {
    // The hook's own resolution of the choice, on the trade reply and the groups shown.
    '/widgets/trade-flows/useTradeFlows.ts': `import { resolveCountries } from '../../dashboard/groups.ts';
      import { decodeTrade, tradeScope } from './tradeData.ts';
      export const useTradeFlows = (widget, runtime) => {
        const track = runtime.tracks[0], { data, groups } = globalThis.__tradePending;
        const resolved = resolveCountries(widget.countries, [], groups, track.branch.lastHour, Number.MAX_SAFE_INTEGER);
        return { track, all: false, resolved, groups: { data: groups, retry() {} }, data, scaleData: data, retry() {},
          scope: tradeScope(decodeTrade(data), widget.countries, resolved, groups, track.branch.lastHour, false) };
      };`,
    '/dashboard/playback.ts': `export const usePlaybackHour = clock => clock.getHour();`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryFlagsAt.ts': `export const flagReadMetadata = () => ({});
      export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag, known: true }) });`,
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsRead = () => new Map();',
    '/dashboard/countryFlags.ts': 'export const useCountryFlagsRead = () => new Map();',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456'], surface: '#000000', text: '#ffffff', muted: '#777777', countryColours: 'raw' });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/widgets/trade-flows/TradeDrillView.tsx': 'export default () => null;',
    '/charts/flow/FlowDiagram.tsx': 'export default props => props.message ?? "Trade diagram";',
  });
  try {
    const props = { widget: { id: 'trade', kind: 'trade-flows', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' }, options: {} },
      runtime: { client: {}, tracks: [{ id: 'main', axisOffset: 0, branch: BRANCH, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'campaign', countries: [] } } }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ position: BRANCH.lastHour, playing: false, hoursPerSecond: 0 }) } },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No trade/);
    // Read, GER is the human player then and never traded.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No trade at this moment\./);
  } finally { delete host.__tradePending; await close(); }
});

test('one scale all campaign never fits the moment while its scale is unread or failed', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  // GER sells ENG 10 steel a day throughout.
  const data: TradeFlowsReply = { branchId: BRANCH.id, decimals: 0, resources: [{ id: 1, token: 'steel', ordinal: 1, name: null, icon: null }],
    countries: ['GER', 'ENG'], reads: [0], known: [span], partial: [], warnings: [], flows: [{ seller: 0, buyer: 1, resource: 1,
      sellerIdentity: '[1,101]', buyerIdentity: '[2,102]', sellerTags: [0], buyerTags: [1], firstHours: [span.firstHour], lastHours: [span.lastHour],
      deliveredRaw: ['10'], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] }],
    countryIdentities: ['GER', 'ENG'].map((tag, index) => ({ tag, runs: [{ ...span, slot: index + 1, countryId: index + 101 }] })) };
  const state: { scaleData?: TradeFlowsReply; scaleError?: Error } = {};
  const host = globalThis as typeof globalThis & { __tradeScale?: typeof state }; host.__tradeScale = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/trade-flows/TradeFlowsWidget.tsx', {
    '/widgets/trade-flows/useTradeFlows.ts': `import { decodeTrade, tradeScope } from './tradeData.ts';
      export const useTradeFlows = (widget, runtime) => {
        const track = runtime.tracks[0], data = ${JSON.stringify(data)}, { scaleData, scaleError } = globalThis.__tradeScale;
        return { track, all: true, resolved: null, groups: {}, data, scaleData, scaleError, retryScale() {}, retry() {},
          scope: tradeScope(decodeTrade(data), widget.countries, null, undefined, track.branch.lastHour, false) };
      };`,
    '/dashboard/playback.ts': `export const usePlaybackHour = clock => clock.getHour();`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryFlagsAt.ts': `export const flagReadMetadata = () => ({});
      export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag, known: true }) });`,
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsRead = () => new Map();',
    '/dashboard/countryFlags.ts': 'export const useCountryFlagsRead = () => new Map();',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456'], surface: '#000000', text: '#ffffff', muted: '#777777', countryColours: 'raw' });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/widgets/trade-flows/TradeDrillView.tsx': 'export default () => null;',
    '/charts/flow/FlowDiagram.tsx': 'export default props => `Ribbon scale ${props.scale}`;',
  });
  try {
    const render = (options: Record<string, unknown>) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'trade', kind: 'trade-flows',
      trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options },
      runtime: { client: {}, tracks: [{ id: 'main', axisOffset: 0, branch: { ...BRANCH, gaps: [] }, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'campaign', countries: [] } } }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ position: BRANCH.lastHour, playing: false, hoursPerSecond: 0 }) } },
      active: true, onDiagnostics() {} } as unknown as WidgetProps));
    const pending = render({});
    assert.match(pending, /aria-label="Loading the campaign-wide ribbon scale…"/);
    assert.doesNotMatch(pending, /Ribbon scale/, 'no diagram sized to the moment instead');
    assert.match(render({ scale: 'moment' }), /Ribbon scale null/, 'fitting the moment needs no campaign scale');
    state.scaleError = new Error('refused');
    const failed = render({});
    assert.match(failed, /Couldn’t load the campaign-wide ribbon scale\./);
    assert.doesNotMatch(failed, /Ribbon scale/);
    assert.match(render({ scale: 'moment' }), /Ribbon scale null/);
    state.scaleError = undefined; state.scaleData = data;
    assert.match(render({}), /Ribbon scale 10\b/);
  } finally { delete host.__tradeScale; await close(); }
});

test('chosen countries the market view cannot place at an hour still being prepared are loading, not unavailable', async () => {
  const state = { groups: pendingGroups() as CountryGroupReply, requests: [] as unknown[], result: null as null | { marketReason: string | null } };
  const host = globalThis as typeof globalThis & { __marketPending?: typeof state }; host.__marketPending = state;
  const { module, close } = await mountModule<{ useTradeFlows: (...args: unknown[]) => { marketReason: string | null } }>('/ui/src/widgets/trade-flows/useTradeFlows.ts', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__marketPending.groups });',
    '/dashboard/products.ts': 'export const useProduct = (_client, request) => { if (request) globalThis.__marketPending.requests.push(request); return { retry() {} }; }; export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
  });
  try {
    const widget = { trackIds: ['t'], countries: { kind: 'tags', tags: ['ENG'] }, time: { mode: 'campaign' }, options: { view: 'market' } };
    const track = { id: 't', source: { id: 'source', revision: '1', metadata: { campaignKey: 'c', countries: [] } }, branch: BRANCH, axisOffset: 0 };
    const runtime = { client: { preparedQuery() {} }, tracks: [track], clock: { getHour: () => PENDING_HOUR } };
    function Probe() { state.result = module.useTradeFlows(widget, runtime, true, PENDING_HOUR); return null; }
    const render = () => { state.requests = []; renderToStaticMarkup(createElement(Probe)); return state.result!.marketReason; };
    // ENG's holders after the prepared hours are not read yet.
    assert.equal(render(), null);
    assert.deepEqual(state.requests, [], 'an unknown choice never becomes an all-countries request');
    // Read, ENG's only holder stopped holding it before the hour shown.
    const final = pendingGroups({ final: true });
    state.groups = { ...final, countryIdentities: final.countryIdentities.map(country => ({ ...country,
      identities: country.identities.map(run => ({ ...run, lastHour: KNOWN_HOUR })) })) };
    assert.equal(render(), 'Selected countries are unavailable at this time.');
  } finally { delete host.__marketPending; await close(); }
});

test('trade of a group with no member found yet says what the moment was once read, and loads until then', async () => {
  const state = { groups: { ...pendingGroups({ humans: [] }), unobserved: { humans: [{ firstHour: 140, lastHour: 150 }], factions: [], wars: [] } } as CountryGroupReply, hour: 0 };
  const host = globalThis as typeof globalThis & { __tradeGap?: typeof state }; host.__tradeGap = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/trade-flows/TradeFlowsWidget.tsx', {
    // No member to ask for: no request, no reply.
    '/widgets/trade-flows/useTradeFlows.ts': `import { resolveCountries } from '../../dashboard/groups.ts';
      export const useTradeFlows = (widget, runtime) => {
        const track = runtime.tracks[0], { groups } = globalThis.__tradeGap;
        const resolved = resolveCountries(widget.countries, [], groups, track.branch.lastHour, Number.MAX_SAFE_INTEGER);
        if (resolved.tags.length) throw new Error('no member was found, so nothing is asked');
        return { track, all: false, resolved, groups: { data: groups, retry() {} }, data: undefined, scope: null, retry() {} };
      };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__tradeGap.hour;',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryFlagsAt.ts': `export const flagReadMetadata = () => ({});
      export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag, known: true }) });`,
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsRead = () => new Map();',
    '/dashboard/countryFlags.ts': 'export const useCountryFlagsRead = () => new Map();',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456'], surface: '#000000', text: '#ffffff', muted: '#777777', countryColours: 'raw' });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/widgets/trade-flows/TradeDrillView.tsx': 'export default () => null;',
    '/charts/flow/FlowDiagram.tsx': 'export default props => props.message ?? "Trade diagram";',
  });
  try {
    const render = (hour: number) => { state.hour = hour; return renderToStaticMarkup(createElement(module.default, { widget: { id: 'trade', kind: 'trade-flows',
      trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' }, options: {} },
      runtime: { client: {}, tracks: [{ id: 'main', axisOffset: 0, branch: BRANCH, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'campaign', countries: [] } } }],
        clock: { getHour: () => hour, getSnapshot: () => ({ position: hour, playing: false, hoursPerSecond: 0 }) } },
      active: true, onDiagnostics() {} } as unknown as WidgetProps)); };
    assert.match(render(145), /Group membership is unavailable at this time\./);
    assert.match(render(100), /This group has no members at this moment\./);
    const later = render(170);
    assert.match(later, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(later, /unavailable|no members/);
  } finally { delete host.__tradeGap; await close(); }
});

test('while playback reads its newest hour, the hour last read stays drawn with its date', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const data: TradeFlowsReply = { branchId: BRANCH.id, decimals: 0, resources: [{ id: 1, token: 'steel', ordinal: 1, name: null, icon: null }],
    countries: ['GER', 'ENG'], reads: [0], known: [span], partial: [], warnings: [], flows: [{ seller: 0, buyer: 1, resource: 1,
      sellerIdentity: '[1,101]', buyerIdentity: '[2,102]', sellerTags: [0], buyerTags: [1], firstHours: [span.firstHour], lastHours: [span.lastHour],
      deliveredRaw: ['10'], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] }],
    countryIdentities: ['GER', 'ENG'].map((tag, index) => ({ tag, runs: [{ ...span, slot: index + 1, countryId: index + 101 }] })) };
  const held = BRANCH.lastHour - 10;
  const state: { reachedHour?: number } = {};
  const host = globalThis as typeof globalThis & { __tradeHeld?: typeof state }; host.__tradeHeld = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/trade-flows/TradeFlowsWidget.tsx', {
    '/widgets/trade-flows/useTradeFlows.ts': `import { decodeTrade, tradeScope } from './tradeData.ts';
      export const useTradeFlows = (widget, runtime) => {
        const track = runtime.tracks[0], data = ${JSON.stringify(data)};
        return { track, all: true, resolved: null, groups: {}, data, scaleData: data, retryScale() {}, retry() {},
          hasExactHour: hour => hour === ${held}, reachedHour: globalThis.__tradeHeld.reachedHour,
          scope: tradeScope(decodeTrade(data), widget.countries, null, undefined, track.branch.lastHour, false) };
      };`,
    '/dashboard/playback.ts': `export const usePlaybackHour = clock => clock.getHour();`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryFlagsAt.ts': `export const flagReadMetadata = () => ({});
      export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag, known: true }) });`,
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsRead = () => new Map();',
    '/dashboard/countryFlags.ts': 'export const useCountryFlagsRead = () => new Map();',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123456'], surface: '#000000', text: '#ffffff', muted: '#777777', countryColours: 'raw' });
      export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/widgets/trade-flows/TradeDrillView.tsx': 'export default () => null;',
    '/charts/flow/FlowDiagram.tsx': 'export default props => `Diagram at ${props.hour}`;',
  });
  try {
    const render = () => renderToStaticMarkup(createElement(module.default, { widget: { id: 'trade', kind: 'trade-flows',
      trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: { scale: 'moment' } },
      runtime: { client: {}, tracks: [{ id: 'main', axisOffset: 0, branch: { ...BRANCH, gaps: [] }, source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'campaign', countries: [] } } }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ position: BRANCH.lastHour, playing: true, hoursPerSecond: 24 }) } },
      active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assert.match(render(), /aria-label="Loading trade amounts…"/, 'nothing read yet loads');
    state.reachedHour = held;
    const lagging = render();
    assert.match(lagging, new RegExp(`Diagram at ${held}\\b`));
    assert.ok(lagging.includes(`Per day · As of ${dayLabel(held)}`), 'the held hour is dated, never shown as the playhead’s');
  } finally { delete host.__tradeHeld; await close(); }
});
