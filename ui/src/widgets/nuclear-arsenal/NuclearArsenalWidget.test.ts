import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { CountryGroupReply, CountryNukesReply, NukesParams } from '../../generated/dashboard';

test('actual nuclear widget aligns drill playback and projects manual roots through dated aliases', async () => {
  const groups: CountryGroupReply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    unobserved: { humans: [], factions: [], wars: [] }, warnings: [], countryIdentities: [
      { tag: 'OLD', identities: [{ firstHour: 1000, lastHour: 1009, slot: 1, countryId: 11 }] },
      { tag: 'NEW', identities: [{ firstHour: 1010, lastHour: 1029, slot: 1, countryId: 11 },
        { firstHour: 1030, lastHour: 1100, slot: 2, countryId: 22 }] },
    ] };
  const reply: CountryNukesReply = { branchId: '1', readHours: [1000, 1010, 1030], emptyReadHours: [], warnings: [],
    types: [{ ordinal: 0, token: 'synthetic_type', name: 'Synthetic type', supported: true },
      { ordinal: 1, token: 'unrecorded_type', name: 'Unrecorded weapon', supported: true }], series: groups.countryIdentities.map(history => ({
      tag: history.tag, segments: history.identities.map(({ firstHour, lastHour }) => ({ firstHour, lastHour })), membership: null,
      runs: history.identities.map(({ firstHour, lastHour, slot }) => ({ firstHour, lastHour, startsSegment: true,
        counts: [{ ordinal: 0, value: slot === 1 ? 3 : 99 }], total: slot === 1 ? 3 : 99 })),
    })) };
  const host = globalThis as typeof globalThis & { __nuclearCharts?: CountryRaceViewProps[];
    __nuclearRequests?: { params: NukesParams }[]; __nuclearGroupsActive?: boolean;
    __nuclearParts?: { parts: { label: string; value: number | null; text?: string }[] } };
  const replacements: Record<string, string> = {
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = (_runtime, _track, active) => {
      globalThis.__nuclearGroupsActive = active; return { data: ${JSON.stringify(groups)}, retry() {} }; };`,
    '/dashboard/products.ts': `export const useProduct = (_client, request) => {
      if (request) globalThis.__nuclearRequests.push(request); return { data: ${JSON.stringify(reply)}, retry() {} }; };
      export const loadProduct = (client, request, _signal, _validate, _source, _restart, publish) => {
        publish?.(useProduct(client, request).data); return new Promise(() => {}); };`,
    // The race's view read answers in the same render, as its products do here.
    '/dashboard/viewRead.ts': `export const useViewRead = (lane, view, _viewKey, active, read) => { let data;
      if (active && lane != null && view != null) void read(view, new AbortController().signal, value => { data = value; });
      return { data, retry() {} }; };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag: () => undefined });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ muted: '#888888', categories: ['#ffffff'] });",
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__nuclearCharts.push(props); return null; };',
    '/charts/drill/DrillHost.tsx': 'import { createElement, Fragment } from "react"; export default props => createElement(Fragment, null, props.main, props.children);',
    '/charts/drill/DrillParts.tsx': 'export default props => { globalThis.__nuclearParts = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'nuclear-widget-selection', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/nuclear-arsenal/NuclearArsenalWidget.tsx');
    let hour = 20;
    const props = { widget: { id: 'nuclear', kind: 'nuclear-arsenal', trackIds: ['track'], countries: { kind: 'tags', tags: ['OLD'] },
      time: { mode: 'follow' }, options: { view: 'lines' } }, runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 1000,
        selectedTag: null, source: { id: 'source', revision: '1', metadata: { countries: [] } },
        branch: { id: '1', firstHour: 1000, lastHour: 1100 } }], clock: { getHour: () => hour,
        getSnapshot: () => ({ position: hour, playing: true }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => {
      host.__nuclearCharts = []; host.__nuclearRequests = [];
      renderToStaticMarkup(createElement(Widget, props));
      return host.__nuclearCharts!;
    };
    const [overview, drill] = render();
    assert.equal(host.__nuclearGroupsActive, true, 'manual picks need native identity history');
    assert.deepEqual(host.__nuclearRequests?.[0].params.tags, ['OLD'], 'drawing aliases never become new backend roots');
    assert.equal(overview.playhead, 1020);
    assert.equal(drill.moment, 1020, 'drill Follow uses the selected campaign offset');
    assert.deepEqual(overview.ranked.map(entry => entry.tag), ['NEW']);
    assert.deepEqual(overview.lines.map(line => [line.tag, line.runs.map(run => [run.firstHour, run.lastHour])]),
      [['OLD', [[1000, 1009]]], ['NEW', [[1010, 1029]]]]);
    hour = 40;
    const [later] = render();
    assert.deepEqual(later.ranked, [], 'the later NEW holder was never picked');
    assert.equal(later.tooltip('NEW', 1040, 'Unrelated holder', new Map(), () => () => ({ name: '' })), null);
    props.widget.options = { view: 'mix' }; hour = 20;
    assert.equal(render()[0].mix?.tag, 'NEW', 'the first manual country mix follows its current alias');
    props.widget.options = { show: 'breakdown', drillOriginHour: 1000 };
    const [pinned] = render();
    renderToStaticMarkup(pinned.beforeChart);
    const missing = host.__nuclearParts!.parts.find(part => part.label === 'Unrecorded weapon');
    assert.equal(missing?.value, null);
    assert.equal(missing?.text, 'Not recorded', 'missing weapon types stay in the same part rows');
  } finally {
    delete host.__nuclearCharts; delete host.__nuclearRequests; delete host.__nuclearGroupsActive; delete host.__nuclearParts;
    await vite.close();
  }
});
