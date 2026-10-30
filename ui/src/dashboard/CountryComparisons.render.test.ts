import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType, ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from './definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from './pendingGroups.fixture.ts';
import type { CountryGroupReply } from '../generated/dashboard';

test('comparisons of a group with no member read yet are loading, and a group proven empty says so', async () => {
  const state = { groups: pendingGroups({ humans: [] }) as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __comparisonsPending?: typeof state }; host.__comparisonsPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps & { component: ComponentType<WidgetProps> }) => ReactElement }>('/ui/src/dashboard/CountryComparisons.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__comparisonsPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
  });
  try {
    const props = { widget: { id: 'stockpile', kind: 'equipment-stockpile', trackIds: ['main'], countries: humansBy('ever'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {}, component: () => 'panel' } as unknown as WidgetProps & { component: ComponentType<WidgetProps> };
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // No human player has been read yet, and the later hours are still being prepared.
    assert.match(render(), /aria-label="This range is still being prepared\."/);
    // The final reply proves the group never had a member.
    state.groups = pendingGroups({ final: true, humans: [] });
    assert.match(render(), /This group has no members in this campaign\./);
  } finally { delete host.__comparisonsPending; await close(); }
});
