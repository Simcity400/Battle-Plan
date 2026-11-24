import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import type { ComponentProps } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { plotFrame } from '../frame.ts';
import { PENDING_ALPHA } from '../prepared.ts';
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

function makeCanvas() {
  const rects: { x: number; y: number; width: number; height: number; fillStyle: string; globalAlpha: number }[] = [];
  const ctx = {
    fillStyle: '',
    strokeStyle: '',
    globalAlpha: 1,
    lineWidth: 1,
    save() {},
    restore() {},
    beginPath() {},
    rect() {},
    clip() {},
    moveTo() {},
    lineTo() {},
    stroke() {},
    setLineDash() {},
    strokeRect() {},
    fillRect(x: number, y: number, width: number, height: number) {
      rects.push({ x, y, width, height, fillStyle: this.fillStyle, globalAlpha: this.globalAlpha });
    },
  };
  return { ctx: ctx as unknown as CanvasRenderingContext2D, rects };
}

const heatTokens = {
  ...tokensFromValues(() => '#8899aa', 'sans-serif'),
  surface: '#111111',
  text: '#eeeeee',
  muted: '#777777',
  error: '#cc2222',
  border: '#444444',
  gapOpacity: 0.35,
};

test('shared HeatChart keeps exact sub-pixel widths for Convoy health, Air superiority and Naval supremacy without inclusiveTerminal', () => {
  const tenYearSpan = 3650 * 24;
  const periods = [
    { firstHour: 0, lastHour: 23, readHour: 23 },
    { firstHour: 24, lastHour: 47, readHour: 47 },
  ];
  const expectedWidth = (24 / tenYearSpan) * 400;
  const frame = plotFrame(480, 120, { left: 60, right: 20, top: 20, bottom: 20 }, [0, tenYearSpan], [0, 1], false);
  for (const [label, look] of [['Convoy health', 'margin'], ['Air superiority timeline', 'margin'], ['Naval supremacy timeline', 'split']] as const) {
    const rows = [{ id: 'r', label: 'Region', cells: [{ parts: [{ id: 'p', label: 'P', color: '#33aa66', share: 1 }] }, { parts: [] }] }];
    renderToStaticMarkup(createElement(HeatChart, {
      label, rows, periods, extent: [0, tenYearSpan], branchStart: 0, playhead: tenYearSpan,
      look, active: true, revision: label, tooltip: () => null,
    }));
    const { ctx, rects } = makeCanvas();
    capture.heatChartProps!.layers[0].draw(ctx, frame, heatTokens, { playhead: tenYearSpan });
    assert.equal(rects.length, 1, `${label} draws only the active day`);
    assert.ok(Math.abs(rects[0].width - expectedWidth) < 1e-9, `${label} keeps exact sub-pixel width identical to main`);
  }
});

test('opt-in inclusiveTerminal reserves the right pixel for clear, missing, loading and snowy midnight states and hit target', () => {
  const rain = { parts: [{ id: 'rain', label: 'Rain', color: '#2277cc', share: 1 }] };
  const snow = { parts: [{ id: 'snow', label: 'Snow', color: '#ffffff', share: 1 }] };
  const clear = { parts: [] };
  const yesterday = { firstHour: 2136, lastHour: 2159, readHour: 2159 };
  const today = { firstHour: 2160, lastHour: 2160, readHour: 2160 };
  const frame = plotFrame(436, 120, { left: 80, right: 20, top: 20, bottom: 20 }, [1992, 2160], [0, 1], false);
  const plotRight = frame.plotLeft + frame.plotWidth;

  const hoveredPeriods: (number | null)[] = [];
  renderToStaticMarkup(createElement(HeatChart, {
    label: 'Weather', rows: [{ id: 'r1', label: 'Alps', cells: [rain] }], periods: [yesterday],
    tail: { period: today, cells: [clear] }, inclusiveTerminal: true,
    extent: [1992, 2160], branchStart: 0, playhead: 2160, look: 'split', active: true, revision: 1,
    tooltip: (_row, period) => { hoveredPeriods.push(period?.firstHour ?? null); return null; },
  }));
  const clearCanvas = makeCanvas();
  capture.heatChartProps!.layers[0].draw(clearCanvas.ctx, frame, heatTokens, { playhead: 2160 });
  const rainRect = clearCanvas.rects.find(r => r.fillStyle === '#2277cc');
  const clearRect = clearCanvas.rects.find(r => r.fillStyle === heatTokens.surface);
  assert.ok(rainRect && rainRect.x + rainRect.width === plotRight - 1, 'yesterday rain stops before the terminal pixel');
  assert.ok(clearRect && clearRect.x === plotRight - 1 && clearRect.width === 1, 'clear today paints surface over the terminal pixel');
  const snappedToday = capture.heatChartProps!.snap!(frame.hourAt(plotRight - 0.5), frame);
  assert.equal(snappedToday, 2160, 'pointer inside the terminal pixel snaps to today');
  capture.heatChartProps!.rows!.tooltip(0, snappedToday);
  assert.deepEqual(hoveredPeriods, [2160]);
  assert.equal(capture.heatChartProps!.snap!(frame.hourAt(plotRight - 2), frame), 2159, 'pointer left of terminal pixel hits yesterday');

  renderToStaticMarkup(createElement(HeatChart, {
    label: 'Weather', rows: [{ id: 'r1', label: 'Alps', cells: [rain] }], periods: [yesterday],
    tail: { period: today, cells: [snow] }, inclusiveTerminal: true,
    extent: [1992, 2160], branchStart: 0, playhead: 2160, look: 'split', active: true, revision: 2,
    tooltip: () => null,
  }));
  const snowCanvas = makeCanvas();
  capture.heatChartProps!.layers[0].draw(snowCanvas.ctx, frame, heatTokens, { playhead: 2160 });
  const snowRect = snowCanvas.rects.find(r => r.fillStyle === '#ffffff');
  assert.ok(snowRect && snowRect.x === plotRight - 1 && snowRect.width === 1, 'snowy today paints the terminal pixel');

  renderToStaticMarkup(createElement(HeatChart, {
    label: 'Weather', rows: [{ id: 'r1', label: 'Alps', cells: [rain, clear] }],
    periods: [yesterday, { firstHour: 2160, lastHour: 2160, readHour: null }], inclusiveTerminal: true,
    extent: [1992, 2160], branchStart: 0, playhead: 2160, look: 'split', active: true, revision: 3,
    tooltip: () => null,
  }));
  const gapCanvas = makeCanvas();
  capture.heatChartProps!.layers[0].draw(gapCanvas.ctx, frame, heatTokens, { playhead: 2160 });
  const gapRect = gapCanvas.rects.find(r => r.fillStyle === heatTokens.error && r.globalAlpha === heatTokens.gapOpacity);
  assert.ok(gapRect && gapRect.x === plotRight - 1 && gapRect.width === 1, 'missing today paints a gap stripe in the terminal pixel');

  renderToStaticMarkup(createElement(HeatChart, {
    label: 'Weather', rows: [{ id: 'r1', label: 'Alps', cells: [rain] }],
    periods: [yesterday], loading: [{ firstHour: 2160, lastHour: 2160 }], inclusiveTerminal: true,
    extent: [1992, 2160], branchStart: 0, playhead: 2160, look: 'split', active: true, revision: 4,
    tooltip: () => null,
  }));
  const loadingCanvas = makeCanvas();
  capture.heatChartProps!.layers[0].draw(loadingCanvas.ctx, frame, heatTokens, { playhead: 2160 });
  const pendingRect = loadingCanvas.rects.find(r => r.fillStyle === heatTokens.muted && r.globalAlpha === PENDING_ALPHA);
  assert.ok(pendingRect && pendingRect.x === plotRight - 1 && pendingRect.width === 1, 'loading today paints a pending stripe in the terminal pixel');
  assert.equal(capture.heatChartProps!.snap!(frame.hourAt(plotRight - 0.5), frame), 2160);
});
