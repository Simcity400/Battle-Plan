import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import { reply } from './transfers.fixture.ts';

test('no transfers while the group hours they are matched at are still being prepared is loading, not none for these countries', async () => {
  const state = { groups: pendingGroups(), data: reply([]) };
  const host = globalThis as typeof globalThis & { __transfersPending?: typeof state }; host.__transfersPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/territory-transfers/TerritoryTransfersWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__transfersPending.groups });',
    '/dashboard/usePeaceDeals.ts': 'export const usePeaceDeals = (widget, runtime) => ({ track: runtime.tracks[0], data: globalThis.__transfersPending.data, preparing: false, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/useStateNamesAt.ts': 'export const useStateNamesAt = () => () => ({ name: "State" });',
  });
  try {
    const props = { widget: { id: 'transfers', kind: 'territory-transfers', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'campaign' } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No territory transfers/);
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No territory transfers for these countries yet\./);
  } finally { delete host.__transfersPending; await close(); }
});
