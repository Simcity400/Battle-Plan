import assert from 'node:assert/strict';
import test from 'node:test';
import { BADGE_STRIP } from './badges.ts';
import {
  bucketsStream,
  buildSummaryShape,
  drawBucketStream,
  drawLineSummary,
  emitBucketCorners,
  type ChartBucket,
  type ChartSummary,
} from './chartSummary.ts';
import { VALUE_LABEL_GAP } from './draw.ts';
import { plotFrame, STRIP_GAP } from './frame.ts';
import {
  TIME_CHART_DEFAULT_PADDING,
  TIME_CHART_LABEL_MARGIN,
  TIME_CHART_MIN_SPAN,
  chartTokensEqual,
  drawTimeChartScene,
  fitTimeChartFrame,
  resolveAxisFormatter,
  resolveChartFit,
  resolveTimeChartFrame,
  resolveTimeChartPadding,
  resolveTimeChartTargetX,
  resolveTimeChartTargetY,
} from './renderTimeChart.ts';
import type { Canvas2D, MeasureText } from './textWidth.ts';
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
  blueprint: '#223344',
  blueprintBorder: '#334455',
  blueprintText: '#99bbdd',
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

test('resolveTimeChartPadding handles badges, strip height, custom overrides, and row labels', () => {
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

test('declarative formatters, fits, and resolveTimeChartTargetY match chart semantics', () => {
  assert.equal(resolveAxisFormatter('powerAxis')(1500, []), '1.5k');
  assert.equal(resolveAxisFormatter('powerAxis')(420.4, []), '420');
  assert.equal(resolveAxisFormatter('percent')(50, [0, 50, 100]), '50%');
  assert.equal(resolveAxisFormatter()(10, [0, 10]), '10');
  assert.equal(resolveChartFit('none'), undefined);

  const stabilityFit = resolveChartFit('stabilityZoom');
  assert.ok(stabilityFit);
  assert.equal(stabilityFit(null), null);
  assert.deepEqual(stabilityFit([60, 70]), [59, 71]);
  assert.deepEqual(stabilityFit([50, 50]), [49, 51]);

  const target = resolveTimeChartTargetY(
    [{ extent: () => [60, 70] }, { extent: () => [62, 75] }],
    [0, 100],
    EMPTY_VIEW,
    { ...defaultAxes, yMin: 0 },
    'stabilityZoom',
  );
  assert.equal(target.log, false);
  assert.equal(target.range[0], 0);
  assert.ok(target.range[1] >= 77);
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

test('resolveTimeChartFrame and drawTimeChartScene run cleanly on a Canvas2D mock with no DOM', () => {
  const customMeasure: MeasureText = text => text.length * 8;
  const layerCalls: number[] = [];
  let overlayCalled = false;
  const resolved = resolveTimeChartFrame({
    layoutWidth: 480,
    layoutHeight: 240,
    tokens: baseTokens,
    extent: [0, 720],
    playhead: 480,
    playback: 'cursor',
    paused: true,
    view: EMPTY_VIEW,
    axes: defaultAxes,
    branchStart: 0,
    layers: [{ extent: () => [0, 500] }],
    formatValue: 'powerAxis',
    measure: customMeasure,
  });

  const { ctx, calls } = mockCanvas2D();
  drawTimeChartScene(
    ctx,
    resolved.frame,
    baseTokens,
    {
      layoutWidth: 480,
      layoutHeight: 240,
      scaleX: 2,
      scaleY: 2,
      playhead: 480,
      playback: 'cursor',
      formatValue: 'powerAxis',
      gaps: [{ firstHour: 100, lastHour: 140 }],
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
      endLabels: [{ id: 'GER', text: 'GER', suffix: '500', y: resolved.frame.toY(500), color: '#4488ff' }],
    },
    customMeasure,
  );

  assert.deepEqual(calls[0], ['setTransform', 2, 0, 0, 2, 0, 0]);
  assert.deepEqual(calls[1], ['clearRect', 0, 0, 480, 240]);
  assert.deepEqual(layerCalls, [480]);
  assert.equal(overlayCalled, true);
  assert.equal(calls.some(call => call[0] === 'measureText'), false);
  assert.ok(calls.some(call => call[0] === 'fillText' && call[1] === 'GER 500'));
});

test('emitBucketCorners preserves tie order and reuses caller scratch buffer', () => {
  const scratch = emitBucketCorners(0, 0, 0, 0, 0, 0, 0, 0);
  const reused = emitBucketCorners(10, 10, 10, 10, 1, 2, 3, 4, scratch);
  assert.equal(reused, scratch);
  assert.deepEqual(
    reused.map(c => [c.hour, c.value, c.order]),
    [
      [10, 1, 0],
      [10, 2, 1],
      [10, 3, 2],
      [10, 4, 3],
    ],
  );
  const ordered = emitBucketCorners(10, 30, 20, 40, 5, -2, 9, 4, scratch);
  assert.deepEqual(
    ordered.map(c => [c.hour, c.value]),
    [
      [10, 5],
      [20, 9],
      [30, -2],
      [40, 4],
    ],
  );
});

test('drawBucketStream paints the exact same stream as drawLineSummary across joins, breaks, gaps, and log fallback', () => {
  const buckets: (ChartBucket | null)[] = [
    {
      firstHour: 0,
      lastHour: 9,
      firstValue: 2,
      lastValue: 5,
      lastValueHour: 8,
      minHour: 2,
      minValue: 1,
      maxHour: 6,
      maxValue: 9,
      firstBreak: false,
      hasBreak: false,
    },
    {
      firstHour: 10,
      lastHour: 19,
      firstValue: 7,
      lastValue: 4,
      lastValueHour: 18,
      minHour: 15,
      minValue: 3,
      maxHour: 12,
      maxValue: 8,
      firstBreak: false,
      hasBreak: false,
    },
    {
      firstHour: 20,
      lastHour: 29,
      firstValue: 10,
      lastValue: 11,
      lastValueHour: 29,
      minHour: 20,
      minValue: 10,
      maxHour: 29,
      maxValue: 11,
      firstBreak: true,
      hasBreak: false,
    },
    null,
    {
      firstHour: 40,
      lastHour: 49,
      firstValue: 6,
      lastValue: 2,
      lastValueHour: 48,
      minHour: 45,
      minValue: 1,
      maxHour: 42,
      maxValue: 7,
      firstBreak: false,
      hasBreak: true,
    },
    {
      firstHour: 50,
      lastHour: 59,
      firstValue: 3,
      lastValue: 6,
      lastValueHour: 55,
      minHour: 50,
      minValue: 3,
      maxHour: 55,
      maxValue: 6,
      firstBreak: false,
      hasBreak: false,
    },
  ];
  const summary: ChartSummary = { from: 0, to: 60, width: 100, buckets };
  const frame = plotFrame(120, 100, { left: 10, right: 10, top: 10, bottom: 10 }, [0, 60], [0, 12], false);

  for (const held of [false, true]) {
    for (const options of [{}, { area: true }, { dash: [4, 2], endDot: false }, { area: true, dash: [6, 3] }]) {
      const legacy = mockCanvas2D();
      const streamed = mockCanvas2D();
      const expectedEnd = drawLineSummary(legacy.ctx, frame, baseTokens, summary, '#4488ff', held, null, options);
      const actualEnd = drawBucketStream(
        streamed.ctx,
        frame,
        baseTokens,
        bucketsStream(buckets),
        '#4488ff',
        held,
        summary.to,
        options,
      );
      assert.deepEqual(actualEnd, expectedEnd, `end held=${held} opts=${JSON.stringify(options)}`);
      assert.deepEqual(streamed.calls, legacy.calls, `calls held=${held} opts=${JSON.stringify(options)}`);
    }
  }

  const shape = buildSummaryShape(bucketsStream(buckets), true, summary.to);
  assert.equal(shape.low, 1);
  assert.equal(shape.high, 11);
  assert.deepEqual(shape.end, [60, 6]);
  assert.ok(shape.dots.length === 8);

  const logFrame = plotFrame(120, 100, { left: 10, right: 10, top: 10, bottom: 10 }, [0, 29], [1e-323, 1], true);
  const logBuckets: ChartBucket[] = [
    {
      firstHour: 0,
      lastHour: 9,
      firstValue: 0.5,
      lastValue: 0.5,
      lastValueHour: 0,
      minHour: 2,
      minValue: 0.25,
      maxHour: 0,
      maxValue: 0.5,
      firstBreak: false,
      hasBreak: false,
    },
    {
      firstHour: 10,
      lastHour: 19,
      firstValue: 0.5,
      lastValue: 0.5,
      lastValueHour: 10,
      minHour: 12,
      minValue: 0,
      maxHour: 10,
      maxValue: 0.5,
      firstBreak: false,
      hasBreak: false,
    },
  ];
  const logSummary: ChartSummary = { from: 0, to: 29, width: 100, buckets: logBuckets };
  const logLegacy = mockCanvas2D();
  const logStream = mockCanvas2D();
  assert.deepEqual(
    drawBucketStream(logStream.ctx, logFrame, baseTokens, bucketsStream(logBuckets), '#123', false, 29),
    drawLineSummary(logLegacy.ctx, logFrame, baseTokens, logSummary, '#123', false),
  );
  assert.deepEqual(logStream.calls, logLegacy.calls);
});

test('chartTokensEqual compares every token field by value', () => {
  const clone: ChartTokens = {
    ...baseTokens,
    fontSize: { ...baseTokens.fontSize },
    lineWidth: { ...baseTokens.lineWidth },
    categories: [...baseTokens.categories],
  };
  assert.equal(chartTokensEqual(baseTokens, clone), true);
  assert.equal(chartTokensEqual(null, null), true);
  assert.equal(chartTokensEqual(baseTokens, null), false);
  assert.equal(chartTokensEqual(baseTokens, { ...clone, accent: '#ff0000' }), false);
  assert.equal(chartTokensEqual(baseTokens, { ...clone, fontSize: { ...clone.fontSize, small: 12 } }), false);
  assert.equal(chartTokensEqual(baseTokens, { ...clone, lineWidth: { ...clone.lineWidth, series: 2.5 } }), false);
  assert.equal(chartTokensEqual(baseTokens, { ...clone, categories: ['#112233', '#445566'] }), false);
  assert.equal(chartTokensEqual(baseTokens, { ...clone, categories: ['#112233', '#445566', '#000000'] }), false);
  assert.equal(chartTokensEqual(baseTokens, { ...clone, reducedMotion: true }), false);
});
