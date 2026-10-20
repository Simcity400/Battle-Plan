import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('no licence at an hour whose group membership is still being prepared is loading, not a group with no members', async () => {
  const state = { groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __licencesPending?: typeof state }; host.__licencesPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/production-licences/ProductionLicencesWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__licencesPending.groups, retry() {} });',
    '/dashboard/products.ts': `export const useProduct = () => ({ retry() {}, data: { lastHour: 200, known: [{ firstHour: 0, lastHour: 200 }], licences: [], holders: [], warnings: [] } });`,
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 200;',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ muted: "#777", fontSize: { title: 12, small: 10 } }); export const useViewportHeight = () => 1000;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, color: () => "#000", flag: () => null });',
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "#000";',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/flow/FlowDiagram.tsx': 'export default props => props.message;',
  });
  try {
    const props = { widget: { id: 'production-licences', kind: 'production-licences', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'campaign' } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The hour shown is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /This group has no members at this date\./);
    // The final reply proves there is no human player then.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /This group has no members at this date\./);
  } finally { delete host.__licencesPending; await close(); }
});
