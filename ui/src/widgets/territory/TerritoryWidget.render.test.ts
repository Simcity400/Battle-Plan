import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups.ts';
import { assertNoMemberStates, BRANCH, gapGroups, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, TerritoryReply } from '../../generated/dashboard';

test('territory of a group at an hour still being prepared is loading, not unrecorded', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const values = [5, 10, 250, 800, 4, 9, 200, 700, null, null, null, null];
  const reply: TerritoryReply = { branchId: BRANCH.id, areaUnit: null, areaReason: null, warnings: [], worldTotals: [{ ...span, startsSegment: true, values }],
    series: [{ tag: 'GER', segments: [span], membership: null, runs: [{ ...span, startsSegment: true, values }] }] };
  const state = { groups: pendingGroups() as CountryGroupReply, view: undefined as CountryRaceViewProps | undefined, share: undefined as Record<string, unknown> | undefined };
  const host = globalThis as typeof globalThis & { __territoryPending?: typeof state }; host.__territoryPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/territory/TerritoryWidget.tsx', {
    '/charts/race/useRaceTooltip.ts': 'export const useRaceTooltip = () => ({ showTip() {}, tipWanted: [] });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__territoryPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, color: () => [1, 2, 3], flag() {}, flagPending: () => false, flagReason() {} });',
    '/dashboard/FactionColours.tsx': 'export const useFactionColours = () => ({ colours: new Map(), names: new Map(), warnings: [], loading: false, error: null });',
    '/dashboard/preparedFactionTotals.ts': 'export const usePreparedFactionTotals = () => ({ retry() {} });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: ['#123'], muted: '#888', surface: '#000', accent: '#fff' }); export const useViewportHeight = () => 1000;",
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/widgets/territory/preparedTerritory.ts': `export const usePreparedTerritory = () => ({ data: ${JSON.stringify(reply)}, sumRuns: [], lineSummaries: new Map(), retry() {} });`,
    '/widgets/territory/TerritoryShare.tsx': 'export default props => { globalThis.__territoryPending.share = props; return "territory share"; };',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__territoryPending.view = props; return null; };',
  });
  try {
    const props = { widget: { id: 'territory', kind: 'territory', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    render();
    assert.equal(state.view?.empty, MEMBERSHIP_PREPARING);
    props.widget.options = { view: 'share' };
    assert.match(render(), /aria-label="This range is still being prepared\."/);
    // The final reply proves no human player at the end.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /territory share/);
    props.widget.options = { view: 'race' };
    render();
    assert.match(state.view!.empty, /^Not recorded on /);
  } finally { delete host.__territoryPending; await close(); }
});

test('territory of a group with no member found yet says what the moment was once read, and loads until then', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const values = [5, 10, 250, 800, 4, 9, 200, 700, null, null, null, null];
  const empty: TerritoryReply = { branchId: BRANCH.id, areaUnit: null, areaReason: null, warnings: [], worldTotals: [{ ...span, startsSegment: true, values }], series: [] };
  const state = { groups: gapGroups(), reply: empty };
  const host = globalThis as typeof globalThis & { __territoryGap?: typeof state }; host.__territoryGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/territory/TerritoryWidget.tsx', {
    '/charts/race/useRaceTooltip.ts': 'export const useRaceTooltip = () => ({ showTip() {}, tipWanted: [] });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__territoryGap.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, color: () => [1, 2, 3], flag() {}, flagPending: () => false, flagReason() {} });',
    '/dashboard/FactionColours.tsx': 'export const useFactionColours = () => ({ colours: new Map(), names: new Map(), warnings: [], loading: false, error: null });',
    '/dashboard/preparedFactionTotals.ts': 'export const usePreparedFactionTotals = () => ({ retry() {} });',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: ['#123'], muted: '#888', surface: '#000', accent: '#fff' }); export const useViewportHeight = () => 1000;",
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    // No member to ask for: the country totals stay inactive, so no reply.
    '/widgets/territory/preparedTerritory.ts': `export const usePreparedTerritory = (_client, _track, _params, _measure, _rule, _moment, active) => ({
      data: active ? globalThis.__territoryGap.reply : undefined, sumRuns: [], lineSummaries: new Map(), retry() {} });`,
    '/widgets/territory/TerritoryShare.tsx': 'export default () => "territory share";',
    '/charts/race/CountryRaceView.tsx': 'export default props => props.state ?? null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'territory', kind: 'territory', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assertNoMemberStates(render);
  } finally { delete host.__territoryGap; await close(); }
});
