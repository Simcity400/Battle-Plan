import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply } from '../../generated/dashboard';
import type { CasualtyPairsReply } from './casualtyPairsData';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('casualty pairs of a group counted at an hour still being prepared are loading, not none', async () => {
  const span = { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour };
  // ENG fights D01 throughout; GER, the human player, is in no recorded pair.
  const reply: CasualtyPairsReply = { branchId: BRANCH.id, available: true, windows: [span], window: 0, observed: [span], hasGap: false, warnings: [],
    pairs: [{ identity: '2:102|3:103', firstCountry: 'ENG', secondCountry: 'D01', runs: [{ firstHour: 10, lastHour: 200,
      readHour: 10, hourly: false, firstTaken: '5', secondTaken: '7', incomplete: false }] }] };
  const tags = ['GER', 'ENG', 'D01'];
  const state = { groups: pendingGroups({ tags }) as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __pairsPending?: typeof state }; host.__pairsPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/casualties-between/CasualtyPairsWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify({ ...reply, window: null, observed: [], pairs: [] })}, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': `export const useProductWindows = (_client, identity) => ({ data: identity ? new Map([[0, ${JSON.stringify(reply)}]]) : new Map(), errors: new Map(), retry() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__pairsPending.groups, retry() {} });',
    '/dashboard/playback.ts': `export const usePlaybackHour = clock => clock.getHour(); export const usePlaybackMotion = () => 0;`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsAt = () => new Map();',
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "#777777";',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123456"], muted: "#777777", surface: "#000000", countryColours: "raw" }); export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null, open() {} });',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/charts/chord/ChordChart.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default () => null;',
    '/charts/table/Table.tsx': 'export const CountryCell = () => null; export default () => null;',
  });
  try {
    const props = { widget: { id: 'pairs', kind: 'casualties-between', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ playing: false, seekSequence: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No recorded country pairs/);
    // Read, GER is the human player then and is in no pair.
    state.groups = pendingGroups({ final: true, tags, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No recorded country pairs at this moment\./);
  } finally { delete host.__pairsPending; await close(); }
});
