import assert from 'node:assert/strict';
import test from 'node:test';
import { firstHourOfMonth } from '../dashboard/calendar.ts';
import { BADGE_STRIP } from './badges.ts';
import {
  bucketsStream,
  buildSummaryShape,
  drawBucketStream,
  drawLineSummary,
  drawSummaryShape,
  emitBucketCorners,
  type BucketStreamReader,
  type ChartBucket,
  type ChartSummary,
} from './chartSummary.ts';
import { VALUE_LABEL_GAP, drawAxes } from './draw.ts';
import { plotFrame, STRIP_GAP } from './frame.ts';
import {
  TIME_CHART_DEFAULT_PADDING,
  TIME_CHART_LABEL_MARGIN,
  TIME_CHART_MIN_SPAN,
  chartTokensEqual,
  drawTimeChartScene,
  fitTimeChartFrame,
  normalizeStripHeight,
  resolveAxisFormatter,
  resolveChartFit,
  resolveTimeChartFrame,
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

  const fractionalStrip = resolveTimeChartFrame({
    layoutWidth: 400,
    layoutHeight: 202,
    tokens: baseTokens,
    extent: [0, 720],
    playhead: 480,
    playback: 'cursor',
    paused: true,
    view: EMPTY_VIEW,
    axes: defaultAxes,
    branchStart: 0,
    layers: [{ extent: () => [0, 100] }],
    stripHeight: 48.48,
    measure: text => text.length * 7,
  });
  const roundedStrip = resolveTimeChartFrame({
    layoutWidth: 400,
    layoutHeight: 202,
    tokens: baseTokens,
    extent: [0, 720],
    playhead: 480,
    playback: 'cursor',
    paused: true,
    view: EMPTY_VIEW,
    axes: defaultAxes,
    branchStart: 0,
    layers: [{ extent: () => [0, 100] }],
    stripHeight: 48,
    measure: text => text.length * 7,
  });
  assert.equal(normalizeStripHeight(48.48), 48);
  assert.equal(fractionalStrip.frame.plotHeight, roundedStrip.frame.plotHeight);
  assert.deepEqual(fractionalStrip.frame.strip, roundedStrip.frame.strip);

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

test('resolveTimeChartFrame and drawTimeChartScene run cleanly on a Canvas2D mock and consult injected measure for dates and end labels', () => {
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
  const resolved = resolveTimeChartFrame({
    layoutWidth: 520,
    layoutHeight: 240,
    tokens: baseTokens,
    extent: [jan1936, end1936],
    playhead: mid1936,
    playback: 'cursor',
    paused: true,
    view: EMPTY_VIEW,
    axes: defaultAxes,
    branchStart: jan1936,
    padding: { right: 120 },
    layers: [{ extent: () => [0, 500] }],
    formatValue: 'powerAxis',
    measure: customMeasure,
  });

  const { ctx, calls } = mockCanvas2D();
  recording = true;
  drawTimeChartScene(
    ctx,
    resolved.frame,
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
      endLabels: [{ id: 'GER', text: 'German Reich', suffix: '500', y: resolved.frame.toY(500), color: '#4488ff' }],
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

test('bucketsStream treats sparse undefined holes as gaps separating subpaths', () => {
  const sparse = new Array<ChartBucket | null>(3);
  sparse[0] = {
    firstHour: 0,
    lastHour: 9,
    firstValue: 2,
    lastValue: 2,
    lastValueHour: 0,
    minHour: 0,
    minValue: 2,
    maxHour: 0,
    maxValue: 2,
    firstBreak: false,
    hasBreak: false,
  };
  sparse[2] = {
    firstHour: 20,
    lastHour: 29,
    firstValue: 8,
    lastValue: 8,
    lastValueHour: 20,
    minHour: 20,
    minValue: 8,
    maxHour: 20,
    maxValue: 8,
    firstBreak: false,
    hasBreak: false,
  };
  const shape = buildSummaryShape(bucketsStream(sparse), false, 30);
  assert.equal(shape.starts.length, 3);
  assert.deepEqual(Array.from(shape.points), [0, 2, 9, 2, 20, 8, 29, 8]);
  assert.deepEqual(Array.from(shape.starts), [0, 2, 4]);
});

test('buildSummaryShape, drawSummaryShape, and drawBucketStream preserve exact joins, breaks, range clip, and zero-walk reuse', () => {
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

  const unheldShape = buildSummaryShape(bucketsStream(buckets), false, summary.to);
  assert.equal(unheldShape.starts.length, 4);
  const subpathPoints = (shape: ReturnType<typeof buildSummaryShape>, idx: number) => {
    const out: [number, number][] = [];
    for (let v = shape.starts[idx]; v < shape.starts[idx + 1]; v++) {
      out.push([shape.points[2 * v], shape.points[2 * v + 1]]);
    }
    return out;
  };
  assert.deepEqual(subpathPoints(unheldShape, 0), [
    [0, 2],
    [2, 1],
    [6, 9],
    [8, 5],
    [10, 5],
    [10, 7],
    [12, 8],
    [15, 3],
    [18, 4],
    [19, 4],
  ]);
  assert.deepEqual(subpathPoints(unheldShape, 1), [
    [20, 10],
    [29, 11],
  ]);
  assert.deepEqual(subpathPoints(unheldShape, 2), [
    [50, 3],
    [55, 6],
    [59, 6],
  ]);

  const heldShape = buildSummaryShape(bucketsStream(buckets), true, summary.to);
  assert.equal(heldShape.starts.length, 4);
  assert.ok(
    subpathPoints(heldShape, 0).some(
      (pt, i, arr) => pt[0] === 10 && pt[1] === 5 && arr[i + 1]?.[0] === 10 && arr[i + 1]?.[1] === 7,
    ),
  );
  assert.deepEqual(subpathPoints(heldShape, 1)[0], [20, 10]);
  assert.equal(heldShape.low, 1);
  assert.equal(heldShape.high, 11);
  assert.deepEqual(heldShape.end, [60, 6]);
  assert.equal(heldShape.dots.length, 8);

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

  let readerTouched = false;
  const untouchedReader: BucketStreamReader = {
    get count() {
      readerTouched = true;
      return 0;
    },
    present: () => {
      readerTouched = true;
      return false;
    },
    firstHour: () => 0,
    lastHour: () => 0,
    firstValue: () => 0,
    lastValue: () => 0,
    lastValueHour: () => 0,
    minHour: () => 0,
    minValue: () => 0,
    maxHour: () => 0,
    maxValue: () => 0,
    firstBreak: () => false,
    hasBreak: () => false,
  };
  const cachedPaint = mockCanvas2D();
  const freshPaint = mockCanvas2D();
  drawSummaryShape(cachedPaint.ctx, frame, baseTokens, unheldShape, '#4488ff', { area: true });
  drawBucketStream(freshPaint.ctx, frame, baseTokens, bucketsStream(buckets), '#4488ff', false, summary.to, { area: true });
  assert.deepEqual(cachedPaint.calls, freshPaint.calls);
  drawBucketStream(mockCanvas2D().ctx, frame, baseTokens, untouchedReader, '#4488ff', false, summary.to, {}, undefined, unheldShape);
  assert.equal(readerTouched, false);

  const rangeFrame = plotFrame(120, 100, { left: 10, right: 10, top: 10, bottom: 10 }, [0, 47], [0, 20], false);
  const rangeBuckets: ChartBucket[] = [
    {
      firstHour: 0,
      lastHour: 23,
      firstValue: 5,
      lastValue: 5,
      lastValueHour: 0,
      minHour: 0,
      minValue: 5,
      maxHour: 0,
      maxValue: 5,
      firstBreak: false,
      hasBreak: false,
    },
    {
      firstHour: 24,
      lastHour: 47,
      firstValue: 5,
      lastValue: 18,
      lastValueHour: 40,
      minHour: 24,
      minValue: 5,
      maxHour: 40,
      maxValue: 18,
      firstBreak: false,
      hasBreak: false,
    },
  ];
  const ranged = mockCanvas2D();
  const rangedEnd = drawBucketStream(ranged.ctx, rangeFrame, baseTokens, bucketsStream(rangeBuckets), '#4488ff', false, 47, {}, [0, 24]);
  assert.equal(rangedEnd, null);
  const clipRect = ranged.calls.find(call => call[0] === 'rect');
  assert.deepEqual(clipRect, ['rect', rangeFrame.plotLeft, rangeFrame.plotTop, rangeFrame.toX(24) - rangeFrame.plotLeft, rangeFrame.plotHeight]);
  assert.ok(ranged.calls.some(call => call[0] === 'clip'));
  assert.equal(ranged.calls.some(call => call[0] === 'arc'), false);

  const logFrame = plotFrame(120, 100, { left: 10, right: 10, top: 10, bottom: 10 }, [0, 47], [1e-323, 1], true);
  const logBuckets: ChartBucket[] = [
    {
      firstHour: 0,
      lastHour: 23,
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
      firstHour: 24,
      lastHour: 47,
      firstValue: 0.5,
      lastValue: 0.5,
      lastValueHour: 24,
      minHour: 30,
      minValue: 0,
      maxHour: 24,
      maxValue: 0.5,
      firstBreak: false,
      hasBreak: false,
    },
  ];
  const logSummary: ChartSummary = { from: 0, to: 47, width: 100, buckets: logBuckets };
  const logLegacy = mockCanvas2D();
  const logStream = mockCanvas2D();
  assert.deepEqual(
    drawBucketStream(logStream.ctx, logFrame, baseTokens, bucketsStream(logBuckets), '#123', false, 47),
    drawLineSummary(logLegacy.ctx, logFrame, baseTokens, logSummary, '#123', false),
  );
  assert.deepEqual(logStream.calls, logLegacy.calls);

  const logRanged = mockCanvas2D();
  const logRangedEnd = drawBucketStream(logRanged.ctx, logFrame, baseTokens, bucketsStream(logBuckets), '#123', false, 47, {}, [0, 24]);
  assert.equal(logRangedEnd, null);
  assert.deepEqual(
    logRanged.calls.find(call => call[0] === 'rect'),
    ['rect', logFrame.plotLeft, logFrame.plotTop, logFrame.toX(24) - logFrame.plotLeft, logFrame.plotHeight],
  );
  assert.equal(
    logRanged.calls.some(call => call[0] === 'arc' && call[3] === baseTokens.lineWidth.series * 2),
    false,
  );
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
