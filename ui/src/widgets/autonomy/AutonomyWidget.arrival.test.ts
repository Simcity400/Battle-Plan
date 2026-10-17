import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { TimelineProps } from '../../charts/timeline/Timeline';

const flush = () => new Promise<void>(resolve => setImmediate(resolve));

// Retain the actual usePreparedWindow's hook cells/effects across SSR draws.
// The client controls reply order; the widget, query loop and merge stay real.
function hookCells() {
  type Cell = { value?: any; initialized?: boolean; deps?: readonly unknown[]; cleanup?: () => void };
  const cells: Cell[] = [];
  let cursor = 0, effects: (() => void)[] = [];
  const changed = (cell: Cell, deps: readonly unknown[]) => !cell.deps || cell.deps.length !== deps.length
    || deps.some((value, index) => !Object.is(value, cell.deps![index]));
  const hooks = {
    useState(initial: any) {
      const cell = cells[cursor++] ??= {};
      if (!cell.initialized) { cell.value = typeof initial === 'function' ? initial() : initial; cell.initialized = true; }
      return [cell.value, (next: any) => { cell.value = typeof next === 'function' ? next(cell.value) : next; }];
    },
    useRef(value: unknown) { return (cells[cursor++] ??= { value: { current: value } }).value; },
    useMemo(make: () => unknown, deps: readonly unknown[]) {
      const cell = cells[cursor++] ??= {};
      if (changed(cell, deps)) { cell.value = make(); cell.deps = [...deps]; }
      return cell.value;
    },
    useCallback(callback: unknown, deps: readonly unknown[]) { return hooks.useMemo(() => callback, deps); },
    useEffect(effect: () => (() => void) | undefined, deps: readonly unknown[]) {
      const cell = cells[cursor++] ??= {};
      if (changed(cell, deps)) { cell.deps = [...deps]; effects.push(() => { cell.cleanup?.(); cell.cleanup = effect(); }); }
    },
  };
  return { ...hooks, begin() { cursor = 0; effects = []; }, commit() { for (const effect of effects) effect(); },
    unmount() { for (const cell of cells) cell.cleanup?.(); } };
}

test('autonomy keeps its census unfinished in both arrival orders through usePreparedWindow', async () => {
  const span = { firstHour: 100, lastHour: 105 };
  const sampled = [{ firstHour: 100, lastHour: 102 }, { firstHour: 104, lastHour: 105 }];
  const skipped = [{ firstHour: 101, lastHour: 102 }, { firstHour: 105, lastHour: 105 }];
  const base = { branchId: '1', scoreScale: 100000, levels: [{ id: 'between/mod-level', name: 'Mod level', defined: true }],
    subjects: [], bands: [], summaries: {}, exactHours: [], recorded: [], unidentified: [], warnings: [],
    identities: [{ tag: 'A', identities: [{ ...span, slot: 1, countryId: 10 }] }],
    coverage: { firstHour: 100, lastHour: 99, complete: false } };
  const hooks = hookCells();
  const host = globalThis as typeof globalThis & { __autonomyArrivalHooks?: typeof hooks; __autonomyArrivalTimeline?: TimelineProps };
  const replacements: Record<string, string> = {
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({});',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag() {}, revision: 0 });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/dashboard/parityRows.ts': 'export const useParityRows = () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ surface: "#000000", text: "#ffffff", accent: "#ff0000", lineWidth: { series: 2 } });',
    '/charts/timeline/Timeline.tsx': 'export default props => { globalThis.__autonomyArrivalTimeline = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'autonomy-arrival-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/preparedSeries.ts')) return source.replace(
          "import { useCallback, useEffect, useMemo, useRef, useState } from 'react';",
          ['useCallback', 'useEffect', 'useMemo', 'useRef', 'useState'].map(name =>
            `const ${name} = (...args) => globalThis.__autonomyArrivalHooks.${name}(...args);`).join('\n'));
        return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      },
    }] });
  host.__autonomyArrivalHooks = hooks;
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/autonomy/AutonomyWidget.tsx');
    for (const skip of [false, true]) for (const first of ['series-state', 'series-summary']) for (const pointReady of [false, true]) {
      const cells = hookCells();
      host.__autonomyArrivalHooks = cells;
      const asks = new Map<string, { resolve: (value: unknown) => void; signal: AbortSignal }>();
      const statuses = new Map<AbortSignal, () => void>();
      let complete = false;
      const run = { firstHour: 101, lastHour: 101, startsSegment: true, level: 0, overlord: null, lowest: true, highest: true };
      const exact = { ...base, coverage: { ...span, complete: true }, recorded: skip ? skipped : sampled, pending: [], approximate: [],
        bands: [{ tag: 'A', runs: [run] }] };
      const client = {
        preparedQuery: async (_source: unknown, product: string, params: { reads?: { hour: number }[]; within?: object }, options: { signal: AbortSignal }) => {
          if (complete) return product === 'series-state' ? { ...exact, bands: [], exactHours: [105] } : exact;
          assert.equal(asks.has(product), false, 'no duplicate bootstrap asks');
          if (product === 'series-state') assert.deepEqual(params.reads, [{ hour: 105 }]);
          else assert.deepEqual(params.within, {});
          return new Promise(resolve => asks.set(product, { resolve, signal: options.signal }));
        },
        preparationStatus: (request: object, signal: AbortSignal) => new Promise((resolve, reject) => {
          signal.addEventListener('abort', () => reject(new DOMException('cancelled', 'AbortError')), { once: true });
          statuses.set(signal, () => resolve({ ...request, state: 'ready', sections: [], backgroundEnabled: true, bytes: '0', error: null,
            coverage: { '1': { ...span, complete: true } } }));
        }),
      };
      const props = { widget: { id: 'autonomy', kind: 'autonomy', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'campaign' }, options: {} },
        runtime: { client, tracks: [{ id: 'main', axisOffset: 0, branch: { id: '1', ...span },
          source: { id: 'source', revision: '1', metadata: { countries: [] } } }],
        clock: { getHour: () => 105, getSnapshot: () => ({ hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
      const render = () => {
        cells.begin(); delete host.__autonomyArrivalTimeline;
        const markup = renderToStaticMarkup(createElement(Widget, props));
        cells.commit(); return markup;
      };
      const publish = async (product: string) => {
        asks.get(product)!.resolve(product === 'series-state'
          ? { ...base, exactHours: [105], recorded: [{ firstHour: 105, lastHour: 105 }],
            coverage: pointReady ? exact.coverage : base.coverage, pending: pointReady ? [] : skip ? skipped : sampled }
          : { ...base, recorded: skip ? [] : sampled, approximate: skip ? [] : sampled, pending: skip ? skipped : [] });
        await flush();
      };
      try {
        render();
        assert.equal(asks.size, 2);
        await publish(first);
        let markup = render();
        assert.match(markup, /Subject data is still being prepared/, `${first} alone, skipped=${skip}, point ready=${pointReady}`);
        assert.doesNotMatch(markup, /No subjects found|was not recorded/);
        await publish(first === 'series-state' ? 'series-summary' : 'series-state');
        markup = render();
        assert.match(markup, /Subject data is still being prepared/);
        assert.doesNotMatch(markup, /No subjects found|was not recorded/);
        assert.equal(/Approximate/.test(markup), !skip);
        assert.equal(/Still being prepared/.test(markup), skip, 'reached history pending must not hide sampled geometry');
        complete = true;
        statuses.get(asks.get('series-summary')!.signal)!();
        await flush();
        markup = render();
        assert.doesNotMatch(markup, /still being prepared|Still being prepared|Approximate|No subjects found/);
        assert.equal(host.__autonomyArrivalTimeline!.lanes.length, 1, 'preparation discovers the between-keyframes subject');
        assert.deepEqual(host.__autonomyArrivalTimeline!.pending, [], 'new geometry clears stale reached-history pending');
        statuses.get(asks.get('series-state')!.signal)?.();
        await flush(); render();
      } finally { cells.unmount(); }
    }
  } finally {
    delete host.__autonomyArrivalHooks; delete host.__autonomyArrivalTimeline;
    await vite.close();
  }
});
