import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { DashboardSource, ProductRequest } from '../../dashboard/client';
import type { WidgetProps } from '../../dashboard/definitions';

test('the breakdown widget supplies prepared sources for its manifest and window reads', async () => {
  const calls: { kind: string; source: DashboardSource; request: ProductRequest; identity?: string }[] = [];
  const globals = globalThis as typeof globalThis & { __breakdownRouting?: typeof calls };
  globals.__breakdownRouting = calls;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export function useProduct(_client, request, _active, _validate, source) {
      globalThis.__breakdownRouting.push({ kind: 'manifest', request, source });
      return { data: { branchId: '1', tag: 'mod holder', available: false, windows: [{firstHour: 100,lastHour: 101}],
        window: null, observed: [], countries: [], series: [], wars: [], hasGap: false, warnings: [] }, retry() {} }; }
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': `export function useProductWindows(_client, _identity, request, _wanted, _active, _validate, source) {
      globalThis.__breakdownRouting.push({ kind: 'window', request: request(0), source, identity: _identity });
      return { data: new Map(), errors: new Map(), retry() {} }; }`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour(); export const usePlaybackMotion = () => 24;',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => `${hour}:${tag}`; export const useCountryNamesAt = () => new Map(); export const useCountryFactsRead = () => new Map();',
    '/dashboard/countryFlags.ts': 'export const useCountryFlagsRead = () => new Map();',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123"] }); export const useViewportHeight = () => 1000;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'breakdown-routing-fixture', enforce: 'pre',
      transform: (_source, id) => { const path = id.replaceAll('\\', '/'); const key = Object.keys(replacements).find(key => path.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/casualties-breakdown/CasualtiesBreakdownWidget.tsx');
    const source = { id: 'prepared-source', revision: 'revision' } as DashboardSource;
    const props = { widget: { id: 'breakdown', trackIds: ['track'], countries: { kind: 'tags', tags: ['mod holder'] },
      time: { mode: 'campaign' }, options: {} }, runtime: { client: {}, clock: { getHour: () => 100 }, tracks: [{ id: 'track',
        axisOffset: 0, source, branch: { id: '1', firstHour: 100, lastHour: 101 } }] }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    assert.match(renderToStaticMarkup(createElement(Widget, props)), /Losses by war were not recorded/);
    assert.deepEqual(calls.map(call => call.kind), ['manifest', 'window']);
    for (const call of calls) {
      assert.equal(call.source, source);
      assert.deepEqual(call.request, { sourceId: source.id, revision: source.revision, product: 'casualty-breakdown',
        params: { branchId: '1', tag: 'mod holder', window: call.kind === 'manifest' ? null : 0 } });
    }
    const before = calls.find(call => call.kind === 'window')!.identity;
    const { contentRevision } = await vite.ssrLoadModule('/ui/src/dashboard/contentRevision.ts');
    contentRevision(props.runtime.client).observe({ game: 'synthetic mod content', user: '', contentReady: true });
    calls.length = 0;
    renderToStaticMarkup(createElement(Widget, props));
    assert.notEqual(calls.find(call => call.kind === 'window')!.identity, before, 'content changes reload the windows carrying dated war names');
  } finally { await vite.close(); delete globals.__breakdownRouting; }
});
