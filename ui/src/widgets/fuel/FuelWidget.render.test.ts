import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, FuelReply } from '../../generated/dashboard';

test('a group with no recorded fuel while membership is still being prepared is loading, not unrecorded', async () => {
  const reply: FuelReply = { branchId: BRANCH.id, tag: 'GER', quantityScale: 1000, gainScale: 100_000, dayHours: 24, detailHours: 720, warnings: [],
    gainLabels: [], transferLabels: [], consumerNames: [], reads: [] } as unknown as FuelReply;
  // Two human players, so no single country's breakdown opens by itself.
  const humans = [{ tag: 'GER', firstHour: BRANCH.firstHour, lastHour: 160 }, { tag: 'ENG', firstHour: BRANCH.firstHour, lastHour: 160 }];
  const state = { groups: pendingGroups({ humans }) as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __fuelPending?: typeof state }; host.__fuelPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/fuel/FuelWidget.tsx', {
    '/dashboard/products.ts': `export const useProducts = (_client, requests) => ({ data: requests.map(request => ({ ...${JSON.stringify(reply)}, tag: request.params.tag })), retry() {} });
      export const useProduct = () => ({ retry() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__fuelPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ revision: 0, labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: ['#123'], muted: '#888', surface: '#000' }); export const useViewportHeight = () => 1000;",
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/widgets/fuel/preparedFuel.ts': 'export const usePreparedFuel = () => ({ enabled: false, summaries: undefined }); export const useFuelSeriesParity = () => {}; export const scaledFuelSummary = () => null; export const fuelSummaryStrip = () => [];',
    '/charts/race/CountryRaceView.tsx': 'export default props => props.state ?? "fuel lines";',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
  });
  try {
    const props = { widget: { id: 'fuel', kind: 'fuel', trackIds: ['main'], countries: humansBy('ever'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // Anyone ever a human player is known only with the whole branch: the later hours are still being prepared.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No fuel was recorded/);
    // The final reply proves GER and ENG, the only human players ever, have no recorded fuel.
    state.groups = pendingGroups({ final: true, humans });
    assert.match(render(), /No fuel was recorded for the chosen countries\./);
  } finally { delete host.__fuelPending; await close(); }
});
