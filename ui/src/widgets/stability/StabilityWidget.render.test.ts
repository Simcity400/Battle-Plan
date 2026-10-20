import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, CountryStabilityReply } from '../../generated/dashboard';

test('a group with no stability line while membership is still being prepared is loading, not unavailable', async () => {
  const reply: CountryStabilityReply = { branchId: BRANCH.id, decimals: 3, warnings: [],
    series: [{ tag: 'GER', stability: [], warSupport: [], maxCommandPower: [], carried: [], recorded: true, breakdown: null }] };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __stabilityPending?: typeof state }; host.__stabilityPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/stability/StabilityWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request?.product === 'country-stability' ? ${JSON.stringify(reply)} : undefined, retry() {} });
      export const useProducts = () => ({ retry() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__stabilityPending.groups, retry() {} });',
    '/dashboard/campaignSettings.ts': 'export const useCampaignSettings = () => ({ campaigns: {} });',
    '/dashboard/playback.ts': `export const usePlaybackHour = () => ${BRANCH.lastHour};`,
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, color: () => [1, 2, 3], flag() {}, flagPending: () => false, flagReason() {}, revision: 0 });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/widgets/national-focus/useFocusArtwork.ts': 'export const useFocusText = () => ({ read: new Map(), warnings: [] });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesAt = () => new Map();',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: ['#123'], muted: '#888', surface: '#000', countryColours: 'game', lineWidth: { series: 2 } }); export const useViewportHeight = () => 1000;",
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/charts/TimeChart.tsx': 'export default () => null;',
  });
  try {
    const props = { widget: { id: 'stability', kind: 'stability', trackIds: ['main'], countries: humansBy('ever'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // Anyone ever a human player is known only with the whole branch: the later hours are still being prepared.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /unavailable for these countries/);
    // The final reply proves GER, the only human player ever, has no recorded line.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /Stability and war support are unavailable for these countries\./);
  } finally { delete host.__stabilityPending; await close(); }
});
