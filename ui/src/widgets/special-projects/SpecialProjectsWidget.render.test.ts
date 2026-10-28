import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply, SpecialProjectsReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('special projects of a group counted at hours still being prepared are loading, never none or loading for ever', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  // GER, the human player, never runs a special project.
  const reply: SpecialProjectsReply = { branchId: BRANCH.id, tag: 'GER', identities: [{ ...span, slot: 1, countryId: 101 }], available: [span],
    readHours: [0], projects: [], people: [], presentations: [], iterationTexts: {}, unavailable: null, warnings: [] };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __projectsPending?: typeof state }; host.__projectsPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/special-projects/SpecialProjectsWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__projectsPending.groups, retry() {} });',
    // As the real hook: nothing asked, nothing answered.
    '/dashboard/products.ts': `export const useProducts = (_client, requests) => ({ data: requests.length ? requests.map(() => (${JSON.stringify(reply)})) : undefined, retry() {} });
export const useFollowedProducts = (client, requests, _moving, ...rest) => ({ ...useProducts(client, requests, ...rest), requests });`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/timeline/Timeline.tsx': 'export default () => null;',
  });
  try {
    const props = (view: string) => ({ widget: { id: 'projects', kind: 'special-projects', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: { view } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} }) as unknown as WidgetProps;
    const render = (view: string) => renderToStaticMarkup(createElement(module.default, props(view)));
    // The branch end is still being prepared: its human players are not known yet.
    for (const view of ['live', 'history']) {
      const pending = render(view);
      assert.match(pending, /aria-label="This range is still being prepared\."/, view);
      assert.doesNotMatch(pending, /No special projects yet|No selected countries/, view);
    }
    // Read, GER is the human player throughout and has no project; no human player is one at the end.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render('history'), /No special projects yet\./);
    state.groups = pendingGroups({ final: true });
    assert.match(render('live'), /No selected countries were recorded at this moment\./);
  } finally { delete host.__projectsPending; await close(); }
});

test('live special projects of a group with no member found yet say what the moment was once read, and load until then', async () => {
  const state = { groups: { ...pendingGroups({ humans: [] }), unobserved: { humans: [{ firstHour: 140, lastHour: 150 }], factions: [], wars: [] } } as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __projectsGap?: typeof state }; host.__projectsGap = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/special-projects/SpecialProjectsWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__projectsGap.groups, retry() {} });',
    // Nothing asked, nothing answered.
    '/dashboard/products.ts': 'export const useProducts = (_client, requests) => ({ data: requests.length ? [] : undefined, retry() {} }); export const useFollowedProducts = (client, requests, _moving, ...rest) => ({ ...useProducts(client, requests, ...rest), requests });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/timeline/Timeline.tsx': 'export default () => null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'projects', kind: 'special-projects', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'live' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assert.match(render(145), /This group’s members were not recorded at this moment\./);
    assert.match(render(100), /This group has no members at this moment\./);
    const later = render(170);
    assert.match(later, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(later, /not recorded|no members/);
  } finally { delete host.__projectsGap; await close(); }
});
