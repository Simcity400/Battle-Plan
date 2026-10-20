import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups.ts';
import { assertNoMemberStates, BRANCH, gapGroups, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, RaidsReply } from '../../generated/dashboard';

test('the raid tally of a group at an hour still being prepared is loading, not unrecorded', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const reply: RaidsReply = { branchId: BRANCH.id, phasePreparing: 0, phaseEnded: 1, outcomeNone: 0, available: true, unavailable: null, types: [], artwork: [],
    episodes: [], readHours: [0], warnings: [], series: [{ tag: 'GER', runs: [{ ...span, readHour: 0, launched: 2, succeeded: 1 }], segments: [span], membership: null }] };
  const state = { groups: pendingGroups() as CountryGroupReply, reply, view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __raidPending?: typeof state }; host.__raidPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/raid-tally/RaidTallyWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__raidPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    '/dashboard/useRaids.ts': 'export const useRaids = () => ({ data: globalThis.__raidPending.reply, retry() {} });',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__raidPending.view = props; return null; };',
  });
  try {
    const props = { widget: { id: 'raids', kind: 'raid-tally', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    render();
    assert.equal(state.view?.empty, MEMBERSHIP_PREPARING);
    state.reply = { ...reply, series: [] };
    assert.match(render(), /aria-label="This range is still being prepared\."/);
    // The final reply proves no human player at the end, and no raid count for the group's countries.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No raid counts were recorded for these countries\./);
    state.reply = reply;
    render();
    assert.equal(state.view?.empty, 'No raid counts recorded at this time.');
  } finally { delete host.__raidPending; await close(); }
});

test('the raid tally of a group with no member found yet says what the moment was once read, and loads until then', async () => {
  const empty: RaidsReply = { branchId: BRANCH.id, phasePreparing: 0, phaseEnded: 1, outcomeNone: 0, available: true, unavailable: null, types: [], artwork: [],
    episodes: [], readHours: [0], warnings: [], series: [] };
  const state = { groups: gapGroups(), reply: empty };
  const host = globalThis as typeof globalThis & { __raidGap?: typeof state }; host.__raidGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/raid-tally/RaidTallyWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__raidGap.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    // The members asked for, none: the tally answers with no series; nothing is asked while inactive.
    '/dashboard/useRaids.ts': 'export const useRaids = (_widget, _runtime, active) => ({ data: active ? globalThis.__raidGap.reply : undefined, retry() {} });',
    '/charts/race/CountryRaceView.tsx': 'export default () => null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'raids', kind: 'raid-tally', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assertNoMemberStates(render);
  } finally { delete host.__raidGap; await close(); }
});
