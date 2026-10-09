import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { BattleViewAdapter } from './BattleLogView';

test('actual battle loader refuses aligned Follow and stored Pin moments outside its branch', async () => {
  const replacements: Record<string, string> = {
    '/charts/table/useTableHour.ts': 'export const useTableHour = clock => clock.getHour();',
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({
      data: request?.product.endsWith('-manifest') ? { windows: [{ firstHour: 100, lastHour: 200 }],
        available: true, unavailable: null, contentWarnings: [] } : undefined, retry() {} });`,
    '/dashboard/productWindows.ts': `export const useProductWindows = (_client, _identity, _request, wanted, active) => {
      globalThis.__battleWindowLoads.push({ wanted, active });
      return { data: new Map([[0, { observed: [[100, 200]], warnings: [], contentWarnings: [] }]]), errors: new Map(), retry() {} };
    };`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/widgets/battles/BattleLog.tsx': 'export default () => null;',
  };
  const host = globalThis as typeof globalThis & { __battleWindowLoads?: { wanted: number[]; active: boolean }[] };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'battle-window-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/widgets/battles/BattleLogView.tsx') as typeof import('./BattleLogView');
    const { resolveWindow } = await vite.ssrLoadModule('/ui/src/dashboard/model.ts') as typeof import('../../dashboard/model');
    let position = -850;
    const props = { widget: { id: 'battle', kind: 'naval-battles', trackIds: ['short'], countries: { kind: 'all' }, time: { mode: 'follow' } },
      runtime: { tracks: [{ id: 'short', axisOffset: 1000, source: { id: 'synthetic', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 200 } }],
        clock: { getHour: () => position, getSpeed: () => 24, subscribe: () => () => {} }, client: {} }, active: true, onDiagnostics() {},
      adapter: { definition: { title: 'Naval battles', icon: 'shield', options: [] }, product: 'naval-battles',
        validateManifest: (value: unknown) => value, validateWindow: (value: unknown) => value, validateRanking: (value: unknown) => value,
        rows: () => [], rankedRows: () => [], labels: {}, measures: [{ label: 'Ships sunk' }] } as unknown as BattleViewAdapter<never, never, never>,
    } as unknown as WidgetProps & { adapter: BattleViewAdapter<never, never, never> };
    host.__battleWindowLoads = [];
    const track = props.runtime.tracks[0];
    assert.equal(resolveWindow(props.widget, track, [track], position)?.lastHour, 150);
    assert.doesNotMatch(renderToStaticMarkup(createElement(View<never, never, never>, props)), /outside this campaign branch/);
    for (const later of [-950, -750]) {
      position = later;
      host.__battleWindowLoads = [];
      assert.match(renderToStaticMarkup(createElement(View<never, never, never>, props)), /outside this campaign branch/);
      assert.deepEqual(host.__battleWindowLoads, [{ wanted: [], active: false }]);
    }
    props.widget = { ...props.widget, time: { mode: 'pin', windows: { short: { firstOffset: 101, lastOffset: 101 } } } };
    assert.equal(resolveWindow(props.widget, track, [track], position)?.lastHour, 201);
    assert.match(renderToStaticMarkup(createElement(View<never, never, never>, props)), /outside this campaign branch/);
    props.widget = { ...props.widget, time: { mode: 'pin', windows: { short: { firstOffset: 5, lastOffset: 5 } } } };
    position = 5000;
    assert.equal(resolveWindow(props.widget, track, [track], position)?.lastHour, 105);
    assert.doesNotMatch(renderToStaticMarkup(createElement(View<never, never, never>, props)), /outside this campaign branch/);
  } finally {
    delete host.__battleWindowLoads;
    await vite.close();
  }
});
