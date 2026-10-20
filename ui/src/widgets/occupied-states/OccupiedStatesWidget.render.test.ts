import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply, OccupiedStatesManifestReply, OccupiedStatesReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('occupied states of a group counted at an hour still being prepared are loading, not none', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  const manifest: OccupiedStatesManifestReply = { branchId: BRANCH.id, decimals: 5, windows: [span], laws: [], strengthMissing: 'Not recorded', warnings: [] };
  // A complete reading with no occupied state of the human player.
  const window: OccupiedStatesReply = { branchId: BRANCH.id, window: 0, interval: span, texts: [], states: [], warnings: [],
    observed: [{ ...span, readHour: BRANCH.lastHour, complete: true }] };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __occupiedPending?: typeof state }; host.__occupiedPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/occupied-states/OccupiedStatesWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(manifest)}, retry() {} });`,
    '/dashboard/productWindows.ts': `export const useProductWindows = () => ({ data: new Map([[0, ${JSON.stringify(window)}]]), errors: new Map(), retry() {} });
      export const useWindowDisplay = (_identity, data, hour) => data ? { data, hour } : undefined;`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__occupiedPending.groups, retry() {} });',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryNamesAt = () => new Map();',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${BRANCH.lastHour};`,
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/table/Table.tsx': 'export const CellBar = () => null; export const CountryCell = () => null; export default () => null;',
  });
  try {
    const props = { widget: { id: 'occupied', kind: 'occupied-states', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No occupied states match|unavailable for these countries/);
    // Read, GER is the human player then and occupies nothing.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No occupied states match these countries at this moment\./);
  } finally { delete host.__occupiedPending; await close(); }
});
