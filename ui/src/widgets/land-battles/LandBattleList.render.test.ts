import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { rankedLandBattleRows, validateLandBattlesRanking } from './landBattlesData.ts';
import type { LandBattle } from './landBattlesData.ts';

/** A battle of an older recording: no casualties were read, only its rosters (peak 5 divisions). */
const battle: LandBattle = { id: 'episode', provinceId: 17, start: { hour: 10, bound: 'exact' }, end: { hour: 21, bound: 'exact' },
  endUnknownHour: null, counterBase: 0, priorPeak: null, priorSidePeaks: null, losses: [], result: null,
  runs: [{ firstHour: 10, lastHour: 20, counts: [3, 2], rosterUnknown: false, countries: [['ATT'], ['DEF']], countriesUnknown: [false, false], duration: 1 }] };

test('Biggest shows the measure the ranking fell back to, not the one asked for', async () => {
  const replacements: Record<string, string> = {
    '/charts/log/LogList.tsx': `import { createElement, Fragment } from 'react';
      export default ({ rows, renderRow }) => createElement(Fragment, null, ...rows.map(row => createElement(Fragment, { key: row.key }, renderRow(row))));`,
    '/charts/drill/DrillHost.tsx': 'export default ({ main }) => main;',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['red', 'blue'] });`,
    '/dashboard/products.ts': 'export const useProduct = () => ({ retry() {} });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'land-battle-list-fixture', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const { default: List } = await vite.ssrLoadModule('/ui/src/widgets/land-battles/LandBattleList.tsx') as typeof import('./LandBattleList');
    const ranking = validateLandBattlesRanking({ labels: [], rows: [battle], censusHour: 20, effectiveMeasure: 'Peak size', warnings: [], contentWarnings: [] });
    const rows = rankedLandBattleRows(ranking, 20);
    assert.equal(rows[0].casualties.value, null, 'the recording has no casualties to rank by');
    const props = { rows, view: 'biggest', label: 'Land battles', live: false, menu: () => [], empty: 'None', onVisibleRows() {},
      country: (tag: string) => ({ name: tag }), log: { density: 'comfortable', months: false, animate: false },
      context: { runtime: { client: {} }, track: { source: { id: 'source', revision: '1' }, branch: { id: '1' } }, active: true,
        // The saved choice is still Casualties; the ranking fell back to peak size.
        widget: { options: { view: 'biggest', measure: 'Casualties' } }, shownHour: 20, rankedMeasure: 'Peak size', placePending: () => false } };
    const html = renderToStaticMarkup(createElement(List, props as never));
    const measure = /<span class="log-value lb-measure">([^<]*)<\/span>/.exec(html)?.[1];
    assert.equal(measure, '5', 'the column shows the peak the battle was ranked by');
  } finally { await vite.close(); }
});
