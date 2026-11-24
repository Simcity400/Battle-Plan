import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import type { ComponentProps } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';

const capture = globalThis as typeof globalThis & { heatChartProps?: ComponentProps<typeof import('../TimeChart.tsx').default> };

const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{
  name: 'heat-chart-props',
  enforce: 'pre',
  transform(code, id) {
    if (id.replaceAll('\\', '/').endsWith('/heat/HeatChart.tsx')) return code
      .replace("from '../TimeChart'", "from 'heat-time-chart'")
      .replace("from '../useChartTokens'", "from 'heat-tokens'");
    if (id.replaceAll('\\', '/').endsWith('/charts/Legend.tsx')) return code.replace("from './useChartTokens'", "from 'heat-tokens'");
  },
  resolveId(id) {
    if (id === 'heat-time-chart') return '\0heat-time-chart';
    if (id === 'heat-tokens') return '\0heat-tokens';
  },
  load(id) {
    if (id === '\0heat-time-chart') return `import { createElement } from 'react'; export default function Chart(props) {
      globalThis.heatChartProps = props;
      return createElement('span', { 'data-keys': props.keyHours.join(','), 'data-snaps': [2,52,30].map(props.snap).join(',') }); }`;
    if (id === '\0heat-tokens') return `export const useChartTokens = () => (${JSON.stringify(tokensFromValues(() => '#8899aa', 'sans-serif'))});`;
  },
}] });
const { default: HeatChart } = await vite.ssrLoadModule('/ui/src/charts/heat/HeatChart.tsx').finally(() => vite.close()) as typeof import('./HeatChart.tsx');




test('a pick names the row under the pointer and its period, and no pick is offered without a handler', () => {
  const periods = [{ firstHour: 0, lastHour: 23, readHour: 23 }, { firstHour: 24, lastHour: 47, readHour: 47 }];
  const rows = ['North', 'South'].map(label => ({ id: label, label, cells: periods.map(() => ({ parts: [] })) }));
  const base = { label: 'Picks', periods, rows, extent: [0, 47] as [number, number], branchStart: 0, playhead: null, look: 'margin' as const,
    active: true, revision: 1, tooltip: () => null };
  renderToStaticMarkup(createElement(HeatChart, base));
  assert.equal(capture.heatChartProps!.onPick, undefined);
  const picked: [string, number | null, number][] = [];
  renderToStaticMarkup(createElement(HeatChart, { ...base, onPick: (row, period, hour) => picked.push([row.id, period?.firstHour ?? null, hour]) }));
  capture.heatChartProps!.onPick!(30);
  assert.deepEqual(picked, [['North', 24, 30]]);
});
