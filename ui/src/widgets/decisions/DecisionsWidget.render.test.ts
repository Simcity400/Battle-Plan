import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('an empty decision log whose group hours are still being prepared is loading, not a log with no matches', async () => {
  const state = { groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __decisionsPending?: typeof state }; host.__decisionsPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/decisions/DecisionsWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__decisionsPending.groups });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/event-feed/useScriptOccurrences.ts': `export const useScriptOccurrences = () => ({ data: { unavailable: [] }, pages: [], warnings: [], replies: [],
      errors: new Map(), total: null, identity: 'decisions', prefix: null, onWantedPages() {}, retryPages() {} });`,
    '/charts/log/LogList.tsx': 'import { createElement } from "react"; export default props => createElement("p", {}, props.empty);',
  });
  try {
    const props = { widget: { id: 'decisions', kind: 'decisions', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No recorded decisions/);
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No recorded decisions match these settings\./);
  } finally { delete host.__decisionsPending; await close(); }
});
