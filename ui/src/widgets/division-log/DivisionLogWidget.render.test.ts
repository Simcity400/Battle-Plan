import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('an empty division log whose group hours are still being prepared is loading, not a log with no matches', async () => {
  const state = { groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __divisionsPending?: typeof state }; host.__divisionsPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/division-log/DivisionLogWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__divisionsPending.groups });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 200;',
    '/dashboard/productWindows.ts': 'export const useProductWindows = () => ({ data: new Map(), errors: new Map(), retry() {} });',
    '/dashboard/products.ts': `export const useProduct = (client, request) => ({ retry() {}, data: request?.product === 'division-log-manifest'
      ? { available: true, lossesAvailable: true, windows: [], templates: [], contentWarnings: [] } : undefined });`,
    '/charts/log/LogList.tsx': 'import { createElement } from "react"; export default props => createElement("p", {}, props.empty); export const PreparedLogCoverage = () => null;',
  });
  try {
    const props = { widget: { id: 'division-log', kind: 'division-log', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No recorded division events/);
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No recorded division events match these settings\./);
  } finally { delete host.__divisionsPending; await close(); }
});
