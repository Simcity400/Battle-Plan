import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { SunburstProps } from '../../charts/sunburst/Sunburst';
import { groupTree, sliceTooltip } from '../../charts/sunburst/layout.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { IdeaInfo, PoliticsLedgerReply } from '../../generated/dashboard';

test('registered spending breakdown uses supplying reads, exact charges and the current sunburst tooltip contract', async () => {
  const payment = (id: string, hour: number, readHour: number, chargeRaw: string) => ({id, hour, readHour, sequence: id === 'first' ? '1' : '2', ordinal: 0,
    countryTag: 'synthetic actor', countryName: null, currency: 'Political' as const, category: 'mod category', itemId: 'mod action',
    chargeRaw, realizedDeltaRaw: '-50000', covered: true, occurrences: 1,
    changes: [{readHour, chargeRaw, realizedDeltaRaw: '-50000', covered: true, occurrences: 1}]});
  const info: IdeaInfo = {token: 'idea/mod', category: null, slot: null, law: true, designer: false, effects: [],
    targeted: [{target: 'other actor', instant: false, lines: [], tooltips: []}], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null};
  const data: PoliticsLedgerReply = {branchId: 'branch', tag: 'synthetic actor', decimals: 5, mutations: [], governments: [], daily: [], dailyCosts: [],
    coverage: [], warnings: [], spending: [payment('first', 10, 12, '150000'), payment('second', 20, 22, '100000')],
    spendItems: [{id: 'mod action', groupId: 'mod action', name: 'Recorded action', action: 'Recorded action', icon: null, tooltipToken: 'idea/mod',
      character: null, targetTag: null, targetName: null, unknown: null}], spendCategories: [{id: 'mod category', name: 'Recorded category', identified: true}],
    ideaTooltips: {ideas: [info], artwork: [{token: 'idea/mod', name: 'Recorded action', description: null, baseDurationDays: null, icon: null}],
      modifiers: [], texts: []}, commandHolds: [], spendingCoverage: [
      {readHour: 0, sinceHour: 0, throughHour: 24, political: true, command: true, reason: null},
      {readHour: 30, sinceHour: 25, throughHour: 26, political: false, command: true, reason: 'Synthetic stream gap'},
    ]};
  const state = {data: [data] as PoliticsLedgerReply[] | undefined, clock: 11, subscribed: false,
    chart: undefined as SunburstProps | undefined, namedAt: [] as number[]};
  const host = globalThis as typeof globalThis & {__spendingBreakdown?: typeof state};
  host.__spendingBreakdown = state;
  const replacements: Record<string, string> = {
    '/dashboard/usePoliticsSpending.ts': `export const usePoliticsSpending = (_widget, runtime) => ({track: runtime.tracks[0], tags: ['synthetic actor'],
      data: globalThis.__spendingBreakdown.data, ready: true, groupWarnings: [], retry() {}});
      export const useSpendingIdeaTooltips = () => ({infos: new Map(globalThis.__spendingBreakdown.data[0].ideaTooltips.ideas.map(info => [info.token, info])),
        art: new Map(), sources: {formats: new Map(), text: () => null, countryName: tag => tag, nameAt: (tag, hour) => {
          globalThis.__spendingBreakdown.namedAt.push(hour); return 'Dated ' + tag + ' at ' + hour; }}});`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = (_clock, active) => {globalThis.__spendingBreakdown.subscribed = active;};',
    '/charts/sunburst/Sunburst.tsx': 'export default props => {globalThis.__spendingBreakdown.chart = props; return null;};',
  };
  const vite = await createServer({configFile: false, optimizeDeps: {noDiscovery: true},
    server: {middlewareMode: true, watch: null, hmr: false, ws: false}, plugins: [{name: 'spending-breakdown-adapter', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      },
    }]});
  try {
    const {default: registered} = await vite.ssrLoadModule('/ui/src/widgets/political-power-spending-breakdown/index.ts');
    const Widget = registered.render as ComponentType<WidgetProps>;
    const track = {id: 'track', axisOffset: 0, source: {id: 'source', revision: '1', metadata: {countries: [], branches: []}},
      branch: {id: 'branch', firstHour: 0, lastHour: 100, gaps: []}};
    const props = {widget: {id: 'spending', kind: 'political-power-spending-breakdown', configVersion: 1, trackIds: ['track'],
      countries: {kind: 'tags', tags: ['synthetic actor']}, time: {mode: 'follow'}, options: {growth: 'fill'}}, runtime: {client: {}, tracks: [track],
      clock: {getHour: () => state.clock, getSnapshot: () => ({hoursPerSecond: 24})}}, active: true, onDiagnostics() {}} as unknown as WidgetProps;
    const render = () => {state.chart = undefined; return renderToStaticMarkup(createElement(Widget, props));};
    assert.match(render(), /No political power spending recorded/);
    assert.equal(state.chart, undefined, 'a native action is unavailable until its supplying read');
    state.clock = 12; render();
    assert.equal(state.subscribed, true);
    const chart = state.chart!;
    assert.equal(chart.settings.growth, 'fill');
    assert.deepEqual(chart.levels, [true, true]);
    assert.equal(chart.items.find(item => item.present)!.exact, 150000n, 'the evaluated charge differs from the realized balance delta');
    assert.equal(chart.items.find(item => item.path[0].id === 'coverage-gap')!.present, false);
    assert.equal(chart.footnote, undefined);
    const tree = groupTree(chart.items, chart.root, chart.levels);
    const category = tree.children.find(node => node.key === 'mod category')!;
    const leaf = category.children[0];
    const tooltip = chart.tooltip!(leaf, tree, chart.text);
    assert.equal(tooltip[0].title, 'Recorded action');
    assert.ok(tooltip[0].rows.some(row => row.label === 'Spent' && row.value === '1.5 PP'));
    assert.ok(tooltip[0].rows.some(row => row.label === 'Most recent read' && row.value === formatDateTime(12)));
    assert.ok(tooltip[0].rows.some(row => row.label === 'Against Dated other actor at 10:'));
    assert.deepEqual(state.namedAt, [10, 10], 'each tooltip form names targets at the native action, apart from the read and playhead');
    assert.deepEqual(chart.tooltip!(category, tree, chart.text), sliceTooltip(category, tree, chart.text));
    const folded = {...leaf, other: true};
    assert.deepEqual(chart.tooltip!(folded, tree, chart.text), sliceTooltip(folded, tree, chart.text));
    props.widget.time = {mode: 'campaign'}; render();
    assert.equal(state.subscribed, false);
    const complete = groupTree(state.chart!.items, state.chart!.root);
    assert.equal(complete.exact, null); assert.equal(complete.valueUnknown, true);
    assert.equal(complete.children.find(node => node.key === 'mod category')!.exact, 250000n);
    assert.match(state.chart!.footnote!, /Known recorded charges total 2.5 PP; the complete spending total is not known/);
    assert.equal(state.chart!.items.find(item => item.path[0].id === 'coverage-gap')!.unknownValue, true);
    props.widget.time = {mode: 'range', firstOffset: 15, lastOffset: 25}; render();
    assert.equal(groupTree(state.chart!.items, state.chart!.root).exact, 100000n);
    props.widget.time = {mode: 'pin', windows: {track: {firstOffset: 0, lastOffset: 12}}}; render();
    assert.equal(groupTree(state.chart!.items, state.chart!.root).exact, 150000n);
    props.active = false; render(); assert.equal(state.chart!.active, false); assert.equal(state.subscribed, false);
    state.data = [{...data, spendingCoverage: []}];
    assert.match(render(), /Power spending coverage was not recorded/); assert.equal(state.chart, undefined);
  } finally {await vite.close(); delete host.__spendingBreakdown;}
});
