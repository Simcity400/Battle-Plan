import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { LeaderRow, UnitLeadersData } from './generalsData';

test('leaderboard keeps the dated country selection when an alias is reused', async () => {
  const data: UnitLeadersData = { branchId: '1', firstHour: 100,
    selectionTags: ['original', 'alias', 'current'],
    people: [5, 6].map(characterId => ({ characterId, token: null, name: String(characterId), portraitHour: 100, portraitReference: String(characterId) })),
    roles: ['current', 'alias'].map(owner => ({ owner, category: 1, rank: null, kind: 1, maximumSkill: 10, attributeNames: [null, null, null, null] })),
    traits: [], leaders: [5, 6].map((id, index) => ({ reference: String(id),
      runs: [{ firstHour: 100, lastHour: 102, value: { person: index, role: index, skill: 3, components: [1, 2, 3, 4], traits: [] } }],
      traitGains: [], commands: [], battleStarts: [], battleStartReadHours: [], unknownBattleStarts: [], partialFrom: null })),
    observed: [{ firstHour: 100, lastHour: 102, leaders: true, landBattles: true, navalBattles: false, landCommands: true, navalCommands: false }],
    resultsMissing: 'Result not recorded', navyCommandsMissing: 'Navy commands not recorded', warnings: [] };
  const replacements: Record<string, string> = {
    '/widgets/generals/useLeaders.ts': `export const useLeaders = () => ({ data: ${JSON.stringify(data)}, retry() {} });
      export const useLeaderCountries = () => ({ countries: new Set(['current']), requestSelection: { kind: 'countries', tags: ['original'] }, group: {} });`,
    '/widgets/generals/portraits.ts': 'export const useLeaderPortraits = () => ({ at() {}, warnings: [] });',
    '/widgets/general-career/CareerView.tsx': 'export default () => null;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 102;',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/charts/table/Table.tsx': 'export default props => { globalThis.__selectedLeaders = props.rows; return null; };',
  };
  const host = globalThis as typeof globalThis & { __selectedLeaders?: readonly LeaderRow[] };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'leader-selection-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/generals/GeneralsWidget.tsx') as typeof import('./GeneralsWidget.tsx');
    const props = { widget: { id: 'generals', kind: 'generals', trackIds: ['main'], countries: { kind: 'tags', tags: ['original'] },
      time: { mode: 'campaign' }, options: {} }, runtime: { tracks: [{ id: 'main', axisOffset: 0,
        source: { id: 'synthetic', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 102 } }],
        clock: { getHour: () => 102 }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    renderToStaticMarkup(createElement(Widget, props));
    assert.deepEqual(host.__selectedLeaders?.map(row => row.key), ['5']);
  } finally {
    delete host.__selectedLeaders;
    await vite.close();
  }
});
