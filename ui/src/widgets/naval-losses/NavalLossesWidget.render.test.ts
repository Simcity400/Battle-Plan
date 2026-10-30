import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups.ts';
import { assertNoMemberStates, BRANCH, gapGroups, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, NavalLossesReply } from '../../generated/dashboard';

test('naval losses of a group at an hour still being prepared are loading, not unknown', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const reply: NavalLossesReply = { branchId: BRANCH.id, available: true, firstRead: 0, shipFirstRead: 0, convoyFirstRead: 0, shipLastRead: 200, convoyLastRead: 200,
    shipReads: [0], convoyReads: [0], warnings: [], series: [{ tag: 'GER', runs: [{ ...span, startsSegment: true, ships: 1, convoys: 2 }], segments: [span], membership: null }] };
  const state = { groups: pendingGroups() as CountryGroupReply, reply, views: [] as CountryRaceViewProps[] };
  const host = globalThis as typeof globalThis & { __navalPending?: typeof state }; host.__navalPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/naval-losses/NavalLossesWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__navalPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: [], muted: '#888' }); export const useViewportHeight = () => 800;",
    '/widgets/naval-losses/preparedNaval.ts': `export const usePreparedNaval = () => ({ data: globalThis.__navalPending.reply, hour: ${BRANCH.lastHour}, summaries: new Map(), dense: false });`,
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__navalPending.views.push(props); return props.state ?? null; };',
  });
  try {
    const props = { widget: { id: 'naval', kind: 'naval-losses', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const race = () => { state.views = []; renderToStaticMarkup(createElement(module.default, props)); return state.views.find(view => view.view === 'race')!; };
    // The branch end is still being prepared: its human players are not known yet.
    assert.equal(race().empty, MEMBERSHIP_PREPARING);
    // The final reply proves no human player at the end.
    state.groups = pendingGroups({ final: true });
    assert.equal(race().empty, 'Naval losses unknown at this time.');
  } finally { delete host.__navalPending; await close(); }
});

test('naval losses of a group with no member found yet say what the moment was once read, and load until then', async () => {
  const state = { groups: gapGroups() };
  const host = globalThis as typeof globalThis & { __navalGap?: typeof state }; host.__navalGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/naval-losses/NavalLossesWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__navalGap.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: [], muted: '#888' }); export const useViewportHeight = () => 800;",
    // The members asked for, none: an empty reply at the hour asked; no reply without a request.
    '/widgets/naval-losses/preparedNaval.ts': `export const usePreparedNaval = (_client, _track, params, hour) => ({ data: params ? { branchId: '1', available: true,
      firstRead: 0, shipFirstRead: 0, convoyFirstRead: 0, shipLastRead: 200, convoyLastRead: 200, shipReads: [0], convoyReads: [0], warnings: [], series: [] } : undefined,
      hour, summaries: new Map(), dense: false });`,
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => props.state ?? null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'naval', kind: 'naval-losses', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assertNoMemberStates(render);
  } finally { delete host.__navalGap; await close(); }
});
