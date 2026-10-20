import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups.ts';
import { assertNoMemberStates, BRANCH, gapGroups, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, OccupationTrendReply } from '../../generated/dashboard';

test('occupation trends of a group at an hour still being prepared are loading, not missing', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const reply: OccupationTrendReply = { branchId: BRANCH.id, decimals: 5, warnings: [], series: [{ tag: 'GER', membership: null, segments: [span],
    runs: [{ ...span, readHour: 0, startsSegment: true, resistanceRaw: '4133540', complianceRaw: '940099', states: 2, heldStates: 2, heldComplete: true, collaboration: [] }] }] };
  const state = { groups: pendingGroups() as CountryGroupReply, reply, view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __occupationPending?: typeof state }; host.__occupationPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/occupation-trend/OccupationTrendWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__occupationPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    '/widgets/occupation-trend/preparedOccupation.ts': `export const usePreparedOccupation = () => ({ data: globalThis.__occupationPending.reply, retry() {},
      lineSummaries: new Map(), onSummaryMembership() {} });`,
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__occupationPending.view = props; return props.state ?? null; };',
  });
  try {
    const props = { widget: { id: 'occupation', kind: 'occupation-trend', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { renderToStaticMarkup(createElement(module.default, props)); return state.view!; };
    // The branch end is still being prepared: its human players are not known yet.
    assert.deepEqual([render().empty, state.view!.mixEmpty?.message], [MEMBERSHIP_PREPARING, MEMBERSHIP_PREPARING]);
    // The final reply proves no human player at the end.
    state.groups = pendingGroups({ final: true });
    assert.deepEqual([render().empty, state.view!.mixEmpty?.message], ['No occupied-state data at this time.', 'Choose an occupier.']);
  } finally { delete host.__occupationPending; await close(); }
});

test('occupation trends of a group with no member found yet say what the moment was once read, and load until then', async () => {
  const state = { groups: gapGroups() };
  const host = globalThis as typeof globalThis & { __occupationGap?: typeof state }; host.__occupationGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/occupation-trend/OccupationTrendWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__occupationGap.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    // No member to ask for: no request, so no reply.
    '/widgets/occupation-trend/preparedOccupation.ts': `export const usePreparedOccupation = (_client, _track, params) => ({
      data: params ? { branchId: '1', decimals: 5, warnings: [], series: [] } : undefined, retry() {}, lineSummaries: new Map(), onSummaryMembership() {} });`,
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => props.state ?? null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'occupation', kind: 'occupation-trend', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assertNoMemberStates(render);
  } finally { delete host.__occupationGap; await close(); }
});
