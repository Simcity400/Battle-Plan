import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import { syntheticEpisode, syntheticRaidLog } from '../../dashboard/raids.fixtures.ts';
import type { CountryGroupReply, RaidLogReply } from '../../generated/dashboard';

const CLOCK = (hour: number) => ({ getHour: () => hour, getSnapshot: () => ({ position: hour, playing: false, hoursPerSecond: 0 }),
  subscribe: () => () => {}, subscribeFrame: () => () => {} });
const props = (time: string, hour: number) => ({ widget: { id: 'raids', kind: 'raid-tally', trackIds: ['main'], countries: humansBy('hour'), time: { mode: time }, options: { view: 'race' } },
  runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
    clock: CLOCK(hour), client: {} }, active: true, onDiagnostics() {} }) as unknown as WidgetProps;

test('the raid tally of a group says when it has no count at all, and hands the race its rows otherwise', async () => {
  const reply: RaidLogReply = syntheticRaidLog([syntheticEpisode('1', 0, 'GER', 10, 20, true)]);
  const state = { groups: pendingGroups({ final: true, humans: [] }) as CountryGroupReply, reply, view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __raidGroups?: typeof state }; host.__raidGroups = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/raid-tally/RaidTallyWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__raidGroups.groups, retry() {} });',
    '/dashboard/products.ts': 'export const useProduct = (_client, _request, active) => ({ data: active ? globalThis.__raidGroups.reply : undefined, retry() {} });',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__raidGroups.view = props; return null; };',
  });
  try {
    const render = () => renderToStaticMarkup(createElement(module.default, props('campaign', BRANCH.lastHour)));
    assert.match(render(), /This group has no members in this campaign\./);
    assert.equal(state.view, undefined);
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour }] });
    render();
    const view = state.view as CountryRaceViewProps | undefined;
    assert.deepEqual(view?.ranked.map(entry => [entry.tag, entry.value, entry.inset]), [['GER', 1, 1]]);
  } finally { delete host.__raidGroups; await close(); }
});
