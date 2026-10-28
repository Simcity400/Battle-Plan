import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { rankedLandBattleRows, validateLandBattlesRanking } from './landBattlesData.ts';
import type { LandBattle } from './landBattlesData.ts';
import type { LogListProps } from '../../charts/log/LogList';
import type { LandBattleRow } from './landBattlesData';

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
    '/dashboard/products.ts': 'export const useProduct = () => ({ retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
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

test('the redesigned list prepares raw full-history rows in log order and keeps Biggest order only in parity builds', async () => {
  const host = globalThis as typeof globalThis & { __landParity?: LogListProps<LandBattleRow> };
  const ranking = validateLandBattlesRanking({ labels: [], rows: [battle], censusHour: 20, effectiveMeasure: 'Peak size', warnings: [], contentWarnings: [] });
  const earlier = { ...rankedLandBattleRows(ranking, 20)[0], id: 'earlier' };
  const later = { ...earlier, id: 'later', start: { hour: 30, bound: 'exact' as const }, end: { hour: 41, bound: 'exact' as const } };
  const prepare = () => {}, reads = { windows: [{ index: 0, loaded: true }] };
  const preparation = { id: 'stable-list', prepare, ready: () => false };
  const props = { rows: [{ ...earlier, place: { ...earlier.place, name: 'Drawn location' } }], view: 'log', label: 'Land battles', live: false,
    menu: () => [], empty: 'None', onVisibleRows() {}, country: (tag: string) => ({ name: tag }),
    log: { density: 'comfortable', months: false, animate: false },
    paritySemantic: { rows: [earlier, later], complete: false, historyComplete: false, preparation, reads },
    context: { runtime: { client: {} }, track: { source: { id: 'source', revision: '1' }, branch: { id: '1' } }, active: true,
      widget: { options: {} }, shownHour: 50, rankedMeasure: 'Peak size', placePending: () => false } };
  const replacements: Record<string, string> = {
    '/charts/log/LogList.tsx': 'export default props => { globalThis.__landParity = props; return null; };',
    '/charts/drill/DrillHost.tsx': 'export default ({ main }) => main;',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['red', 'blue'] });`,
    '/dashboard/products.ts': 'export const useProduct = () => ({ retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
  };
  for (const mode of ['parity', 'test']) {
    const vite = await createServer({ mode, configFile: false, optimizeDeps: { noDiscovery: true },
      server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'land-parity-fixture', enforce: 'pre',
        transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
      }] });
    try {
      const { default: List } = await vite.ssrLoadModule('/ui/src/widgets/land-battles/LandBattleList.tsx') as typeof import('./LandBattleList');
      const render = () => { renderToStaticMarkup(createElement(List, props as never)); return host.__landParity!; };
      props.rows = [{ ...earlier, place: { ...earlier.place, name: 'Drawn location' } }];
      props.paritySemantic.rows = [earlier, later];
      props.view = 'log';
      const log = render();
      assert.equal(log.rows?.[0].item.place.name, 'Drawn location', 'rendered rows retain ordinary drawn names');
      if (mode === 'parity') {
        assert.equal(log.parityHistoryComplete, false);
        assert.equal(log.paritySemantic?.complete, false);
        assert.equal(log.paritySemantic?.preparation, preparation);
        assert.equal(log.paritySemantic?.reads, reads);
        assert.deepEqual(log.paritySemantic?.rows?.map(row => [row.id, row.hour]), [['later', 41], ['earlier', 10]]);
        assert.equal(log.paritySemantic?.rows?.[1].item, earlier, 'full rows retain raw operands without deferred names');
        props.view = 'biggest';
        assert.deepEqual(render().paritySemantic?.rows?.map(row => [row.id, row.hour]), [['earlier', 10], ['later', 30]],
          'Biggest preserves ranking order and dates each battle by its start');
        props.rows = [];
        props.paritySemantic.rows = [];
        const empty = render();
        assert.deepEqual(empty.paritySemantic?.rows, []);
        assert.equal(empty.paritySemantic?.preparation, preparation, 'an empty custom view keeps preparation reachable');
      } else {
        assert.equal(Object.hasOwn(log, 'paritySemantic'), false);
        assert.equal(Object.hasOwn(log, 'parityHistoryComplete'), false);
      }
    } finally { await vite.close(); delete host.__landParity; }
  }
});
