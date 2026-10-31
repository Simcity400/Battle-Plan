import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { TimeChartProps } from '../../charts/TimeChart';
import { plotFrame } from '../../charts/frame.ts';
import { dayLabel } from '../../charts/draw.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ChartTokens } from '../../charts/tokens';
import { formatBalance } from './measureData.ts';
import { wireSummary } from '../../charts/chartSummary.fixture.ts';

test('actual balance widget uses prepared viewport, exact hour and keyboard reads with no mount history scans', async () => {
  type Slot = { value?: unknown; deps?: readonly unknown[]; cleanup?: () => void };
  const tag = 'opaque / country', raw = '9223372036854775807';
  const fixture = { cursor: 0, slots: [] as Slot[], effects: [] as (() => void)[], chart: undefined as TimeChartProps | undefined,
    products: [] as { product: string; active: boolean }[], queries: [] as { product: string; params: Record<string, unknown>; signal: AbortSignal }[],
    tokens: { surface: '#000', muted: '#888', lineWidth: { series: 2 }, countryColours: 'raw', categories: ['#123'] } as unknown as ChartTokens,
    styles: { known: new Map([[tag, { tag, name: tag, color: [20, 50, 80] }]]), data: { warnings: [] } },
    drill: undefined as { open: boolean; day: string; onClose: () => void } | undefined, held: undefined as Map<number, () => void> | undefined };
  const host = globalThis as typeof globalThis & { __preparedBalanceFixture?: typeof fixture; __preparedBalanceHook?: (name: string, ...args: unknown[]) => unknown };
  host.__preparedBalanceFixture = fixture;
  host.__preparedBalanceHook = (name, ...args) => {
    const slot = fixture.slots[fixture.cursor++] ??= {};
    if (name === 'useMemo' || name === 'useCallback' || name === 'useEffect') {
      const [factory, deps] = args as [() => unknown, readonly unknown[]];
      if (!slot.deps || slot.deps.length !== deps.length || deps.some((value, index) => !Object.is(value, slot.deps![index]))) {
        slot.deps = [...deps];
        if (name === 'useMemo') slot.value = factory();
        else if (name === 'useCallback') slot.value = factory;
        else fixture.effects.push(() => { slot.cleanup?.(); const cleanup = factory(); slot.cleanup = typeof cleanup === 'function' ? cleanup as () => void : undefined; });
      }
      return slot.value;
    }
    if (!('value' in slot)) slot.value = name === 'useReducer' ? args[1] : name === 'useRef' ? { current: args[0] } : args[0];
    if (name === 'useRef') return slot.value;
    return [slot.value, (next: unknown) => { slot.value = name === 'useReducer'
      ? (args[0] as (previous: unknown, action: unknown) => unknown)(slot.value, next)
      : typeof next === 'function' ? (next as (previous: unknown) => unknown)(slot.value) : next; }];
  };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = (_client, request, active) => {
      if (request) globalThis.__preparedBalanceFixture.products.push({product:request.product,active}); return {retry(){}};
    }; export const useProducts = (_client, requests, active) => {
      for (const request of requests) globalThis.__preparedBalanceFixture.products.push({product:request.product,active}); return {retry(){}};
    };
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });
export const useFollowedProducts = (client, requests, _moving, ...rest) => ({ ...useProducts(client, requests, ...rest), requests });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({retry(){}});',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/useCountryLineStyles.ts': 'export const useCountryLineStyles = () => globalThis.__preparedBalanceFixture.styles;',
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour,tag) => JSON.stringify([hour,tag]);
      export const useCountryNamesRead = (_client,_track,wanted) => new Map(wanted.map(row => [nameKey(row.hour,row.tag),row.tag]));`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt:()=>tag=>({name:tag,known:true}),revision:0});',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({flag(){},flagPending:()=>false,flagReason(){}});',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => globalThis.__preparedBalanceFixture.tokens; export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({open(){},menu:null});',
    '/charts/Legend.tsx': 'export default () => null;',
    '/charts/TimeChart.tsx': 'export default props => {globalThis.__preparedBalanceFixture.chart=props; return null;};',
    '/charts/drill/DrillHost.tsx': 'export default props => { globalThis.__preparedBalanceFixture.drill = props; return props.main; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'prepared-balance-fixture', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
      if (replacement) return replacement[1];
      if (path.endsWith('/widgets/country-measure/MeasureWidget.tsx') || path.endsWith('/widgets/country-measure/preparedMeasure.ts') || path.endsWith('/dashboard/viewRead.ts'))
        return source.replace(/\b(useMemo|useCallback|useState|useReducer|useRef|useEffect)(?:<[\s\S]*?>)?\(/g,
          (_match, name) => `globalThis.__preparedBalanceHook(${JSON.stringify(name)}, `);
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/country-measure/MeasureWidget.tsx');
    const source = { id: 'source', revision: '1', path: 'synthetic', metadata: { campaignKey: 'synthetic', branches: [], countries: [] } };
    let lateNavigation: ((reply: unknown) => void) | undefined;
    const client = { preparedQuery: async (_source: unknown, product: string, params: Record<string, unknown>, { signal }: { signal: AbortSignal }) => {
      fixture.queries.push({ product, params, signal });
      if (product === 'series-next-change') {
        if (params.key === 'ArrowRight') return new Promise(resolve => { lateNavigation = resolve; });
        return { hour: 90 };
      }
      assert.deepEqual(params.family, { kind: 'country-measure', measure: 'politicalPower' });
      const reads = params.reads as { tag: string; hour: number }[];
      const states = () => ({ branchId: 'branch', measure: 'politicalPower', decimals: 5, warnings: [],
        states: reads.map(read => ({ ...read,
          run: read.hour === 50 ? null : { firstHour: read.hour < 50 ? 0 : 51, lastHour: read.hour < 50 ? 49 : 100,
            valueRaw: raw, slot: 7, countryId: 11, startsSegment: true }, drop: null })) });
      // A chart pick's exact read, held until the test lets it finish.
      if (product === 'series-state' && fixture.held && reads.length === 1 && [20, 30, 40].includes(reads[0].hour))
        return new Promise(resolve => fixture.held!.set(reads[0].hour, () => resolve(states())));
      if (product === 'series-state') return states();
      assert.equal(product, 'series-summary');
      if (Number(params.to) > 100) return null;
      const width = params.width as number;
      return { branchId: 'branch', measure: 'politicalPower', decimals: 5, warnings: [], series: [{ tag,
        summary: wireSummary({ from: params.from, to: params.to, width,
          runs: [{ firstHour: 0, lastHour: 49, value: Number(raw) / 1e5, breakBefore: true },
            { firstHour: 51, lastHour: 100, value: Number(raw) / 1e5, breakBefore: true }],
          buckets: Array.from({ length: Math.floor(Number(params.to)) - Math.floor(Number(params.from)) + 1 }, (_, index) => {
            const hour = Math.floor(Number(params.from)) + index;
            return hour === 50 ? null : { firstHour: hour, lastHour: hour,
            firstValue: Number(raw) / 1e5, lastValue: Number(raw) / 1e5, lastValueHour: hour,
            minHour: hour, minValue: Number(raw) / 1e5, maxHour: hour, maxValue: Number(raw) / 1e5,
            firstBreak: hour === 0 || hour === 51, hasBreak: false }; }) }), marks: null }] };
    }, preparationStatus: async (_request: object, signal: AbortSignal) => new Promise((_resolve, reject) => {
      signal.addEventListener('abort', () => reject(new DOMException('Cancelled', 'AbortError')), { once: true });
    }) };
    const props = { config: { measure: 'politicalPower', title: 'Political power', noun: 'political power', unit: 'PP' },
      widget: { id: 'balance', kind: 'political-power', trackIds: ['track'], countries: { kind: 'tags', tags: [tag] }, time: { mode: 'campaign' },
        options: { endLabels: false, ceiling: false, decimals: 2, spendTicks: false } },
      runtime: { client, tracks: [{ id: 'track', axisOffset: 0, source, branch: { id: 'branch', firstHour: 0, lastHour: 100 } }],
        clock: { getHour: () => 100, getSnapshot: () => ({ playing: false, hoursPerSecond: 24 }) } }, active: true, headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => { fixture.cursor = 0; fixture.effects = []; fixture.chart = undefined;
      const html = renderToStaticMarkup(createElement(Widget, props)); for (const effect of fixture.effects) effect(); return html; };
    const settle = async () => { await new Promise(resolve => setImmediate(resolve)); render(); await new Promise(resolve => setImmediate(resolve)); render(); };
    render(); await settle();
    assert.ok(fixture.chart);
    assert.ok(fixture.products.every(request => !request.active), 'no full balance/ledger/maximum history starts on mount');
    fixture.chart.onSummaryView!({ from: 0, to: 100, width: 100 }); render(); await settle();
    const frame = plotFrame(100, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 2e14], false);
    let operations = 0;
    let curves = 0;
    const context = { save(){},restore(){},beginPath(){},moveTo(){operations++;},lineTo(){operations++;},bezierCurveTo(){operations++; curves++;},stroke(){},fill(){},arc(){},closePath(){},setLineDash(){} } as unknown as CanvasRenderingContext2D;
    fixture.chart!.layers[0].draw(context, frame, fixture.tokens, { playhead: null });
    assert.ok(operations > 0 && operations <= 100 * 27, 'line and area paths are bounded by viewport width');
    assert.ok(curves > 0, 'exact country balances use the legacy cubic path');
    assert.deepEqual(fixture.chart!.layers[0].extent?.([0, 100]), [Number(raw) / 1e5, Number(raw) / 1e5]);
    fixture.chart!.onHover!(90); render(); await settle();
    const tooltip = fixture.chart!.tooltip!(90, frame, []);
    assert.equal(tooltip!.rows[0].value, `${formatBalance(raw, 5, 2)} PP`, 'geometry f64 never supplies the exact tooltip');
    fixture.chart!.onHover!(50); render(); await settle();
    assert.equal(fixture.chart!.tooltip!(50, frame, [])!.rows[0].value, 'Not recorded');
    // Picks finish out of order: only the newest still-open one chooses the breakdown.
    fixture.held = new Map();
    const pickAt = (hour: number) => fixture.chart!.onPick!(hour, { frame, until: null, point: { x: frame.toX(hour), y: frame.toY(Number(raw) / 1e5) } });
    pickAt(20); pickAt(30);
    fixture.held.get(30)!(); await settle();
    assert.equal(fixture.drill!.open, true);
    assert.equal(fixture.drill!.day, dayLabel(30));
    fixture.held.get(20)!(); await settle();
    assert.equal(fixture.drill!.day, dayLabel(30), 'an older pick finishing last does not take the breakdown back');
    pickAt(40); fixture.drill!.onClose(); render();
    fixture.held.get(40)!(); await settle();
    assert.equal(fixture.drill!.open, false, 'a pick still reading when the breakdown closes does not reopen it');
    fixture.held = undefined;
    fixture.chart!.onSummaryView!({ from: 0, to: 101, width: 100 }); render(); await settle();
    assert.match(render(), /Preparing the requested balance hours/);
    assert.ok(fixture.chart, 'pending terminal coverage retains the bounded chart and exact reached values');
    operations = 0;
    const laterFrame = plotFrame(100, 100, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 101], [0, 2e14], false);
    fixture.chart!.layers[0].draw(context, laterFrame, fixture.tokens, { playhead: null });
    assert.ok(operations > 0, 'the landed summary keeps painting its hours while the next viewport prepares');
    fixture.chart!.onNavigateHour!('ArrowLeft', 100, [0, 100], null); await settle();
    assert.equal(fixture.chart!.keyboardHour, 90);
    fixture.chart!.onNavigateHour!('ArrowRight', 90, [0, 100], null);
    const pending = fixture.queries.at(-1)!;
    props.runtime.tracks[0].source = { ...props.runtime.tracks[0].source, id: 'other source' };
    const before = render(); assert.equal(fixture.chart, undefined, 'a source switch masks reached values immediately');
    assert.ok(before.includes('Loading'));
    assert.ok(pending.signal.aborted);
    lateNavigation!({ hour: 99 }); await settle();
    assert.equal((fixture.chart as TimeChartProps | undefined)?.keyboardHour, undefined, 'late keyboard replies cannot reclaim another source');
  } finally {
    for (const slot of fixture.slots) slot.cleanup?.();
    await vite.close(); delete host.__preparedBalanceFixture; delete host.__preparedBalanceHook;
  }
});
