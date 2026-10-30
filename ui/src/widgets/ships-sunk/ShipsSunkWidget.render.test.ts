import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('an empty ships sunk log whose group hours are still being prepared is loading, not a log with no matches', async () => {
  const state = { groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __sunkPending?: typeof state }; host.__sunkPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/ships-sunk/ShipsSunkWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__sunkPending.groups });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/productWindows.ts': 'export const useProductWindows = () => ({ data: new Map(), errors: new Map(), retry() {} });',
    '/ships-sunk/useShipsSunk.ts': `export const validateSinkingWindow = value => value; export const useShipsSunkBattleTitles = () => ({});
      export const useShipsSunk = (widget, runtime) => ({ track: runtime.tracks[0], retry() {}, data: { windows: [], pending: [],
        classes: { available: true, classes: [], rows: [], recordedNames: [], places: [], warnings: [] } } });`,
    '/charts/log/LogList.tsx': 'import { createElement } from "react"; export default props => createElement("p", {}, props.empty); export const PreparedLogCoverage = () => null;',
  });
  try {
    const props = { widget: { id: 'ships-sunk', kind: 'ships-sunk', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No ships sunk recorded/);
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No ships sunk recorded for these settings\./);
  } finally { delete host.__sunkPending; await close(); }
});
