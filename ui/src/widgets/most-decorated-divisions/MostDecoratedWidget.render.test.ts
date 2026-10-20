import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('divisions of a group counted at an hour still being prepared are loading, not none', async () => {
  const state = { groups: pendingGroups() as CountryGroupReply, tags: [] as unknown[] };
  const host = globalThis as typeof globalThis & { __decoratedPending?: typeof state }; host.__decoratedPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/most-decorated-divisions/MostDecoratedWidget.tsx', {
    // No division of the countries asked for is in service.
    '/dashboard/reachedProduct.ts': `export const useReachedProduct = (_client, _source, request) => { if (request) globalThis.__decoratedPending.tags.push(request.params.tags);
      return { data: request ? { branchId: '1', hour: request.params.hour, divisions: [], medals: [], templates: [], warnings: [], missing: false } : undefined, retry() {} }; };`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__decoratedPending.groups, retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${BRANCH.lastHour};`,
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
  });
  try {
    const props = { widget: { id: 'decorated', kind: 'most-decorated-divisions', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No divisions at this date/);
    // Read, GER is the human player then and has no division in service.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No divisions at this date\./);
    assert.deepEqual(state.tags.at(-1), ['GER']);
  } finally { delete host.__decoratedPending; await close(); }
});
