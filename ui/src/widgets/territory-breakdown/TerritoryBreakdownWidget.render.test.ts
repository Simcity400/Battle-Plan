import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply, TerritoryBreakdownReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('the territory of a group counted at an hour still being prepared is loading, not none', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  // ENG holds a state throughout; GER, the human player, holds none.
  const values = [1, 1, 7, null, 1, 1, 7, null, null, null, null, null];
  const data: TerritoryBreakdownReply = { branchId: BRANCH.id, regions: [{ id: 12, name: 'Modded region', missing: null }],
    observed: Array.from({ length: 8 }, () => [span]), areaUnit: null, areaReason: 'No raster', warnings: [],
    countries: [{ tag: 'GER', runs: [], segments: [span], membership: null, identities: [{ ...span, slot: 1, countryId: 101 }] },
      { tag: 'ENG', runs: [{ ...span, startsSegment: true, values }], segments: [span], membership: null, identities: [{ ...span, slot: 2, countryId: 102 }] }],
    worldTotals: [{ ...span, startsSegment: true, values }],
    series: [{ tag: 'ENG', region: 12, missing: null, runs: [{ ...span, startsSegment: true, values: ['1', '7', null, '1', '7', null],
      stateShares: [[{ numerator: '1', denominator: 1 }], [{ numerator: '1', denominator: 1 }]] }] }] };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __breakdownPending?: typeof state }; host.__breakdownPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/territory-breakdown/TerritoryBreakdownWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(data)}, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__breakdownPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag: () => undefined, flagPending: () => false, flagReason: () => undefined });',
    '/dashboard/FactionColours.tsx': 'export const useFactionColours = () => ({ names: () => ({ name: null, reason: null }), colours: new Map(), warnings: [], loading: false });',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null });',
    // The chart says its empty label when nothing is present at the hour.
    '/charts/sunburst/Sunburst.tsx': `import { createElement } from 'react';
      export default props => createElement('p', null, props.items.some(item => item.present) ? 'Territory drawn' : props.text.emptyLabel);`,
  });
  try {
    const props = { widget: { id: 'territory', kind: 'territory-breakdown', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No territory at this hour/);
    // Read, GER is the human player then and holds nothing.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No territory at this hour\./);
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'ENG', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /Territory drawn/);
  } finally { delete host.__breakdownPending; await close(); }
});
