import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply } from '../../generated/dashboard';

test('licences load until the groups come; with no licence, a group with no member at the hour says so, one with members says no licences', async () => {
  const state: { groups: CountryGroupReply | undefined } = { groups: undefined };
  const host = globalThis as typeof globalThis & { __licencesPending?: typeof state }; host.__licencesPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/production-licences/ProductionLicencesWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__licencesPending.groups, retry() {} });',
    '/dashboard/headProduct.ts': `export const useHeadProduct = () => ({ data: { branchId: '1', licences: [], missing: null, warnings: [] }, retry() {}, root() {}, read: 'asked',
  hour: ${BRANCH.lastHour}, at: ${BRANCH.lastHour}, live: { read: () => ({ get: () => undefined, missing: () => '' }), subscribe: () => () => {} }, shownAt: () => ${BRANCH.lastHour} });`,
    '/dashboard/products.ts': 'export const useProduct = () => ({ retry() {}, data: undefined });',
    '/dashboard/atHead.ts': `export const useAtHead = () => ${BRANCH.lastHour}; export const useIdentityHour = () => null;`,
    '/dashboard/widgetPerf.ts': 'export const useShownEachFrame = () => {}; export const useDrillShown = () => {}; export const DrillScope = props => props.children; export const ShownHour = () => null;',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ muted: "#777", fontSize: { title: 12, small: 10 } }); export const useViewportHeight = () => 1000;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryLooks.ts': 'export const LOOK_STEP = 720; export const useCountryLooks = () => ({ name: tag => tag, color: () => "#000", flag: () => null }); export const lookHour = hour => hour; export const looksRead = (following, identityHour, moment) => ({ hour: moment, exact: false });',
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "#000";',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/flow/FlowDiagram.tsx': 'export default props => props.message;',
  });
  try {
    const props = { widget: { id: 'production-licences', kind: 'production-licences', trackIds: ['main'], countries: humansBy('hour'),
      time: { mode: 'pin', windows: { main: { firstOffset: BRANCH.lastHour - BRANCH.firstHour, lastOffset: BRANCH.lastHour - BRANCH.firstHour } } }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'campaign' } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false, position: BRANCH.lastHour }), subscribeFrame: () => () => {} } },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const loading = render();
    assert.match(loading, /aria-label="Loading licences…"/);
    assert.doesNotMatch(loading, /no members|No production licences/);
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /This group has no members at this date\./);
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour }] });
    const none = render();
    assert.match(none, /No production licences at this date\./);
    assert.doesNotMatch(none, /no members/);
  } finally { delete host.__licencesPending; await close(); }
});
