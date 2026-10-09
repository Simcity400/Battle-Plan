import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement, isValidElement } from 'react';
import type { ComponentType, ReactElement, ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { LogListProps } from '../charts/log/LogList';
import type { LogRow } from '../charts/log/logData';
import type { WidgetProps } from './definitions';
import type { SpendingRow } from './politicsSpending';
import type { PoliticsLedgerReply } from '../generated/dashboard';

for (const currency of ['Political', 'Command'] as const) test(`${currency} spending flags retain native row dates and completed reasons in the registered renderer`, async () => {
  const data: PoliticsLedgerReply = { branchId: 'branch', tag: 'native actor', decimals: 5, mutations: [], governments: [], daily: [], dailyCosts: [], coverage: [], warnings: [],
    spending: [{ id: 'payment', hour: 10, readHour: 12, sequence: '1', ordinal: 0, countryTag: 'native actor', countryName: 'Recorded actor', currency,
      category: 'mod category', itemId: 'mod action', chargeRaw: '100000', realizedDeltaRaw: '-100000', covered: true, occurrences: 1,
      changes: [{ readHour: 12, chargeRaw: '100000', realizedDeltaRaw: '-100000', covered: true, occurrences: 1 }] }],
    spendItems: [{ id: 'mod action', groupId: 'mod action', name: 'Recorded action', action: 'Recorded action', icon: null, tooltipToken: null,
      character: null, targetTag: 'native target', targetName: 'Recorded target', unknown: null }],
    spendCategories: [{ id: 'mod category', name: 'Recorded category', identified: true }], ideaTooltips: null, commandHolds: [],
    spendingCoverage: [{ readHour: 7, sinceHour: 3, throughHour: 8, political: false, command: false, reason: 'Synthetic native stream refusal' }] };
  const state = { visible: [] as readonly LogRow<SpendingRow>[], list: undefined as LogListProps<SpendingRow> | undefined,
    wanted: [] as { tag: string; hour: number }[], clock: 100, pending: true, reason: undefined as string | undefined,
    url: undefined as string | undefined, failedImage: undefined as string | undefined };
  const host = globalThis as typeof globalThis & { __spendingFlags?: typeof state;
    __spendingFlagState?: () => [readonly LogRow<SpendingRow>[], (rows: readonly LogRow<SpendingRow>[]) => void];
    __spendingImageState?: () => [string | undefined, (url: string) => void] };
  host.__spendingFlags = state;
  host.__spendingFlagState = () => [state.visible, rows => { state.visible = rows; }];
  host.__spendingImageState = () => [state.failedImage, url => { state.failedImage = url; }];
  const replacements: Record<string, string> = {
    '/dashboard/usePoliticsSpending.ts': `export const usePoliticsSpending = (_widget, runtime) => ({track: runtime.tracks[0], tags: ['native actor', 'native target'],
      data: [${JSON.stringify(data)}, {...${JSON.stringify(data)}, tag: 'native target', spending: [], spendingCoverage:
        [{readHour: 0, sinceHour: 0, throughHour: 100, political: true, command: true, reason: null}]}], ready: true, groupWarnings: [],
      selection: {matches: () => true, membership: new Map(['native actor', 'native target'].map(tag => [tag, [{firstHour: 0, lastHour: 100}]])),
        unobserved: [{firstHour: 23, lastHour: 23}]}});
      export const useSpendingChoices = () => ({}); export const useSpendingIdeaTooltips = () => undefined;`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = (_client, _track, wanted) => {globalThis.__spendingFlags.wanted = wanted;
      return {labelAt: hour => tag => ({name: 'Dated ' + tag + ' at ' + hour, flag: globalThis.__spendingFlags.url,
        flagPending: globalThis.__spendingFlags.pending, flagReason: globalThis.__spendingFlags.reason})}; };`,
    '/dashboard/products.ts': 'export const loadProduct = () => Promise.resolve({});',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => globalThis.__spendingFlags.clock;',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({categories: ['#123456'], muted: '#888888'});`,
    '/charts/log/LogList.tsx': 'export default props => {globalThis.__spendingFlags.list = props; return null;};',
    '/dashboard/PoliticsCommandHeld.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{name: 'spending-flag-adapter', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/dashboard/PoliticsSpendingLog.tsx')) return source.replace(/useState(?:<[\s\S]*?>)?\(NO_ROWS\)/, 'globalThis.__spendingFlagState()');
        if (path.endsWith('/components/CountryFlag.tsx')) return source.replace('useState<string>()', 'globalThis.__spendingImageState()');
      },
    }]});
  try {
    const kind = currency === 'Political' ? 'political-power-spending' : 'command-power-spending';
    const {default: registered} = await vite.ssrLoadModule(`/ui/src/widgets/${kind}/index.ts`);
    const Widget = registered.render as ComponentType<WidgetProps>;
    const track = {id: 'track', axisOffset: 0, source: {id: 'source', revision: '1', metadata: {countries: [], branches: []}},
      branch: {id: 'branch', firstHour: 0, lastHour: 100, gaps: []}};
    const props = {widget: {id: kind, kind, configVersion: 1, trackIds: ['track'], countries: {kind: 'tags', tags: ['native actor', 'native target']},
      time: {mode: 'follow'}, options: {totals: false, chips: false, artwork: false}}, runtime: {client: {}, tracks: [track],
      clock: {getHour: () => state.clock}}, active: true, onDiagnostics() {}} as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(Widget, props));
    const wanted = () => state.wanted;
    const rows = () => { const rows = state.list?.rows; assert.ok(rows); return rows; };
    render(); assert.deepEqual(wanted(), []);
    const spend = rows().find(row => row.item.kind === 'spend')!;
    const nativeGap = rows().find(row => row.item.kind === 'gap' && row.item.gap.tag)!;
    const groupGap = rows().find(row => row.item.kind === 'gap' && !row.item.gap.tag)!;
    state.list!.onVisible!([spend, nativeGap, groupGap]); render();
    assert.deepEqual(wanted(), [{hour: 10, tag: 'native actor'}, {hour: 10, tag: 'native target'}, {hour: 3, tag: 'native actor'}]);
    const markup = (row: LogRow<SpendingRow>) => renderToStaticMarkup(createElement('div', null, state.list!.renderRow(row)));
    assert.match(markup(spend), /Flag loading/); assert.match(markup(nativeGap), /Flag loading/);
    assert.doesNotMatch(markup(groupGap), /country-flag|Flag loading/);
    const tip = () => state.list!.tooltip!(spend)[0];
    assert.deepEqual(tip().rows[0].countryFlag, {name: 'Dated native actor at 10', flag: undefined, flagPending: true, flagReason: undefined});
    assert.equal(tip().rows[0].value, 'Recorded actor');
    assert.equal(tip().rows[0].countryFlag?.flagPending, true);
    assert.equal(tip().rows.find(row => row.label === 'Target')!.value, 'Recorded target');
    assert.equal(state.list!.tooltip!(nativeGap)[0].rows[0].countryFlag?.flagPending, true);
    assert.equal(state.list!.tooltip!(groupGap)[0].rows[0].countryFlag, undefined);
    state.pending = false; state.reason = 'Synthetic recorded-stack flag refusal'; render();
    assert.match(markup(spend), /Flag not available: Synthetic recorded-stack flag refusal/);
    assert.doesNotMatch(markup(spend), /Flag loading/);
    assert.equal(tip().rows.find(row => row.label === 'Target')!.countryFlag?.flagReason, state.reason);
    state.reason = 'The flag request failed.'; render(); assert.match(markup(nativeGap), /The flag request failed/);
    state.reason = 'No readable flag was returned for this country at this hour.'; render(); assert.match(markup(spend), /No readable flag/);
    for (const filterMode of ['show', 'hide']) {
      props.widget.options = { totals: false, chips: true, artwork: false, filterMode, categories: 'mod:20:category' };
      const filtered = render();
      assert.match(filtered, new RegExp(`aria-label="${filterMode === 'show' ? 'Shown' : 'Hidden'} spending categories"`));
      assert.match(filtered, /aria-pressed="true"/);
      assert.match(filtered, /class="chip-dot" style="background:#123456"/);
      assert.equal(rows().filter(row => row.item.kind === 'spend').length, filterMode === 'show' ? 1 : 0);
      assert.equal(rows().filter(row => row.item.kind === 'gap').length, 2, 'filtering keeps unknown coverage visible');
    }
    props.widget.options = { totals: false, chips: false, artwork: false };
    state.url = 'data:image/png;base64,synthetic'; state.reason = undefined; render();
    const findFlag = (node: ReactNode): ReactElement | undefined => {
      if (Array.isArray(node)) return node.map(findFlag).find(Boolean);
      if (!isValidElement<{children?: ReactNode}>(node)) return undefined;
      if (typeof node.type === 'function' && node.type.name === 'CountryFlagUrl') return node;
      return findFlag(node.props.children);
    };
    const flag = findFlag(state.list!.renderRow(spend))!;
    const image = () => (flag.type as (props: unknown) => ReactElement)(flag.props);
    (image() as ReactElement<{onError: () => void}>).props.onError();
    assert.match(renderToStaticMarkup(image()), /The flag image could not be displayed/);
    state.clock = 99; state.url = undefined; state.pending = true; render();
    assert.deepEqual(wanted().map(row => row.hour), [10, 10, 3], 'playback cannot redate the native actor or target');
    state.list!.onVisible!([nativeGap]); render(); assert.deepEqual(wanted(), [{hour: 3, tag: 'native actor'}]);
    state.list!.onVisible!([]); render(); assert.deepEqual(wanted(), []);
  } finally {await vite.close(); delete host.__spendingFlags; delete host.__spendingFlagState; delete host.__spendingImageState;}
});
