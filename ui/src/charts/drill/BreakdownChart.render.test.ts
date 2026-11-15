import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { TimeChartProps } from '../TimeChart';
import { plotFrame } from '../frame.ts';
import type { ChartTokens } from '../tokens';
import type { ChartSummary } from '../chartSummary';

function canvas() {
  const commands: unknown[][] = [];
  const context = new Proxy({}, { get: (_target, key) => key === 'globalAlpha' ? 1 : (...args: unknown[]) => commands.push([key, ...args]),
    set: (_target, key, value) => { commands.push([key, value]); return true; } }) as CanvasRenderingContext2D;
  return { context, commands };
}

test('BreakdownChart keeps the exact signed canvas path and uses shared dense geometry', async () => {
  const state = { chart: undefined as TimeChartProps | undefined };
  const globals = globalThis as typeof globalThis & { __preparedBreakdown?: typeof state }; globals.__preparedBreakdown = state;
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'prepared-breakdown-render', enforce: 'pre', transform(_source, id) {
      if (id.replaceAll('\\', '/').endsWith('/charts/TimeChart.tsx')) return 'export default props => { globalThis.__preparedBreakdown.chart = props; return null; };';
    } }] });
  try {
    const { default: Breakdown } = await vite.ssrLoadModule('/ui/src/charts/drill/BreakdownChart.tsx') as { default: ComponentType<Record<string, unknown>> };
    const tokens = { surface: '#ffffff', lineWidth: { series: 2 } } as ChartTokens;
    const frame = plotFrame(320, 200, { left: 10, right: 10, top: 10, bottom: 10 }, [100.25, 119.5], [-10, 10], false);
    const runs = [{ firstHour: 100, lastHour: 104, value: 2, breakBefore: true }, { firstHour: 108, lastHour: 120, value: 5, breakBefore: true }];
    const exact = (values: typeof runs): ChartSummary => ({ from: 100.25, to: 119.5, width: 300, buckets: [], runs: values });
    const parts = [{ id: 'positive', label: 'Produced', color: '#123456', runs, amount: String },
      { id: 'other', label: 'Other', color: '#abcdef', runs: runs.map(run => ({ ...run, value: -run.value })), amount: String }];
    const props = { label: 'Parts', extent: [100, 120], playhead: null, active: true, series: parts };
    const draw = (input: Record<string, unknown>) => {
      renderToStaticMarkup(createElement(Breakdown, input));
      const recorded = canvas();
      for (const layer of state.chart!.layers!) layer.draw?.(recorded.context, frame, tokens, { playhead: null, reveal: 1 } as never);
      return recorded.commands;
    };
    const legacy = draw(props);
    const exactSummaries = draw({ ...props, summaries: { parts: parts.map(part => exact(part.runs)), total: exact(runs.map(run => ({ ...run, value: 0 }))) } });
    assert.deepEqual(exactSummaries, legacy, 'exact summary metadata cannot change a legacy path');
    const bucket = { firstHour: 100, lastHour: 119, firstValue: 2, lastValue: 5, lastValueHour: 118, minHour: 105, minValue: 1,
      maxHour: 112, maxValue: 9, firstBreak: true, hasBreak: false };
    const summary: ChartSummary = { from: 100.25, to: 119.5, width: Math.ceil(frame.plotWidth), buckets: [bucket] };
    const dense = draw({ ...props, summaries: { parts: parts.map(() => summary), total: { ...summary, buckets: [{ ...bucket, firstValue: 0, lastValue: 0, minValue: 0, maxValue: 0 }] } } });
    assert.notDeepEqual(dense, legacy);
    assert.ok(dense.some(command => command[0] === 'fill'));
    assert.ok(dense.length < 200, 'the signed stack emits only its bounded display samples');
    const totalFree = draw({ ...props, summaries: { parts: parts.map(() => summary) } });
    assert.notDeepEqual(totalFree, legacy, 'a complete parts inventory uses dense geometry without an authoritative total');
    assert.ok(totalFree.some(command => command[0] === 'fill'));
    assert.ok(totalFree.length < 200);
    const late = draw({ ...props, summaries: { parts: parts.map(() => ({ ...summary, width: summary.width + 1 })) } });
    assert.ok(!late.some(command => command[0] === 'fill'), 'a reply for another width cannot paint this one');
  } finally { await vite.close(); delete globals.__preparedBreakdown; }
});
