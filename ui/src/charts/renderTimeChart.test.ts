import assert from 'node:assert/strict';
import test from 'node:test';
import { firstHourOfMonth } from '../dashboard/calendar.ts';
import { VALUE_LABEL_GAP } from './draw.ts';
import {
  TIME_CHART_DEFAULT_PADDING,
  TIME_CHART_LABEL_MARGIN,
  TIME_CHART_MIN_SPAN,
  drawTimeChartScene,
  fitTimeChartFrame,
  resolveTimeChartTargetX,
  resolveTimeChartTargetY,
} from './renderTimeChart.ts';
import { canvasMeasureText, clearTextWidthCache, type Canvas2D, type MeasureText } from './textWidth.ts';
import type { ChartTokens } from './tokens.ts';
import { EMPTY_VIEW, type AxisSettings } from './view.ts';

const baseTokens: ChartTokens = {
  surface: '#111111',
  raised: '#1a1a1a',
  text: '#eeeeee',
  muted: '#888888',
  border: '#333333',
  grid: '#222222',
  axis: '#555555',
  accent: '#4488ff',
  positive: '#44bb66',
  warning: '#ddaa33',
  error: '#dd4444',
  font: 'Inter, sans-serif',
  fontSize: { small: 11, body: 13, title: 15 },
  lineWidth: { grid: 1, series: 2, emphasis: 3 },
  hatchSpacing: 8,
  gapOpacity: 0.2,
  motion: 'smooth',
  reducedMotion: false,
  countryColours: 'raw',
  categories: ['#112233', '#445566', '#778899'],
};

const defaultAxes: AxisSettings = { xFrom: null, xTo: null, yMin: null, yMax: null, log: false };

function mockCanvas2D() {
  const calls: unknown[][] = [];
  const state = {
    globalAlpha: 1,
    strokeStyle: '',
    fillStyle: '',
    lineWidth: 1,
    lineJoin: 'miter',
    font: '',
    textAlign: 'left',
    textBaseline: 'alphabetic',
  };
  const ctx = new Proxy(state, {
    get(target, key) {
      if (key in target) return Reflect.get(target, key);
      if (key === 'measureText') {
        return (text: string) => {
          calls.push(['measureText', text]);
          return { width: text.length * 7 };
        };
      }
      return (...args: unknown[]) => {
        calls.push([String(key), ...args]);
      };
    },
    set(target, key, value) {
      calls.push([String(key), value]);
      if (key in target) Reflect.set(target, key, value);
      return true;
    },
  }) as unknown as Canvas2D;
  return { ctx, calls };
}

test('resolveTimeChartTargetX resolves draw-in minimum span, window, and cursor modes', () => {
  const extent: [number, number] = [1000, 5000];
  assert.deepEqual(
    resolveTimeChartTargetX({
      extent,
      playhead: null,
      playback: 'draw-in',
      paused: true,
      view: EMPTY_VIEW,
      axes: defaultAxes,
      branchStart: 1000,
    }),
    extent,
  );

  const shortDrawIn = resolveTimeChartTargetX({
    extent,
    playhead: 1050,
    playback: 'draw-in',
    paused: true,
    view: EMPTY_VIEW,
    axes: defaultAxes,
    branchStart: 1000,
  });
  assert.deepEqual(shortDrawIn, [1000, 1000 + TIME_CHART_MIN_SPAN]);

  const cursorMode = resolveTimeChartTargetX({
    extent,
    playhead: 2000,
    playback: 'cursor',
    paused: false,
    view: EMPTY_VIEW,
    axes: defaultAxes,
    branchStart: 1000,
  });
  assert.deepEqual(cursorMode, extent);

  const windowed = resolveTimeChartTargetX({
    extent,
    playhead: 3000,
    playback: 'window',
    windowHours: 720,
    paused: true,
    view: EMPTY_VIEW,
    axes: defaultAxes,
    branchStart: 1000,
  });
  assert.deepEqual(windowed, [2280, 3000]);

  const clampedWindow = resolveTimeChartTargetX({
    extent,
    playhead: 3000,
    playback: 'window',
    windowHours: 240,
    paused: true,
    view: EMPTY_VIEW,
    axes: defaultAxes,
    branchStart: 1000,
  });
  assert.deepEqual(clampedWindow, [3000 - TIME_CHART_MIN_SPAN, 3000]);
});

test('fitTimeChartFrame expands left padding from injected MeasureText without DOM', () => {
  const measured: string[] = [];
  const wideMeasure: MeasureText = text => {
    measured.push(text);
    return text.length * 20;
  };
  const frame = fitTimeChartFrame({
    layoutWidth: 400,
    layoutHeight: 200,
    padding: TIME_CHART_DEFAULT_PADDING,
    x: [0, 240],
    y: [0, 100_000],
    log: false,
    tokens: baseTokens,
    measure: wideMeasure,
  });
  assert.ok(measured.length > 0);
  const maxWidth = Math.max(...measured.map(text => text.length * 20));
  assert.equal(frame.plotLeft, maxWidth + VALUE_LABEL_GAP + TIME_CHART_LABEL_MARGIN);
});

test('fitTimeChartFrame and drawTimeChartScene run cleanly on a Canvas2D mock and consult injected measure for dates', () => {
  const measuredDuringDraw: string[] = [];
  let recording = false;
  const customMeasure: MeasureText = text => {
    if (recording) measuredDuringDraw.push(text);
    return text.length * 8;
  };
  const jan1936 = firstHourOfMonth(1936 * 12);
  const end1936 = firstHourOfMonth(1937 * 12);
  const mid1936 = firstHourOfMonth(1936 * 12 + 6);
  const layerCalls: number[] = [];
  let overlayCalled = false;
  const x = resolveTimeChartTargetX({
    extent: [jan1936, end1936],
    playhead: mid1936,
    playback: 'cursor',
    paused: true,
    view: EMPTY_VIEW,
    axes: defaultAxes,
    branchStart: jan1936,
  });
  const y = resolveTimeChartTargetY([{ extent: () => [0, 500] }], x, EMPTY_VIEW, defaultAxes);
  const frame = fitTimeChartFrame({
    layoutWidth: 520,
    layoutHeight: 240,
    padding: TIME_CHART_DEFAULT_PADDING,
    x,
    y: y.range,
    log: y.log,
    tokens: baseTokens,
    measure: customMeasure,
  });

  const { ctx, calls } = mockCanvas2D();
  recording = true;
  drawTimeChartScene(
    ctx,
    frame,
    baseTokens,
    {
      layoutWidth: 520,
      layoutHeight: 240,
      scaleX: 2,
      scaleY: 2,
      playhead: mid1936,
      playback: 'cursor',
      layers: [
        {
          draw: (_c, _f, _t, state) => {
            layerCalls.push(state.playhead ?? -1);
          },
        },
      ],
      overlay: () => {
        overlayCalled = true;
      },
    },
    customMeasure,
  );
  recording = false;

  assert.deepEqual(calls[0], ['setTransform', 2, 0, 0, 2, 0, 0]);
  assert.deepEqual(calls[1], ['clearRect', 0, 0, 520, 240]);
  assert.deepEqual(layerCalls, [mid1936]);
  assert.equal(overlayCalled, true);
  assert.equal(calls.some(call => call[0] === 'measureText'), false);
  assert.ok(measuredDuringDraw.some(text => /1936|Apr|Jul|Oct/.test(text)), `date labels measured: ${JSON.stringify(measuredDuringDraw)}`);
});

test('canvasMeasureText keys relative font shorthands by resolved canvas font', () => {
  clearTextWidthCache();
  const makeContext = (resolvedPx: number) => {
    let rawFont = `${resolvedPx}px serif`;
    return {
      get font() {
        return rawFont === '1em serif' ? `${resolvedPx}px serif` : rawFont;
      },
      set font(value: string) {
        rawFont = value;
      },
      measureText(text: string) {
        return { width: text.length * resolvedPx } as TextMetrics;
      },
    };
  };
  const smallCtx = makeContext(12);
  const largeCtx = makeContext(20);
  const measureSmall = canvasMeasureText(smallCtx);
  const measureLarge = canvasMeasureText(largeCtx);
  assert.equal(measureSmall('100', '1em serif'), 36);
  assert.equal(measureLarge('100', '1em serif'), 60);
  assert.equal(canvasMeasureText(smallCtx), measureSmall);
});
