import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, MeasureReply } from '../../generated/dashboard';
import type { MeasureConfig } from './MeasureWidget';

test('a group balance with no recorded line while membership is still being prepared is loading, not unrecorded', async () => {
  const reply: MeasureReply = { branchId: BRANCH.id, measure: 'politicalPower', decimals: 3, warnings: [], series: [{ tag: 'GER', runs: [], drops: [] }] };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __measurePending?: typeof state }; host.__measurePending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps & { config: MeasureConfig }) => ReactElement }>('/ui/src/widgets/country-measure/MeasureWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request?.product === 'country-measure' ? ${JSON.stringify(reply)} : undefined, retry() {} });
      export const useProducts = () => ({ retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });
export const useFollowedProducts = (client, requests, _moving, ...rest) => ({ ...useProducts(client, requests, ...rest), requests });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__measurePending.groups, retry() {} });',
    '/dashboard/playback.ts': `export const usePlaybackHour = clock => clock.getHour();`,
    '/dashboard/useCountryLineStyles.ts': 'export const useCountryLineStyles = () => ({ known: new Map(), data: { warnings: [] } });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesRead = () => new Map();',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag, known: true }), revision: 0 });',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ flag() {}, flagPending: () => false, flagReason() {} });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: ['#123'], muted: '#888', surface: '#000', countryColours: 'game', lineWidth: { series: 2 } }); export const useViewportHeight = () => 1000;",
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
  });
  try {
    const props = { widget: { id: 'power', kind: 'political-power', trackIds: ['main'], countries: humansBy('ever'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) }, client: {} }, active: true, onDiagnostics() {},
      config: { measure: 'politicalPower', title: 'Political power', noun: 'political power', unit: 'PP' } } as unknown as WidgetProps & { config: MeasureConfig };
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // Anyone ever a human player is known only with the whole branch: the later hours are still being prepared.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /was recorded for these countries/);
    // The final reply proves GER, the only human player ever, has no recorded balance.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No political power was recorded for these countries\./);
  } finally { delete host.__measurePending; await close(); }
});
