import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('agencies of a group counted at an hour still being prepared are loading, not none', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  // ENG has an agency throughout; GER, the human player, never does.
  const data = { branchId: BRANCH.id, available: true, observed: [span], operativeReadHours: [0], defenseLabel: 'Defence', warnings: [],
    series: [{ tag: 'ENG', segments: [span], membership: null, rosters: [], entries: [{ ...span, value: { name: 'Synthetic agency', created: true,
      upgrades: { nonnegativeLevelSum: 0 }, defenseRaw: '0', defenseValue: '0', activeCount: 0, maximum: 0 } }] }] };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __agencyPending?: typeof state }; host.__agencyPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/agency-comparison/AgencyComparisonWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = request => ({ data: request ? ${JSON.stringify(data)} : undefined, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__agencyPending.groups, retry() {} });',
    '/widgets/agency-comparison/IntelSection.tsx': 'export default () => null;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesAt = () => new Map();',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${BRANCH.lastHour};`,
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/table/Table.tsx': 'export const CellBar = () => null; export const CountryCell = () => null; export default () => null;',
  });
  try {
    const props = { widget: { id: 'agency', kind: 'agency-comparison', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No recorded agencies match/);
    // Read, GER is the human player then and has no agency.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No recorded agencies match these countries and settings at this hour\./);
  } finally { delete host.__agencyPending; await close(); }
});
