import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('an empty espionage log whose group hours are still being prepared is loading, not a log with no matches', async () => {
  const state = { groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __espionagePending?: typeof state }; host.__espionagePending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/espionage-log/EspionageLogWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__espionagePending.groups });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/espionage-log/useEspionageLog.ts': `export const useEspionageWindows = () => ({ data: new Map(), errors: new Map(), retry() {} });
      export const useEspionageLog = (widget, runtime) => ({ track: runtime.tracks[0], retry() {}, data: { available: [{ firstHour: 0, lastHour: 200 }],
        completionsAvailable: [], unavailable: null, windows: [], types: [], warnings: [] } });`,
    '/charts/log/LogList.tsx': 'import { createElement } from "react"; export default props => createElement("p", {}, props.empty); export const PreparedLogCoverage = () => null;',
  });
  try {
    const props = { widget: { id: 'espionage-log', kind: 'espionage-log', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No espionage operations match/);
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No espionage operations match these countries and settings by this moment\./);
  } finally { delete host.__espionagePending; await close(); }
});
