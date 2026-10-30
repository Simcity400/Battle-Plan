import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'vite';
import { MIN_SLOT } from '../../charts/race/race.ts';
import { createElement, type ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { ProductionSelection, ProductionSelectionRequest } from './preparedProducedData';
import type { WidgetProps } from '../../dashboard/definitions';

// Execute the actual widget consumer's routing with a prepared-capable client.
// The returned child remains unmounted, so no DOM or transport fixture can mask
// whether a history view was incorrectly sent to a point-only prepared query.
test('prepared production routes the current race; lines and breakdown retain history and removed settings stay removed', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const module = await vite.ssrLoadModule('/ui/src/widgets/equipment-produced/EquipmentProducedWidget.tsx');
    const render = module.default as (props: WidgetProps) => ReactElement;
    const selected = (view: string, show = 'chart', prepared = true, split = false, grouping = 'type') => {
      const client = prepared ? { preparedQuery: async () => null } : {};
      const props = { widget: { options: { view, show, split, grouping } }, runtime: { client } } as unknown as WidgetProps;
      const element = render(props);
      return (element.type as { name: string }).name;
    };
    assert.equal(selected('race'), 'PreparedEquipmentProducedWidget');
    assert.equal(selected('mix'), 'PreparedEquipmentProducedWidget', 'the removed mix choice resolves to the current default race');
    assert.equal(selected('lines'), 'HistoryEquipmentProducedWidget');
    assert.equal(selected('race', 'breakdown'), 'HistoryEquipmentProducedWidget');
    assert.equal(selected('race', 'chart', false), 'HistoryEquipmentProducedWidget');
    assert.equal(selected('race', 'chart', true, true), 'PreparedEquipmentProducedWidget', 'the removed split setting is ignored');
    assert.equal(selected('race', 'chart', true, false, 'variant'), 'PreparedEquipmentProducedWidget', 'the removed grouping setting is ignored');
    assert.equal(selected('race', 'chart', true, false, 'domain'), 'PreparedEquipmentProducedWidget');
  } finally {
    await vite.close();
  }
});


test('actual prepared race adapter passes full global extent and real page offset to the chart', async () => {
  const page: ProductionSelection = { schema: 1, branchId: '1', firstHour: 0, lastHour: 23, known: true, observation: 0, recorded: true,
    keys: [], types: [], totalCountries: 5000, offset: '4999', valueMin: '-7', valueMax: '9007199254740993', warnings: [],
    countries: [{ tag: 'LAST', slot: 4999, countryId: 5000, segment: '1', total: '-7', equipmentTotal: '-7',
      readHour: 0, observation: 0, values: [] }] };
  const state: { page: ProductionSelection; view?: CountryRaceViewProps; request?: ProductionSelectionRequest } = { page };
  const globals = globalThis as typeof globalThis & { __producedRaceFixture?: typeof state };
  globals.__producedRaceFixture = state;
  const replacements: Record<string, string> = {
    '/widgets/equipment-produced/usePreparedProduction.ts': `export const hasPreparedProduction = () => true;
      export const usePreparedProduction = (client, source, request) => {
        if (request?.limit > 0) globalThis.__producedRaceFixture.request = request;
        return { data: globalThis.__producedRaceFixture.page, hour: 12, pending: false, retry() {} };
      };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ retry() {} });',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 480; export const useChartTokens = () => ({ categories: ["red", "blue"], muted: "grey" });',
    '/charts/race/CountryPartsDrill.tsx': 'export default props => props.main(() => {}, true);',
    '/charts/race/CountryRaceView.tsx': `import {createElement} from 'react'; export default props => {
      globalThis.__producedRaceFixture.view = props; return createElement('div'); };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'produced-race-leaves', enforce: 'pre',
      load(id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
  try {
    const module = await vite.ssrLoadModule('/ui/src/widgets/equipment-produced/PreparedEquipmentProducedWidget.tsx');
    const props = { active: true, widget: { kind: 'equipment-produced', options: { view: 'race', included: 'one' },
      trackIds: ['track'], time: { mode: 'campaign' }, countries: { kind: 'group', group: { type: 'top', count: 5000 } } },
      runtime: { client: {}, clock: { getHour: () => 12 }, tracks: [{ id: 'track', selectedTag: 'FIRST',
        source: { id: 'source', revision: 'revision' }, branch: { id: '1', firstHour: 0, lastHour: 23 } }] } } as unknown as WidgetProps;
    renderToStaticMarkup(createElement(module.default, props));
    assert.equal(state.view?.remoteRows?.total, 5000);
    assert.equal(state.view?.remoteRows?.offset, 4999);
    assert.equal(state.view?.remoteRows?.min, -7);
    assert.equal(state.view?.remoteRows?.max, Number('9007199254740993'));
    assert.equal(state.view?.slots, 5000);
    assert.deepEqual(state.view?.ranked.map(row => row.tag), ['LAST']);
    assert.equal(state.view?.beforeChart, undefined);
    assert.equal(state.view?.raceLegendParts, undefined, 'the current unsplit race has no equipment legend');
    assert.equal(state.view?.moment, 12);
    assert.equal(state.request?.limit, Math.ceil(480 / MIN_SLOT));
  } finally { await vite.close(); delete globals.__producedRaceFixture; }
});
