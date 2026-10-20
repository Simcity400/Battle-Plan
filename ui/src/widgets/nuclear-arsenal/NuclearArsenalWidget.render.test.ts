import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups.ts';
import { assertNoMemberStates, BRANCH, gapGroups, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, CountryNukesReply } from '../../generated/dashboard';

test('the nuclear arsenal of a group at an hour still being prepared is loading, not unrecorded', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const reply: CountryNukesReply = { branchId: BRANCH.id, readHours: [0], emptyReadHours: [], types: [], warnings: [],
    series: [{ tag: 'GER', runs: [{ ...span, startsSegment: true, counts: [], total: 2 }], segments: [span], membership: null }] };
  const state = { groups: pendingGroups() as CountryGroupReply, reply, views: [] as CountryRaceViewProps[] };
  const host = globalThis as typeof globalThis & { __nukesPending?: typeof state }; host.__nukesPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/nuclear-arsenal/NuclearArsenalWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__nukesPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ label: tag => ({ name: tag }), color: () => "#888" });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: [], muted: '#888' }); export const useViewportHeight = () => 800;",
    '/charts/race/PartsRaceView.tsx': `export const usePreparedRace = (_client, _track, request) => ({ data: request?.product === 'country-nukes'
      ? globalThis.__nukesPending.reply : undefined, retry() {}, summaries: new Map() });`,
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__nukesPending.views.push(props); return props.state ?? null; };',
  });
  try {
    const props = { widget: { id: 'nukes', kind: 'nuclear-arsenal', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ playing: false }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { state.views = []; const markup = renderToStaticMarkup(createElement(module.default, props));
      return { markup, race: state.views.find(view => view.view === 'race')! }; };
    // The branch end is still being prepared: its human players are not known yet.
    assert.equal(render().race.empty, MEMBERSHIP_PREPARING);
    state.reply = { ...reply, series: [] };
    assert.match(render().markup, /aria-label="This range is still being prepared\."/);
    // The final reply proves no human player at the end, and no stockpile recorded for the group's countries.
    state.groups = pendingGroups({ final: true });
    assert.match(render().markup, /No nuclear stockpiles were recorded for these countries\./);
    state.reply = reply;
    assert.equal(render().race.empty, 'No nuclear stockpile was recorded at this time.');
  } finally { delete host.__nukesPending; await close(); }
});

test('the nuclear arsenal of a group with no member found yet says what the moment was once read, and loads until then', async () => {
  const state = { groups: gapGroups() };
  const host = globalThis as typeof globalThis & { __nukesGap?: typeof state }; host.__nukesGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/nuclear-arsenal/NuclearArsenalWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__nukesGap.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ label: tag => ({ name: tag }), color: () => "#888" });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: [], muted: '#888' }); export const useViewportHeight = () => 800;",
    // No member to ask for: no request, so no reply.
    '/charts/race/PartsRaceView.tsx': `export const usePreparedRace = (_client, _track, request) => ({ data: request
      ? { branchId: '1', readHours: [], emptyReadHours: [], types: [], warnings: [], series: [] } : undefined, retry() {}, summaries: new Map() });`,
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/components/CountryPicker.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => props.state ?? null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'nukes', kind: 'nuclear-arsenal', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour, getSnapshot: () => ({ playing: false }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assertNoMemberStates(render);
  } finally { delete host.__nukesGap; await close(); }
});
