import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups.ts';
import { assertNoMemberStates, BRANCH, gapGroups, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply } from '../../generated/dashboard';
import type { OccupationReserveReply } from './occupationReserveData';

/** GER holds reserve from ITA at every hour of the branch. */
const reply: OccupationReserveReply = {
  window: { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour, known: [{ firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour }],
    reads: [[BRANCH.firstHour, BRANCH.lastHour, BRANCH.firstHour]], series: [
      { name: 'GER@1|0|amount', runs: [[BRANCH.firstHour, BRANCH.lastHour, '1000', true]] },
      { name: 'GER@1|0|source', runs: [[BRANCH.firstHour, BRANCH.lastHour, '7', true]] },
    ] },
  holders: [{ tag: 'GER', slot: 1 }], sources: { 7: 'ITA@2' }, warnings: [],
};

const replacements = (key: string) => ({
  '/widgets/occupation-reserve-manpower/useOccupationReserve.ts': `export const useOccupationReserve = () => ({ track: globalThis.${key}.track,
    options: {}, data: globalThis.${key}.reply, retry() {} });`,
  '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ data: globalThis.${key}.groups, retry() {} });`,
  '/dashboard/products.ts': 'export const useProduct = () => ({ data: undefined, retry() {} });',
  '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0; export const usePlaybackSpeed = () => 0;',
  '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ color: () => "#888", name: tag => tag });',
  '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "#888";',
  '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ muted: "#999" }); export const useViewportHeight = () => 1000;',
  '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
  '/charts/race/CountryRaceView.tsx': `export default props => { globalThis.${key}.view = props; return null; };`,
});

const runtime = (hour: number) => ({ tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
  clock: { getHour: () => hour, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }) }, client: {}, addWidgetBelow() {} });

test('occupation reserve of a group at an hour still being prepared is loading, not absent', async () => {
  const state = { groups: pendingGroups() as CountryGroupReply, reply, track: runtime(0).tracks[0], view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __occupationPending?: typeof state }; host.__occupationPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>(
    '/ui/src/widgets/occupation-reserve-manpower/OccupationReserveWidget.tsx', replacements('__occupationPending'));
  try {
    const render = () => renderToStaticMarkup(createElement(module.default, { widget: { id: 'occupation', kind: 'occupation-reserve-manpower',
      trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: runtime(BRANCH.lastHour), active: true, onDiagnostics() {} } as unknown as WidgetProps));
    // The branch end is still being prepared: GER's membership there is not known yet.
    render();
    assert.equal(state.view?.empty, MEMBERSHIP_PREPARING);
    assert.deepEqual(state.view?.ranked, []);
    // The final reply proves there is no human player at the end: the race is genuinely empty.
    state.groups = pendingGroups({ final: true });
    render();
    assert.equal(state.view?.empty, 'No reserve manpower is held from occupied countries at this date.');
  } finally { delete host.__occupationPending; await close(); }
});

test('occupation reserve of a group with no member found yet says why once the moment is read, and loads until then', async () => {
  const state = { groups: gapGroups(), reply, track: runtime(0).tracks[0], view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __occupationGap?: typeof state }; host.__occupationGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>(
    '/ui/src/widgets/occupation-reserve-manpower/OccupationReserveWidget.tsx', replacements('__occupationGap'));
  try {
    assertNoMemberStates(hour => renderToStaticMarkup(createElement(module.default, { widget: { id: 'occupation', kind: 'occupation-reserve-manpower',
      trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' }, options: {} },
      runtime: runtime(hour), active: true, onDiagnostics() {} } as unknown as WidgetProps)));
  } finally { delete host.__occupationGap; await close(); }
});
