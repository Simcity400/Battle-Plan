import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, gapGroups, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply } from '../../generated/dashboard';

const clockAt = (hour: number) => ({ getHour: () => hour, getSnapshot: () => ({ playing: false, hoursPerSecond: 0, position: hour }),
  subscribe: () => () => {}, subscribeFrame: () => () => {} });

test('army sizes of a group: a reply with no sizes says the countries have none, one with sizes names the day it has none', async () => {
  const window = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const reply = { branchId: BRANCH.id, available: true, measures: ['divisions'], types: [], warnings: [], windows: [], window: 0, interval: window,
    series: [{ tag: 'GER', membership: null, segments: [window], runs: [[0, 3, BRANCH.lastHour]] }] };
  const state = { groups: pendingGroups({ final: true }) as CountryGroupReply, reply: { ...reply, series: [] as typeof reply.series },
    view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __armyPending?: typeof state }; host.__armyPending = state;
  const manifest = { ...reply, windows: [window], window: null, interval: null, series: [] };
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/army-size/ArmySizeWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(manifest)}, retry() {} });`,
    '/army-size/frameWindows.ts': 'export const useFrameWindows = () => ({ data: new Map([[0, globalThis.__armyPending.reply]]), errors: new Map(), retry() {}, holds: () => true, check() {} });',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__armyPending.groups, retry() {} });',
    '/widgets/army-size/preparedArmy.ts': 'export const usePreparedArmy = () => ({ dense: false, summaries: new Map(), identities: undefined });',
    '/widgets/army-size/ArmyBreakdown.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__armyPending.view = props; return props.state ?? null; };',
  });
  try {
    const props = { widget: { id: 'army', kind: 'army-size', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: clockAt(BRANCH.lastHour), client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // No human player at the end, and the group's series hold no army size.
    assert.match(render(), /No known army sizes for these countries\./);
    state.reply = reply;
    render();
    assert.match(state.view!.empty, /^No known army sizes on /);
  } finally { delete host.__armyPending; await close(); }
});

test('army sizes of a group with no member say why: a recorded gap leaves its membership incomplete, else it has none', async () => {
  const window = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const manifest = { branchId: BRANCH.id, available: true, measures: ['divisions'], types: [], warnings: [], windows: [window], window: null, interval: null, series: [] };
  const state = { groups: gapGroups(), reply: { ...manifest, windows: [], window: 0, interval: window } };
  const host = globalThis as typeof globalThis & { __armyGap?: typeof state }; host.__armyGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/army-size/ArmySizeWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request ? ${JSON.stringify(manifest)} : undefined, retry() {} });`,
    // No member to ask for: no history identity, so no window replies.
    '/army-size/frameWindows.ts': `export const useFrameWindows = (_client, identity) => ({ data: identity ? new Map([[0, globalThis.__armyGap.reply]]) : new Map(),
      errors: new Map(), retry() {}, holds: () => true, check() {} });`,
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__armyGap.groups, retry() {} });',
    '/widgets/army-size/preparedArmy.ts': 'export const usePreparedArmy = () => ({ dense: false, summaries: new Map(), identities: undefined });',
    '/widgets/army-size/ArmyBreakdown.tsx': 'export default () => null;',
    '/charts/race/CountryRaceView.tsx': 'export default props => props.state ?? null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'army', kind: 'army-size', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'race' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: clockAt(hour), client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    // The humans' membership was not recorded at 140-150: whatever the hour, who they were is not known for the campaign.
    for (const hour of [100, 145, 170]) assert.match(render(hour), /Group membership is incomplete for this campaign\./);
    state.groups = { ...gapGroups(), unobserved: { humans: [], factions: [], wars: [] } };
    for (const hour of [100, 170]) assert.match(render(hour), /This group has no members in this campaign\./);
  } finally { delete host.__armyGap; await close(); }
});
