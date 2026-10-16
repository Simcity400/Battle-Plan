import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { LegendItem } from './Legend';
import type { MovingRow } from './rowMotion';
import type { ParityList } from '../dashboard/parityRows';

test('eager and automatic windowed Legends expose exact items only in parity builds', async () => {
  const state = { read: undefined as (() => ParityList) | undefined };
  const host = globalThis as typeof globalThis & { __eagerLegend?: typeof state };
  host.__eagerLegend = state;
  const items: LegendItem[] = Array.from({ length: 5000 }, (_, index) => ({ id: `mod_series_${index}`,
    label: index ? `Recorded ${index}` : '', value: String(index), tooltip: `Exact ${index}` }));
  try {
    for (const count of [20, 5000]) {
      let instrumentedMarkup: string | undefined;
      const selected = items.slice(0, count);
      for (const mode of ['parity', 'production']) {
        const vite = await createServer({ configFile: false, mode, optimizeDeps: { noDiscovery: true },
          server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'eager-legend-model', enforce: 'pre',
            transform(_source, id) {
              if (id.replaceAll('\\', '/').endsWith('/charts/useChartTokens.ts'))
                return 'export const useChartTokens = () => ({ reducedMotion: true, fontSize: { small: 10 } });';
              if (id.replaceAll('\\', '/').endsWith('/dashboard/parityRows.ts'))
                return 'export const useParityRows = (_root, read) => { globalThis.__eagerLegend.read = read; };';
            },
          }] });
        try {
          state.read = undefined;
          const { default: Legend } = await vite.ssrLoadModule('/ui/src/charts/Legend.tsx');
          const markup = renderToStaticMarkup(createElement(Legend, { items: selected, label: 'Recorded series' }));
          if (mode === 'production') { assert.equal(state.read, undefined); assert.equal(markup, instrumentedMarkup, 'instrumentation leaves ordinary DOM unchanged'); }
          else {
            instrumentedMarkup = markup;
            const list = state.read!();
            assert.equal(list.complete, true); assert.equal(list.total, count);
            assert.deepEqual(list.rows, selected.map((item, index) => ({ key: item.id, index, item })));
          }
        } finally { await vite.close(); }
      }
    }
  } finally { delete host.__eagerLegend; }
});

test('windowed Legend reports mounted and leaving entries, releases unmount and hides small inventories', async () => {
  const state = { effects: [] as (() => unknown)[], layouts: [] as (() => unknown)[], rows: undefined as {
    items: readonly { key: string; item: LegendItem }[]; onVisibleRows: (rows: readonly MovingRow<LegendItem>[]) => void } | undefined };
  const host = globalThis as typeof globalThis & { __legendVisible?: typeof state };
  host.__legendVisible = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'legend-mounted-flags', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/Legend.tsx')) return source.replace(/\buseEffect\(/g,
          '(effect => globalThis.__legendVisible.effects.push(effect))(').replace(/\buseLayoutEffect\(/g,
          '(effect => globalThis.__legendVisible.layouts.push(effect))(');
        if (path.endsWith('/charts/WindowedRows.tsx')) return 'export default props => { globalThis.__legendVisible.rows = props; return null; };';
        if (path.endsWith('/charts/useChartTokens.ts')) return 'export const useChartTokens = () => ({ fontSize: { small: 10 } });';
      },
    }] });
  try {
    const { default: Legend } = await vite.ssrLoadModule('/ui/src/charts/Legend.tsx');
    const items: LegendItem[] = Array.from({ length: 5000 }, (_, index) => ({ id: `native_${index}`, label: `Recorded ${index}` }));
    const reported: (readonly LegendItem[])[] = [];
    const onVisibleItems = (visible: readonly LegendItem[]) => reported.push(visible);
    const render = (selected: readonly LegendItem[], callback = onVisibleItems) => {
      state.effects = []; state.layouts = []; state.rows = undefined;
      renderToStaticMarkup(createElement(Legend, { items: selected, label: 'Countries', windowed: true, onVisibleItems: callback }));
      const run = (effects: readonly (() => unknown)[]) => effects.map(effect => effect()).filter((cleanup): cleanup is () => void => typeof cleanup === 'function');
      return { layouts: run(state.layouts), effects: run(state.effects) };
    };
    const cleanups = render(items);
    assert.equal(state.rows!.items.length, 5000, 'the full keyboard inventory is retained');
    const mounted = [0, 4999].map((index, slot) => ({ key: items[index].id, item: items[index], slot,
      phase: slot ? 'leaving' : 'staying', since: 0 } as MovingRow<LegendItem>));
    state.rows!.onVisibleRows(mounted);
    assert.deepEqual(reported.at(-1), [items[0], items[4999]], 'only actual mounted rows request artwork');
    const changed = render(items, visible => reported.push(visible));
    for (const cleanup of cleanups.layouts) cleanup();
    state.rows!.onVisibleRows(mounted);
    for (const cleanup of cleanups.effects) cleanup();
    assert.deepEqual(reported.at(-1), [items[0], items[4999]], 'old passive cleanup cannot clear a newly published viewport');
    for (const cleanup of [...changed.layouts, ...changed.effects]) cleanup();
    assert.deepEqual(reported.at(-1), [], 'unmount releases the artwork set');
    render([items[0]]);
    assert.equal(state.rows, undefined);
    assert.deepEqual(reported.at(-1), [], 'a hidden single-entry legend asks for no flag');
  } finally { await vite.close(); delete host.__legendVisible; }
});
