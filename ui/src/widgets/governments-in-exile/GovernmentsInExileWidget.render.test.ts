import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('no exile at an hour whose group membership is still being prepared is loading, not none', async () => {
  const state = { groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __exilesPending?: typeof state }; host.__exilesPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/governments-in-exile/GovernmentsInExileWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__exilesPending.groups, retry() {} });',
    '/dashboard/products.ts': `export const loadTransientProduct = () => new Promise(() => {}); export const useProduct = () => ({ retry() {}, data: { lastHour: 200, known: [{ firstHour: 0, lastHour: 200 }], exileHistory: [], holders: [], warnings: [] } });`,
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 200;',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ fontSize: { title: 12, small: 10 } }); export const useViewportHeight = () => 1000;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesAt = () => new Map();',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
  });
  try {
    const props = { widget: { id: 'governments-in-exile', kind: 'governments-in-exile', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'campaign' } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The hour shown is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No governments in exile at this date\./);
    // The final reply proves there is no human player then.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No governments in exile at this date\./);
  } finally { delete host.__exilesPending; await close(); }
});
