import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { TimeChartProps } from '../../charts/TimeChart';
import { plotFrame } from '../../charts/frame.ts';
import type { ChartTokens } from '../../charts/tokens';
import type { ChartSummary } from '../../charts/chartSummary';
import type { TradeFlowsReply } from '../../generated/dashboard';

test('trade contributor and country histories draw rough spans immediately and preserve exact canvas paths', async () => {
  const state = { chart: undefined as TimeChartProps | undefined };
  const globals = globalThis as typeof globalThis & { __tradeHistory?: typeof state }; globals.__tradeHistory = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'trade-history-render', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/charts/TimeChart.tsx')) return 'export default props => { globalThis.__tradeHistory.chart = props; return null; };';
      if (path.endsWith('/dashboard/countryFlagsAt.ts')) return source.replace('export function useCountryLabelsAt(', 'function unusedCountryLabelsAt(')
        + '\nexport const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });';
    } }] });
  try {
    const { default: Drill } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/TradeDrillView.tsx');
    const { default: Country } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/TradeCountryView.tsx');
    const { decodeTrade, flowKey } = await vite.ssrLoadModule('/ui/src/widgets/trade-flows/tradeData.ts');
    const tokens = { surface: '#ffffff', muted: '#666666', text: '#111111', categories: ['#123456'], lineWidth: { series: 2, rule: 1 } } as unknown as ChartTokens;
    const frame = plotFrame(320, 200, { left: 10, right: 10, top: 10, bottom: 10 }, [0, 19], [-4, 4], false);
    const native: TradeFlowsReply = { branchId: '1', decimals: 5, warnings: [], countries: ['A', 'B'],
      countryIdentities: ['A', 'B'].map((tag, slot) => ({ tag, runs: [{ firstHour: 0, lastHour: 19, slot, countryId: slot + 1 }] })),
      resources: [{ id: 1, token: 'mod/resource', ordinal: 1, name: null, icon: null }], reads: [0, 19], known: [{ firstHour: 0, lastHour: 19 }], partial: [],
      flows: [{ seller: 0, buyer: 1, sellerIdentity: '[0,1]', buyerIdentity: '[1,2]', resource: 1, sellerTags: [0], buyerTags: [1],
        firstHours: [0], lastHours: [19], deliveredRaw: ['200000'], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] }] };
    const summary: ChartSummary = { from: 0, to: 19, width: Math.ceil(frame.plotWidth), runs: [{ firstHour: 0, lastHour: 19, value: 2, breakBefore: true }],
      buckets: [{ firstHour: 0, lastHour: 19, firstValue: 2, lastValue: 2, lastValueHour: 0, minHour: 0, minValue: 2, maxHour: 0, maxValue: 2, firstBreak: true, hasBreak: false }] };
    const props = { track: { branch: { id: '1', firstHour: 0, lastHour: 19, gaps: [] } }, runtime: { client: {}, clock: { getSnapshot: () => ({ hoursPerSecond: 0 }) } },
      measure: 'current', from: 0, hour: 19, extent: [0, 19], playhead: null, known: native.known, colours: new Map([[1, '#123456']]), tokens,
      active: true, onPick() {}, onContextMenu() {}, holder: '[0,1]', resources: null };
    const key = flowKey('[0,1]', '[1,2]', 1);
    for (const Component of [Drill, Country]) {
      const draw = (summary?: ChartSummary) => {
        const trade = decodeTrade(summary ? { ...native, summaries: { [key]: summary }, cumulativePending: true } : native);
        renderToStaticMarkup(createElement(Component, { ...props, trade, flows: trade.flows }));
        const commands: unknown[][] = [];
        const ctx = new Proxy({}, { get: (_target, field) => field === 'globalAlpha' ? 1 : (...args: unknown[]) => commands.push([field, ...args]),
          set: (_target, field, value) => { commands.push([field, value]); return true; } }) as CanvasRenderingContext2D;
        for (const layer of state.chart!.layers!) layer.draw?.(ctx, frame, tokens, { playhead: null, reveal: 1 } as never);
        return commands;
      };
      const legacy = draw();
      assert.deepEqual(draw(summary), legacy, 'exact windows retain the held legacy canvas path');
      const rough = draw({ ...summary, approximate: [{ firstHour: 10, lastHour: 19 }] });
      assert.ok(rough.some(command => command[0] === 'fill'), 'rough history paints before preparation reaches it');
      assert.ok(rough.some(command => command[0] === 'globalAlpha' && command[1] === 0.55));
      assert.ok(rough.some(command => command[0] === 'setLineDash' && JSON.stringify(command[1]) === '[4,4]'));
      assert.ok(rough.length < 250, 'approximate clipping uses the shared two-pass renderer');
    }
  } finally { await vite.close(); delete globals.__tradeHistory; }
});
