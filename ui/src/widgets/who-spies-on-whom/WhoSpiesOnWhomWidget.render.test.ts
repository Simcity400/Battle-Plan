import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('placements of a group counted at an hour still being prepared are loading, not none placed', async () => {
  // ENG spies on D01 throughout; GER, the human player, places no operative.
  const reply = { branchId: BRANCH.id, warnings: [], readHours: [0], periods: [{ firstHour: 0, lastHour: BRANCH.lastHour, availability: 'recorded' }],
    pairs: [{ id: 'opaque', owner: 'ENG', target: 'D01', runs: [{ firstHour: 0, lastHour: BRANCH.lastHour, count: 1, operatives: [{ id: '1', name: null, unnamed: true }] }] }] };
  const tags = ['GER', 'ENG', 'D01'];
  const state = { groups: pendingGroups({ tags }) as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __placementsPending?: typeof state }; host.__placementsPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/who-spies-on-whom/WhoSpiesOnWhomWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(reply)}, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__placementsPending.groups, retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsAt = () => new Map();',
    '/dashboard/playback.ts': `export const usePlaybackHour = clock => clock.getHour();`,
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ muted: '#888888', surface: '#111111', countryColours: 'game' });",
    '/charts/chord/ChordChart.tsx': 'export default () => null;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null });',
  });
  try {
    const props = { widget: { id: 'placements', kind: 'who-spies-on-whom', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [], playerCountryId: null } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ position: BRANCH.lastHour, seekSequence: 0, playing: false, hoursPerSecond: 1 }) }, client: {} },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No operatives placed yet/);
    // Read, GER is the human player then and places no operative.
    state.groups = pendingGroups({ final: true, tags, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No operatives placed yet\./);
  } finally { delete host.__placementsPending; await close(); }
});
