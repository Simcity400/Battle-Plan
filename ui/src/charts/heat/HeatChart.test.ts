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

test('HeatChart wires its keyboard and pointer props to each period read hour', () => {
  const periods = [{ firstHour: 0, lastHour: 23, readHour: 18 }, { firstHour: 24, lastHour: 47, readHour: null },
    { firstHour: 48, lastHour: 71, readHour: 69 }];
  const html = renderToStaticMarkup(createElement(HeatChart, { label: 'Reads', periods, rows: [], extent: [0, 71],
    branchStart: 0, playhead: null, look: 'margin', active: true, revision: 1, tooltip: () => null }));
  assert.match(html, /data-keys="18,69"/);
  assert.match(html, /data-snaps="18,69,30"/);
  const folded = renderToStaticMarkup(createElement(HeatChart, { label: 'Reads', periods: periods.map(period => ({ ...period, exact: false })), rows: [],
    extent: [0, 71], branchStart: 0, playhead: null, look: 'margin', active: true, revision: 1, tooltip: () => null }));
  assert.match(folded, /data-snaps="2,52,30"/);
  assert.match(folded, /data-keys="18,69"/);
});

test('a recording-sized legend keeps every row reachable without growing or drawing on the canvas', () => {
  const legend = Array.from({ length: 5000 }, (_, index) => ({ id: String(index), label: `Mod side ${index}`, color: '#112233' }));
  legend.splice(1, 0, { id: 'empty', label: 'Blank = no air activity', color: '' });
  const html = renderToStaticMarkup(createElement(HeatChart, { label: 'Legend', periods: [], rows: [], extent: [0, 71],
    branchStart: 0, playhead: null, look: 'margin', active: true, revision: 1, legend, tooltip: () => null }));
  assert.match(html, /aria-setsize="5001"/);
  assert.match(html, /Mod side 0/);
  assert.match(html, /Blank = no air activity/);
  assert.ok((html.match(/role="listitem"/g) ?? []).length < 50);
  assert.doesNotMatch(html, /Mod side 4999/);
  assert.equal(capture.heatChartProps!.label, 'Legend');
  assert.equal(typeof capture.heatChartProps!.overlay, 'function');
  assert.equal(capture.heatChartProps!.padding!.bottom, 8);
  assert.equal(capture.heatChartProps!.rows!.labelWidth, 0);
});

test('a 5,000-row plot only builds runs and labels for its viewport and margin', () => {
  let cellsReached = 0;
  const rows = Array.from({ length: 5000 }, (_, index) => ({ id: String(index), label: `Region ${index}`,
    get cells() { cellsReached++; return [{ parts: [{ id: 'side', label: 'Side', color: '#112233', share: 1 }] }]; } }));
  const html = renderToStaticMarkup(createElement(HeatChart, { label: 'Regions', periods: [{ firstHour: 0, lastHour: 23, readHour: 23 }],
    rows, extent: [0, 23], branchStart: 0, playhead: null, look: 'split', active: true, revision: 1, tooltip: () => null }));
  const chart = capture.heatChartProps!;
  assert.ok(chart.rows!.labels.length < 50);
  assert.doesNotMatch(html, /heat-chart-plot[^>]*height:650/);
  const heights = [...html.matchAll(/height:(\d+)px/g)].map(match => Number(match[1]));
  assert.ok(heights[0] > 60_000, 'the logical spacer retains every row');
  assert.ok(heights.slice(1).every(height => height < 500), 'the drawing surface stays bounded');
  const drawn: number[] = [];
  chart.layers[0].draw({ fillRect: (_x: number, y: number) => { drawn.push(y); } } as unknown as CanvasRenderingContext2D,
    { plotLeft: 30, plotTop: 22, plotWidth: 100, plotHeight: chart.rows!.labels.length * 13,
      x: [0, 23], toX: (hour: number) => 30 + hour * 100 / 23 } as unknown as Parameters<typeof chart.layers[0]['draw']>[1],
    tokensFromValues(() => '#8899aa', 'sans-serif'), { playhead: null });
  assert.equal(cellsReached, chart.rows!.labels.length);
  assert.equal(drawn.length, cellsReached);
});

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
