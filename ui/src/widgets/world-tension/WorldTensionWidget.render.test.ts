import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { tokensFromValues } from '../../charts/tokens.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import { validatePreparedTension } from './preparedTension.ts';

test('tension sources of a group counted at an hour still being prepared are said to be loading', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  // ENG generates all the world tension; GER, the human player, none.
  const data = validatePreparedTension({ branchId: BRANCH.id, tensionScale: 1000, contributionScale: 100000, ceilingRaw: '10000000', warnings: [], events: [],
    reads: [{ ...span, startsSegment: true, tensionRaw: '1000', sumRaw: '100000', unnamedRaw: '0' }],
    countries: [{ tag: 'ENG', slot: 2, runs: [{ ...span, startsSegment: true, valueRaw: '100000' }], identities: [{ ...span, slot: 2, countryId: 102 }] }],
    available: true, catalogue: [{ tag: 'ENG', key: '102', slot: 2, firstHour: 0 }], exactHours: [BRANCH.lastHour], summaries: {} });
  const state = { data, groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __tensionPending?: typeof state }; host.__tensionPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/world-tension/WorldTensionWidget.tsx', {
    '/dashboard/preparedSeries.ts': 'export const usePreparedWindow = () => ({ data: globalThis.__tensionPending.data, retry() {}, onSummaryView() {}, pickOwner: {} });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__tensionPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag: () => undefined, flagPending: () => false, flagReason: () => undefined, color: () => null, revision: 0 });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => (${JSON.stringify(tokensFromValues(() => '#123456', 'sans-serif'))}); export const useViewportHeight = () => 500;`,
    '/charts/TimeChart.tsx': 'export default () => null;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
  });
  try {
    const props = { widget: { id: 'tension', kind: 'world-tension', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players, and so their sources, are not known yet.
    assert.match(render(), /This range is still being prepared\./);
    // Read, GER is the human player then and generates no tension: the list is simply empty.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    const final = render();
    assert.doesNotMatch(final, /still being prepared/);
    assert.match(final, /world-tension-total/);
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'ENG', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /<li[^>]*>.*ENG/);
  } finally { delete host.__tensionPending; await close(); }
});
