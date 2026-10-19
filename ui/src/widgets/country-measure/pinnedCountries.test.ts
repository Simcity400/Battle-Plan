import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, isValidElement } from 'react';
import type { ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { WidgetCountryChipProps, WidgetProps } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import type { CountryGroupReply, CountryStabilityReply, LossesReply, MeasureReply, PoliticsLedgerReply, StabilityPartsReply } from '../../generated/dashboard';

const histories = [
  { tag: 'X', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }, { firstHour: 10, lastHour: 20, slot: 2, countryId: 20 }] },
  { tag: 'Y', identities: [{ firstHour: 10, lastHour: 14, slot: 1, countryId: 10 }, { firstHour: 16, lastHour: 20, slot: 1, countryId: 10 }] },
];
const groups: CountryGroupReply = { branchId: 'b', countryIdentities: histories, warnings: [], humans: [], factions: [], wars: [],
  warStarts: [], capitulations: [], identityChanges: [], unobserved: { humans: [], factions: [], wars: [] } };
const runs = (tag: string) => histories.find(history => history.tag === tag)!.identities.map(identity => ({ firstHour: identity.firstHour,
  lastHour: identity.lastHour, startsSegment: true, valueRaw: identity.countryId === 20 ? '99000' : tag === 'Y' ? '20000' : '10000' }));
const values: CountryStabilityReply = { branchId: 'b', decimals: 5, warnings: [], series: histories.map(history => ({ tag: history.tag,
  recorded: true, carried: [], breakdown: null, maxCommandPower: [], stability: runs(history.tag), warSupport: runs(history.tag) })) };
const balance: MeasureReply = { branchId: 'b', measure: 'politicalPower', decimals: 5, warnings: [],
  series: histories.map(history => ({ tag: history.tag, drops: [], runs: runs(history.tag) })) };
const parts = histories.map<StabilityPartsReply>(history => ({ tag: history.tag, branchId: 'b', decimals: 5, warnings: [],
  stability: { observed: history.identities, parts: [{ id: 'base', term: 'base', line: null, runs: runs(history.tag) }] },
  warSupport: { observed: history.identities, parts: [{ id: 'base', term: 'base', line: null, runs: runs(history.tag) }] } }));
const ledgers = histories.map<PoliticsLedgerReply>(history => ({ tag: history.tag, branchId: 'b', decimals: 5, warnings: [],
  spending: [], spendItems: [], spendCategories: [], commandHolds: [], ideaTooltips: null,
  spendingCoverage: history.identities.map(identity => ({ sinceHour: identity.firstHour, throughHour: identity.lastHour,
    readHour: identity.firstHour, political: true, command: true, reason: null })),
  daily: history.tag === 'X' ? [10, 20].map(country => ({ country, instance: country, epoch: '1', dayHour: 0, readHour: 0,
    ppGrossRaw: '100001', ppCostRaw: '0', ppIncomeRaw: '100001', ppRealizedRaw: '100001', ppSequence: '1',
    cpIncomeRaw: '100001', cpRealizedRaw: '100001', cpSequence: '1' })) : [] }));
const losses: LossesReply = { tag: 'X', branchId: 'b', amountScale: 100000, landSource: 'destruction', landLowerBound: null,
  retainedMonths: null, destructionCountedFrom: 0, warnings: [], coverage: [], designs: [],
  destructionCoverage: [{ firstHour: 0, lastHour: 20, reason: null }], causes: [{ source: { kind: 'sunkConvoys' }, domain: 'naval',
    name: { key: null, text: 'Sunk convoys', template: null }, loss: true, unknown: null, unknownSpans: [], group: null,
    leaves: [{ design: null, lowerBoundFrom: null, points: [{ hour: 0, totalRaw: '100000' }, { hour: 10, totalRaw: '200000' }] }] }] };

function button(node: ReactNode, label: string): ReactElement<{ onClick: () => void }> | null {
  if (Array.isArray(node)) return node.map(child => button(child, label)).find(Boolean) ?? null;
  if (!isValidElement<{ children?: ReactNode }>(node)) return null;
  return node.type === 'button' && node.props.children === label ? node as ReactElement<{ onClick: () => void }> : button(node.props.children, label);
}

test('registered politics and losses pins retain native holders through local picks, Follow and remount', async () => {
  type Cell = { value?: unknown; deps?: readonly unknown[] };
  const state = { cells: [] as Cell[], cursor: 0, effects: [] as (() => unknown)[], clock: 9,
    header: undefined as { hour: number | null; reason: string | null; label: { name: string; flag?: string } | null } | undefined,
    detail: undefined as { hour: number; onPick: (hour: number) => void; reply?: StabilityPartsReply; index?: { reply: PoliticsLedgerReply } } | undefined,
    charts: [] as TimeChartProps[], requests: [] as { product: string; params: Record<string, unknown> }[],
    node: undefined as ReactNode };
  const global = globalThis as typeof globalThis & { __pinnedCountries?: typeof state; __pinnedCountryHook?: (kind: string, ...args: unknown[]) => unknown };
  global.__pinnedCountries = state;
  global.__pinnedCountryHook = (kind, ...args) => {
    const cell = state.cells[state.cursor++] ??= {};
    if (kind === 'useMemo' || kind === 'useCallback' || kind === 'useEffect' || kind === 'useLayoutEffect') {
      const [factory, deps] = args as [() => unknown, readonly unknown[]];
      if (!cell.deps || cell.deps.length !== deps.length || cell.deps.some((value, index) => !Object.is(value, deps[index]))) {
        cell.deps = [...deps];
        if (kind === 'useMemo') cell.value = factory();
        else if (kind === 'useCallback') cell.value = factory;
        else state.effects.push(factory);
      }
      return cell.value;
    }
    if (!('value' in cell)) cell.value = kind === 'useReducer' ? args[1] : kind === 'useRef' ? { current: args[0] }
      : typeof args[0] === 'function' ? (args[0] as () => unknown)() : args[0];
    if (kind === 'useRef') return cell.value;
    return [cell.value, (next: unknown) => { cell.value = kind === 'useReducer'
      ? (args[0] as (before: unknown, action: unknown) => unknown)(cell.value, next)
      : typeof next === 'function' ? (next as (before: unknown) => unknown)(cell.value) : next; }];
  };
  const leaves: Record<string, string> = {
    // Complete numeric fixtures probe the registered body's native holder
    // binding and dated headers.
    // Prepared query transport and bounded geometry have separate real-hook fixtures.
    '/dashboard/products.ts': `const values = ${JSON.stringify(values)}, balance = ${JSON.stringify(balance)}, parts = ${JSON.stringify(parts)},
      ledgers = ${JSON.stringify(ledgers)}, losses = ${JSON.stringify(losses)};
      export const useProduct = (_client, request, active, validate) => {
        const state = globalThis.__pinnedCountries; if (!request || !active) return {retry() {}}; state.requests.push(request);
        const reply = request.product === 'country-stability' ? {...values, series: values.series.filter(row => request.params.tags.includes(row.tag))}
          : request.product === 'country-measure' ? {...balance, measure: request.params.measure}
          : request.product === 'stability-parts' ? parts.find(row => row.tag === request.params.tag)
          : request.product === 'politics-ledger' ? ledgers.find(row => row.tag === request.params.tag)
          : request.product === 'equipment-losses' ? {...losses, landCoverage:losses.coverage, preparedReads:request.params.reads,
            preparedHeadHour:request.params.headHour, preparedDetailHour:request.params.detailHour,
            preparedFrom:request.params.from, preparedTo:request.params.to, preparedWidth:request.params.width} : undefined;
        return {data: reply && validate(reply), retry() {}};
      }; export const useProducts = (client, requests, active, validate) => ({data: requests.length
        ? requests.map(request => useProduct(client, request, active, validate).data) : undefined, retry() {}});
      export const loadProduct = (_client, request) => Promise.resolve({...losses, landCoverage:losses.coverage, preparedReads:request.params.reads,
        preparedHeadHour:request.params.headHour, preparedDetailHour:request.params.detailHour,
        preparedFrom:request.params.from, preparedTo:request.params.to, preparedWidth:request.params.width});`,
    '/dashboard/useCountryGroups.ts': `const data = ${JSON.stringify(groups)}; export const useCountryGroups = (_runtime, _track, active) => ({data: active ? data : undefined, retry() {}});`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__pinnedCountries.clock; export const usePlaybackSpeed = () => 24;',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({revision: 0,
      labelAt: hour => tag => ({known: true, name: tag + '@' + hour, flag: 'flag:' + tag + '@' + hour})});`,
    '/dashboard/DatedCountryChip.tsx': 'export default props => {globalThis.__pinnedCountries.header = props; return null;};',
    '/dashboard/campaignSettings.ts': 'export const useCampaignSettings = () => ({campaigns: {}});',
    '/dashboard/countryLooks.ts': `export const useCountryLooks = () => ({name: tag => tag, color: () => [20, 50, 80], flag() {}, flagPending() {}, flagReason() {}, revision: 0});`,
    '/widgets/national-focus/useFocusArtwork.ts': 'export const useFocusText = () => ({read: new Map(), warnings: []});',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesAt = () => new Map(); export const useCountryNamesRead = () => new Map();',
    '/dashboard/useCountryLineStyles.ts': `export const useCountryLineStyles = () => ({known: new Map([['X', {tag: 'X', color: [20, 50, 80]}]]), data: {warnings: []}});`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({categories: ['#123456'], muted: '#888888', surface: '#000000', countryColours: 'game'}); export const useViewportHeight = () => 1000;`,
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({open() {}, menu: null});',
    '/charts/TimeChart.tsx': 'export default props => {globalThis.__pinnedCountries.charts.push(props); return null;};',
    '/widgets/country-measure/PowerDrill.tsx': 'export default props => {globalThis.__pinnedCountries.detail = props; return null;};',
    '/widgets/stability/StabilityDrill.tsx': `export const MEASURE_NAMES = {stability: 'Stability', warSupport: 'War support'}; export default props => {globalThis.__pinnedCountries.detail = props; return null;};`,
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/drill/DrillParts.tsx': 'export default () => null;',
    '/charts/table/Table.tsx': 'export default () => null;',
    '/charts/sunburst/Sunburst.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'pinned-native-country', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/'), leaf = Object.entries(leaves).find(([suffix]) => path.endsWith(suffix));
      if (leaf) return leaf[1];
      if (path.endsWith('/EquipmentLossesWidget.tsx')) {
        // The registered wrapper renders the loss body beside retained Combat Log
        // details. Capture the body's JSX so its real Follow control stays reachable.
        const bodyReturn = 'return <div ref={anchor} className="equipment-losses">';
        assert.equal(source.split(bodyReturn).length - 1, 1, 'equipment-loss body capture target occurs exactly once');
        source = source.replace(bodyReturn, bodyReturn.replace('return ', 'return globalThis.__pinnedCountries.node = '));
      }
      if (['MeasureWidget', 'StabilityWidget', 'EquipmentLossesWidget'].some(name => path.endsWith('/' + name + '.tsx'))) return source
        .replace("import { createPortal } from 'react-dom';", 'const createPortal = child => child;')
        .replace(/\b(useMemo|useCallback|useState|useReducer|useRef|useEffect|useLayoutEffect)(?:<[\s\S]*?>)?\(/g,
          (_match, kind) => `globalThis.__pinnedCountryHook(${JSON.stringify(kind)}, `);
    } }] });
  try {
    await vite.ssrLoadModule('/ui/src/widgets/definitions.ts');
    const holder = { campaign: 'campaign', branch: 'b', tag: 'X', originOffset: 5, slot: 1, countryId: 10 };
    for (const [kind, show, option] of [['political-power', 'income', 'powerCountry'], ['command-power', 'spending', 'powerCountry'],
      ['stability', 'stability', 'stabilityCountry'], ['equipment-losses', 'loss-details', 'lossCountry']]) {
      const renderer = (await vite.ssrLoadModule('/ui/src/widgets/' + kind + '/index.ts')).default;
      const Widget = renderer.render as (props: WidgetProps) => ReactNode;
      const widget = { id: kind, kind, title: kind, configVersion: 1, trackIds: ['t'], countries: {kind: 'tags', tags: ['X']},
        time: {mode: 'pin', windows: {t: {firstOffset: 12, lastOffset: 12}}}, options: {show, [option]: JSON.stringify(holder)} } as WidgetSpec;
      const track = {id: 't', axisOffset: 3, branch: {id: 'b', firstHour: 0, lastHour: 20}, source: {id: 'source', revision: '1',
        metadata: {campaignKey: 'campaign', countries: [], branches: []}}};
      const props = {widget: JSON.parse(JSON.stringify(widget)), runtime: {client: {}, tracks: [track], clock: {getHour: () => state.clock,
        getSnapshot: () => ({playing: false, hoursPerSecond: 24})}}, active: true, countrySlot: {} as HTMLElement, onDiagnostics() {}} as unknown as WidgetProps;
      const chip = (renderer.countryChip as (props: WidgetCountryChipProps) => ReactElement)({widget, runtime: props.runtime, active: true, onSlot() {}});
      assert.equal(chip.type, 'span', kind + ' registers a country slot');
      // PP/CP registration supplies the common body's config through an element.
      const body = kind === 'political-power' || kind === 'command-power' ? Widget(props) as ReactElement<{ config: unknown }> : null;
      const renderBody = () => {
        state.cursor = 0; state.effects = []; state.charts = []; state.detail = undefined; state.header = undefined;
        // Rendering the registered body gives every shared hook its
        // React dispatcher, including inactive prepared-query hooks.
        const Probe = () => {
          state.node = body ? (body.type as (props: unknown) => ReactNode)({...props, config: body.props.config}) : Widget(props);
          return state.node;
        };
        const html = renderToStaticMarkup(createElement(Probe));
        for (const effect of state.effects) effect();
        return html;
      };
      state.cells = []; state.clock = 9; renderBody(); renderBody();
      assert.equal(state.header!.hour, 12, kind);
      assert.equal(state.header!.label!.name, 'Y@12');
      assert.equal(state.header!.label!.flag, 'flag:Y@12');
      const pick = (hour: number) => kind === 'equipment-losses' ? state.charts.at(-1)!.onPick!(hour) : state.detail!.onPick(hour);
      pick(5); await new Promise(resolve => setImmediate(resolve)); renderBody();
      assert.equal(state.header!.hour, 5);
      assert.equal(state.header!.label!.name, 'X@5');
      const follow = button(state.node, 'Follow playback');
      assert.ok(follow, kind + ' renders Follow after a local pick');
      follow.props.onClick(); state.clock = 14; renderBody();
      assert.equal(state.header!.hour, 17, 'Follow aligns the body to its source');
      assert.equal(state.header!.label!.name, 'Y@17', 'a reused X never replaces the selected actor');
      if (kind === 'stability') assert.deepEqual(state.detail!.reply!.stability.parts[0].runs.map(run => run.valueRaw), ['10000', '20000', '20000']);
      if (kind === 'political-power' || kind === 'command-power') assert.deepEqual(state.detail!.index!.reply.daily!.map(row => row.country), [10]);
      if (kind === 'equipment-losses') assert.deepEqual((state.requests.filter(request => request.product === 'equipment-losses').at(-1)!.params.selection as {country:unknown}).country,
        {slot: 1, countryId: 10, originHour: 5});
      state.clock = 12; renderBody();
      assert.equal(state.header!.hour, 15);
      assert.equal(state.header!.label, null, 'an identity gap keeps a neutral header');
      assert.match(state.header!.reason!, kind === 'political-power' || kind === 'command-power' ? /Country unavailable at this time/ : kind === 'stability' ? /Selected country unavailable/ : /Country unavailable at this time/);
      state.clock = 9; state.cells = []; renderBody(); renderBody();
      assert.equal(state.header!.label!.name, 'Y@12', 'remount restores the native origin');
      pick(5); renderBody();
      props.runtime.client = {} as WidgetProps['runtime']['client']; renderBody();
      assert.equal(state.header!.hour, 12, 'a client change masks the previous local pick before effects');
      props.widget.options = {...props.widget.options, [option]: JSON.stringify({...holder, countryId: 20})};
      assert.match(renderBody(), kind === 'political-power' || kind === 'command-power' ? /Country unavailable at this time/ : kind === 'stability' ? /Selected country unavailable/ : /Country unavailable at this time/);
      assert.equal(state.header!.label, null);
      props.widget.options = {...props.widget.options, [option]: 'malformed'};
      assert.match(renderBody(), kind === 'political-power' || kind === 'command-power' ? /Saved country selection unavailable/ : kind === 'stability' ? /Selected country unavailable/ : /Choose a country again/);
      props.widget.options = {...props.widget.options, [option]: null};
      assert.match(renderBody(), kind === 'political-power' || kind === 'command-power' ? /Saved country selection unavailable/ : kind === 'stability' ? /Selected country unavailable/ : /Choose a country again/);
      assert.equal(state.header!.label, null, 'a present null holder must not bind the reused tag');
      assert.match(state.header!.reason!, kind === 'political-power' || kind === 'command-power' ? /Saved country selection unavailable/ : kind === 'stability' ? /Selected country unavailable/ : /Choose a country again/);
      props.widget.options = {show}; state.cells = []; renderBody(); renderBody();
      assert.equal(state.header!.label, null, 'old pins without holder data retain a neutral header');
    }
  } finally { await vite.close(); delete global.__pinnedCountries; delete global.__pinnedCountryHook; }
});
