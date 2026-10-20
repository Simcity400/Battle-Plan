import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, ExperienceReply } from '../../generated/dashboard';

test('a group with no recorded experience while membership is still being prepared is loading, not missing', async () => {
  const data: ExperienceReply = { branchId: BRANCH.id, scale: 32768, dayHours: 24, warnings: [], series: [{ tag: 'GER', reads: [] }] };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __experiencePending?: typeof state }; host.__experiencePending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/experience/ExperienceWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(data)}, retry() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__experiencePending.groups, retry() {} });',
    '/dashboard/playback.ts': `export const usePlaybackHour = () => ${BRANCH.lastHour}; export const usePlaybackSpeed = () => 0;`,
    '/dashboard/countryLooks.ts': `export const lookPlan = (_branch, tags, hour) => ({ wanted: tags.map(tag => ({ tag, hour })), maxHours: 16 });
      export const useCountryLooks = () => ({ color: () => [20, 50, 80], warnings: [] });`,
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesRead = () => new Map();',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ revision: 0, labelAt: () => tag => ({ name: tag, known: true }) });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: ['#123'], surface: '#000', countryColours: 'game' }); export const useViewportHeight = () => 1000;",
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default () => null;',
  });
  try {
    const props = { widget: { id: 'experience', kind: 'experience', trackIds: ['main'], countries: humansBy('ever'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // Anyone ever a human player is known only with the whole branch: the later hours are still being prepared.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /Not in this recording/);
    // The final reply proves GER, the only human player ever, has no recorded experience.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /Not in this recording/);
  } finally { delete host.__experiencePending; await close(); }
});
