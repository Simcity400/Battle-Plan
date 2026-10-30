import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups.ts';
import { assertNoMemberStates, BRANCH, gapGroups, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryFactoriesReply, CountryGroupReply } from '../../generated/dashboard';

test('factories of a group at an hour still being prepared are loading, not unrecorded', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const reply: CountryFactoriesReply = { branchId: BRANCH.id, groups: [], fields: [{ group: 'civilian', part: 'total' }], warnings: [],
    series: [{ tag: 'GER', runs: [{ ...span, startsSegment: true, values: [5] }], segments: [span], membership: null }] };
  const state = { groups: pendingGroups() as CountryGroupReply, reply, view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __factoriesPending?: typeof state }; host.__factoriesPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/factories/FactoriesWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__factoriesPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/race/PartsRaceView.tsx': `export const usePreparedRace = () => ({ data: globalThis.__factoriesPending.reply, retry() {}, summaries: new Map() });`,
    '/charts/race/CountryPartsDrill.tsx': 'export default props => props.main(() => {}, true);',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__factoriesPending.view = props; return props.state ?? null; };',
  });
  try {
    const props = { widget: { id: 'factories', kind: 'factories', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    render();
    assert.equal(state.view?.empty, MEMBERSHIP_PREPARING);
    assert.equal(state.view?.mixEmpty?.message, MEMBERSHIP_PREPARING);
    state.reply = { ...reply, series: [] };
    assert.match(render(), /aria-label="This range is still being prepared\."/);
    // The final reply proves no human player at the end, and that the group's countries have no recorded factories.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No factories were recorded for these countries\./);
    state.reply = reply;
    render();
    assert.match(state.view!.empty, /^No factories recorded on /);
    assert.match(state.view!.mixEmpty!.message, /^No country of this group was recorded on /);
  } finally { delete host.__factoriesPending; await close(); }
});

test('factories of a group with no member found yet say what the moment was once read, and load until then', async () => {
  const state = { groups: gapGroups() };
  const host = globalThis as typeof globalThis & { __factoriesGap?: typeof state }; host.__factoriesGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/factories/FactoriesWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__factoriesGap.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    // No member to ask for: no request, so no reply.
    '/charts/race/PartsRaceView.tsx': `export const usePreparedRace = (_client, _track, request) => ({ data: request ? { series: [] } : undefined, retry() {}, summaries: new Map() });`,
    '/charts/race/CountryPartsDrill.tsx': 'export default props => props.main(() => {}, true);',
    '/charts/race/CountryRaceView.tsx': 'export default props => props.state ?? null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'factories', kind: 'factories', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assertNoMemberStates(render);
  } finally { delete host.__factoriesGap; await close(); }
});
