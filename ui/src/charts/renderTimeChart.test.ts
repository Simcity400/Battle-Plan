import assert from 'node:assert/strict';
import test from 'node:test';
import { firstHourOfMonth } from '../dashboard/calendar.ts';
import { BADGE_STRIP } from './badges.ts';
import { VALUE_LABEL_GAP, drawAxes } from './draw.ts';
import { plotFrame, STRIP_GAP } from './frame.ts';
import {
  TIME_CHART_DEFAULT_PADDING,
  TIME_CHART_LABEL_MARGIN,
  TIME_CHART_MIN_SPAN,
  drawTimeChartScene,
  fitTimeChartFrame,
  normalizeStripHeight,
  resolveAxisFormatter,
  resolveChartFit,
  resolveTimeChartPadding,
  resolveTimeChartTargetX,
  resolveTimeChartTargetY,
  stabilityZoomFit,
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

test('resolveTimeChartPadding handles badges, strip rounding, custom overrides, and row labels', () => {
  const plain = resolveTimeChartPadding({
    layoutWidth: 400,
    tokens: baseTokens,
    stripHeight: 0,
    hasBadges: false,
    hasBadgeSummary: false,
  });
  assert.deepEqual(plain, TIME_CHART_DEFAULT_PADDING);

  const withBadgesAndStrip = resolveTimeChartPadding({
    layoutWidth: 400,
    tokens: baseTokens,
    stripHeight: 18,
    hasBadges: true,
    hasBadgeSummary: false,
  });
  assert.equal(withBadgesAndStrip.top, BADGE_STRIP);
  assert.equal(withBadgesAndStrip.bottom, TIME_CHART_DEFAULT_PADDING.bottom + 18 + STRIP_GAP);

  const fractionalPadding = resolveTimeChartPadding({
    layoutWidth: 400,
    tokens: baseTokens,
    stripHeight: 48.48,
    hasBadges: false,
    hasBadgeSummary: false,
  });
  const roundedPadding = resolveTimeChartPadding({
    layoutWidth: 400,
    tokens: baseTokens,
    stripHeight: 48,
    hasBadges: false,
    hasBadgeSummary: false,
  });
  const fractionalStrip = fitTimeChartFrame({
    layoutWidth: 400,
    layoutHeight: 202,
    padding: fractionalPadding,
    x: [0, 720],
    y: [0, 100],
    log: false,
    stripHeight: 48.48,
    hasRows: false,
    hasExplicitLeftPadding: false,
    tokens: baseTokens,
    measure: text => text.length * 7,
  });
  const roundedStrip = fitTimeChartFrame({
    layoutWidth: 400,
    layoutHeight: 202,
    padding: roundedPadding,
    x: [0, 720],
    y: [0, 100],
    log: false,
    stripHeight: 48,
    hasRows: false,
    hasExplicitLeftPadding: false,
    tokens: baseTokens,
    measure: text => text.length * 7,
  });
  assert.equal(normalizeStripHeight(48.48), 48);
  assert.equal(fractionalStrip.plotHeight, roundedStrip.plotHeight);
  assert.deepEqual(fractionalStrip.strip, roundedStrip.strip);

  const withRows = resolveTimeChartPadding({
    layoutWidth: 200,
    tokens: baseTokens,
    stripHeight: 0,
    hasBadges: false,
    hasBadgeSummary: false,
    rowLabelWidth: 150,
  });
  assert.equal(withRows.left, 90);
  assert.equal(withRows.top, baseTokens.fontSize.small * 2 + 4);
  assert.equal(withRows.bottom, 8);
});

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

test('declarative formatters, fractional stabilityZoom fit, and resolveTimeChartTargetY match chart semantics', () => {
  assert.equal(resolveAxisFormatter('powerAxis')(1500, []), '1.5k');
  assert.equal(resolveAxisFormatter('powerAxis')(420.4, []), '420');
  assert.equal(resolveAxisFormatter('percent')(50, [0, 50, 100]), '50%');
  assert.equal(resolveAxisFormatter()(10, [0, 10]), '10');
  assert.equal(resolveChartFit('none'), undefined);

  const stabilityFit = resolveChartFit('stabilityZoom');
  assert.equal(stabilityFit, stabilityZoomFit);
  assert.equal(stabilityFit!(null), null);
  assert.deepEqual(stabilityFit!([0.6, 0.7]), [0.59, 0.71]);
  assert.deepEqual(stabilityFit!([0.5, 0.5]), [0.49, 0.51]);

  const target = resolveTimeChartTargetY(
    [{ extent: () => [0.6, 0.7] }, { extent: () => [0.62, 0.75] }],
    [0, 100],
    EMPTY_VIEW,
    { ...defaultAxes, yMin: null, yMax: null },
    'stabilityZoom',
  );
  assert.equal(target.log, false);
  assert.deepEqual(target.range, [0.58, 0.77]);

  const pinnedLow = resolveTimeChartTargetY(
    [{ extent: () => [0.6, 0.7] }],
    [0, 100],
    EMPTY_VIEW,
    { ...defaultAxes, yMin: 0 },
    'stabilityZoom',
  );
  assert.equal(pinnedLow.range[0], 0);
  assert.equal(pinnedLow.range[1], 0.71);
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
    stripHeight: 0,
    hasRows: false,
    hasExplicitLeftPadding: false,
    tokens: baseTokens,
    formatValue: 'axisValue',
    measure: wideMeasure,
  });
  assert.ok(measured.length > 0);
  const maxWidth = Math.max(...measured.map(text => text.length * 20));
  assert.equal(frame.plotLeft, maxWidth + VALUE_LABEL_GAP + TIME_CHART_LABEL_MARGIN);

  const locked = fitTimeChartFrame({
    layoutWidth: 400,
    layoutHeight: 200,
    padding: { ...TIME_CHART_DEFAULT_PADDING, left: 36 },
    x: [0, 240],
    y: [0, 100_000],
    log: false,
    stripHeight: 0,
    hasRows: false,
    hasExplicitLeftPadding: true,
    tokens: baseTokens,
    formatValue: 'axisValue',
    measure: wideMeasure,
  });
  assert.equal(locked.plotLeft, 36);
});

test('fitTimeChartFrame and drawTimeChartScene run cleanly on a Canvas2D mock and consult injected measure for dates and end labels', () => {
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
  const padding = resolveTimeChartPadding({
    layoutWidth: 520,
    tokens: baseTokens,
    stripHeight: 0,
    hasBadges: false,
    hasBadgeSummary: false,
    padding: { right: 120 },
  });
  const frame = fitTimeChartFrame({
    layoutWidth: 520,
    layoutHeight: 240,
    padding,
    x,
    y: y.range,
    log: y.log,
    stripHeight: 0,
    hasRows: false,
    hasExplicitLeftPadding: false,
    tokens: baseTokens,
    formatValue: 'powerAxis',
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
      formatValue: 'powerAxis',
      gaps: [{ firstHour: jan1936 + 100, lastHour: jan1936 + 140 }],
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
      endLabels: [{ id: 'GER', text: 'German Reich', suffix: '500', y: frame.toY(500), color: '#4488ff' }],
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
  assert.ok(measuredDuringDraw.includes(' 500'), `end-label suffix measured: ${JSON.stringify(measuredDuringDraw)}`);
  assert.ok(measuredDuringDraw.some(text => text.startsWith('German')), `end-label text measured: ${JSON.stringify(measuredDuringDraw)}`);
});

test('drawAxes invalidates cached row labels when the injected measurer or text width cache changes', () => {
  const { ctx, calls } = mockCanvas2D();
  const frame = plotFrame(300, 120, { left: 40 + VALUE_LABEL_GAP * 2, right: 10, top: 20, bottom: 10 }, [0, 100], [0, 1], false, 0, true);
  const narrow: MeasureText = text => text.length;
  const wide: MeasureText = text => text.length * 10;

  const lastFill = () => calls.filter(call => call[0] === 'fillText').at(-1);

  drawAxes(ctx, frame, baseTokens, String, ['Germany'], narrow);
  assert.equal(lastFill()?.[1], 'Germany');

  drawAxes(ctx, frame, baseTokens, String, ['Germany'], wide);
  assert.equal(lastFill()?.[1], 'Ger…');

  let scale = 1;
  const mutable: MeasureText = text => text.length * scale;
  drawAxes(ctx, frame, baseTokens, String, ['Germany'], mutable);
  assert.equal(lastFill()?.[1], 'Germany');
  scale = 10;
  clearTextWidthCache();
  drawAxes(ctx, frame, baseTokens, String, ['Germany'], mutable);
  assert.equal(lastFill()?.[1], 'Ger…');
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
