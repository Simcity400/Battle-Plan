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

test('army sizes of a group at an hour still being prepared are loading, not unknown', async () => {
  const window = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const reply = { branchId: BRANCH.id, available: true, measures: ['divisions'], unknownMeasures: [], types: [], warnings: [], windows: [], window: 0, interval: window,
    series: [{ tag: 'GER', membership: null, segments: [window], runs: [[0, 3, BRANCH.lastHour]] }] };
  const state = { groups: pendingGroups() as CountryGroupReply, reply, view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __armyPending?: typeof state }; host.__armyPending = state;
  const manifest = { ...reply, windows: [window], window: null, interval: null, series: [] };
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/army-size/ArmySizeWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(manifest)}, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': 'export const useProductWindows = () => ({ data: new Map([[0, globalThis.__armyPending.reply]]), errors: new Map(), retry() {} });',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__armyPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0; export const usePlaybackSpeed = () => 0;',
    '/widgets/army-size/preparedArmy.ts': 'export const usePreparedArmy = () => ({ dense: false, summaries: new Map(), identities: undefined });',
    '/widgets/army-size/ArmyBreakdown.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__armyPending.view = props; return props.state ?? null; };',
  });
  try {
    const props = { widget: { id: 'army', kind: 'army-size', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    render();
    assert.equal(state.view?.empty, MEMBERSHIP_PREPARING);
    state.reply = { ...reply, series: [] };
    assert.match(render(), /aria-label="This range is still being prepared\."/);
    // The final reply proves no human player at the end, and that the group's series hold no army size.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No known army sizes for these countries\./);
    state.reply = reply;
    render();
    assert.match(state.view!.empty, /^No known army sizes on /);
  } finally { delete host.__armyPending; await close(); }
});

test('army sizes of a group with no member found yet say what the moment was once read, and load until then', async () => {
  const window = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const manifest = { branchId: BRANCH.id, available: true, measures: ['divisions'], unknownMeasures: [], types: [], warnings: [], windows: [window], window: null, interval: null, series: [] };
  const state = { groups: gapGroups(), reply: { ...manifest, windows: [], window: 0, interval: window } };
  const host = globalThis as typeof globalThis & { __armyGap?: typeof state }; host.__armyGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/army-size/ArmySizeWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request ? ${JSON.stringify(manifest)} : undefined, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    // No member to ask for: no history identity, so no window replies.
    '/dashboard/productWindows.ts': `export const useProductWindows = (_client, identity) => ({ data: identity ? new Map([[0, globalThis.__armyGap.reply]]) : new Map(),
      errors: new Map(), retry() {} });`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__armyGap.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0; export const usePlaybackSpeed = () => 0;',
    '/widgets/army-size/preparedArmy.ts': 'export const usePreparedArmy = () => ({ dense: false, summaries: new Map(), identities: undefined });',
    '/widgets/army-size/ArmyBreakdown.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => props.state ?? null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'army', kind: 'army-size', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assertNoMemberStates(render);
  } finally { delete host.__armyGap; await close(); }
});
