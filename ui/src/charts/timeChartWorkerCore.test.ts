import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChartSummary } from './chartSummary.ts';
import type { Canvas2D } from './textWidth.ts';
import type { ChartTokens } from './tokens.ts';
import {
  createTimeChartWorkerHandler,
  packChartSummary,
  type DrawMessage,
  type WorkerCanvasLike,
} from './timeChartWorkerCore.ts';

const TOKENS: ChartTokens = {
  surface: '#111111',
  raised: '#1a1a1a',
  text: '#eeeeee',
  muted: '#888888',
  border: '#333333',
  grid: '#222222',
  axis: '#444444',
  accent: '#4da3ff',
  positive: '#44cc77',
  warning: '#eeaa33',
  error: '#ee4444',
  blueprint: '#223344',
  blueprintBorder: '#335577',
  blueprintText: '#cceeff',
  font: 'sans-serif',
  fontSize: { small: 11, body: 13, title: 15 },
  lineWidth: { grid: 1, series: 2, emphasis: 3 },
  hatchSpacing: 6,
  gapOpacity: 0.2,
  motion: 'snappy',
  reducedMotion: true,
  countryColours: 'raw',
  categories: ['#4da3ff', '#ee6644', '#44cc77'],
};

interface RecordedCall {
  readonly op: string;
  readonly args: readonly unknown[];
  readonly width: number;
  readonly height: number;
}

function makeFakeCanvas(initialWidth = 400, initialHeight = 200, options?: { nullContext?: boolean; throwOnClear?: string }) {
  const calls: RecordedCall[] = [];
  const canvas: WorkerCanvasLike = {
    width: initialWidth,
    height: initialHeight,
    getContext() {
      if (options?.nullContext) return null;
      return ctx;
    },
  };
  const record = (op: string) => (...args: unknown[]) => {
    calls.push({ op, args, width: canvas.width, height: canvas.height });
  };
  const ctx: Canvas2D = {
    fillStyle: '',
    strokeStyle: '',
    font: '',
    lineWidth: 1,
    lineJoin: 'miter',
    lineCap: 'butt',
    globalAlpha: 1,
    textAlign: 'start',
    textBaseline: 'alphabetic',
    setTransform: record('setTransform'),
    clearRect: (...args: unknown[]) => {
      if (options?.throwOnClear) throw new Error(options.throwOnClear);
      calls.push({ op: 'clearRect', args, width: canvas.width, height: canvas.height });
    },
    save: record('save'),
    restore: record('restore'),
    beginPath: record('beginPath'),
    closePath: record('closePath'),
    rect: record('rect'),
    clip: record('clip'),
    moveTo: record('moveTo'),
    lineTo: record('lineTo'),
    stroke: record('stroke'),
    fill: record('fill'),
    fillRect: record('fillRect'),
    strokeRect: record('strokeRect'),
    arc: record('arc'),
    fillText: record('fillText'),
    setLineDash: record('setLineDash'),
    measureText: (text: string) => ({ width: text.length * 7 }),
  } as unknown as Canvas2D;
  return { canvas, calls };
}

function constantSummary(value: number, from = 0, to = 100, width = 342): ChartSummary {
  return {
    from,
    to,
    width,
    buckets: [
      {
        firstHour: from,
        lastHour: to,
        firstValue: value,
        lastValue: value,
        lastValueHour: to,
        minHour: from,
        minValue: value,
        maxHour: from,
        maxValue: value,
        firstBreak: false,
        hasBreak: false,
      },
    ],
  };
}

function makeDraw(id: number, overrides: Partial<DrawMessage> = {}): DrawMessage {
  return {
    type: 'draw',
    id,
    backingWidth: 400,
    backingHeight: 200,
    layoutWidth: 400,
    layoutHeight: 200,
    scaleX: 1,
    scaleY: 1,
    frame: {
      width: 400,
      height: 200,
      padding: { left: 44, right: 14, top: 12, bottom: 24 },
      x: [0, 100],
      y: [0, 100],
      log: false,
      strip: 0,
      timeAxisTop: false,
    },
    layers: [
      {
        kind: 'line-summary',
        summaryRev: 1,
        color: '#ff0000',
        held: false,
      },
    ],
    headValues: [null],
    playhead: 100,
    playback: 'draw-in',
    until: 100,
    formatValue: 'powerAxis',
    ...overrides,
  };
}

test('newest draw wins when multiple draws arrive before drain', () => {
  const scheduled: (() => void)[] = [];
  const handler = createTimeChartWorkerHandler({ schedule: fn => scheduled.push(fn) });
  const { canvas, calls } = makeFakeCanvas();
  handler.handle({ type: 'tokens', tokens: TOKENS });
  handler.handle({ type: 'attach', id: 1, canvas });
  const rev1 = packChartSummary(1, constantSummary(25)).packed;
  const rev2 = packChartSummary(2, constantSummary(75)).packed;
  handler.handle({ type: 'data', id: 1, summaries: [rev1, rev2] });

  handler.handle(makeDraw(1, { layers: [{ kind: 'line-summary', summaryRev: 1, color: '#ff0000', held: false }] }));
  handler.handle(makeDraw(1, { layers: [{ kind: 'line-summary', summaryRev: 1, color: '#00ff00', held: false }] }));
  handler.handle(makeDraw(1, { layers: [{ kind: 'line-summary', summaryRev: 2, color: '#0000ff', held: false }] }));

  assert.equal(scheduled.length, 1);
  scheduled.shift()!();

  const clears = calls.filter(c => c.op === 'clearRect');
  assert.equal(clears.length, 1);
  const moves = calls.filter(c => c.op === 'moveTo').map(c => Number(c.args[1]));
  const expectedYFor75 = 12 + (1 - 0.75) * (200 - 12 - 24);
  const expectedYFor25 = 12 + (1 - 0.25) * (200 - 12 - 24);
  assert.ok(moves.some(y => Math.abs(y - expectedYFor75) < 0.5));
  assert.ok(!moves.some(y => Math.abs(y - expectedYFor25) < 0.5));
});

test('frame referencing a summary revision draws that revision and prunes stale revisions', () => {
  const scheduled: (() => void)[] = [];
  const handler = createTimeChartWorkerHandler({ schedule: fn => scheduled.push(fn) });
  const { canvas, calls } = makeFakeCanvas();
  handler.handle({ type: 'tokens', tokens: TOKENS });
  handler.handle({ type: 'attach', id: 1, canvas });
  handler.handle({ type: 'data', id: 1, summaries: [packChartSummary(1, constantSummary(20)).packed] });
  handler.handle(makeDraw(1, { layers: [{ kind: 'line-summary', summaryRev: 1, color: '#ff0000', held: false }] }));
  scheduled.shift()!();

  const y20 = 12 + (1 - 0.2) * (200 - 12 - 24);
  const y80 = 12 + (1 - 0.8) * (200 - 12 - 24);
  assert.ok(calls.some(c => c.op === 'moveTo' && Math.abs(Number(c.args[1]) - y20) < 0.5));

  calls.length = 0;
  handler.handle({
    type: 'data',
    id: 1,
    summaries: [packChartSummary(2, constantSummary(80)).packed],
    keepSummaries: [2],
  });
  handler.handle(makeDraw(1, { layers: [{ kind: 'line-summary', summaryRev: 2, color: '#ff0000', held: false }] }));
  scheduled.shift()!();
  assert.ok(calls.some(c => c.op === 'moveTo' && Math.abs(Number(c.args[1]) - y80) < 0.5));

  calls.length = 0;
  handler.handle(makeDraw(1, { layers: [{ kind: 'line-summary', summaryRev: 1, color: '#ff0000', held: false }] }));
  scheduled.shift()!();
  assert.ok(!calls.some(c => c.op === 'moveTo' && Math.abs(Number(c.args[1]) - y20) < 0.5));
});

test('detach drops the chart and ignores late draws', () => {
  const scheduled: (() => void)[] = [];
  const handler = createTimeChartWorkerHandler({ schedule: fn => scheduled.push(fn) });
  const { canvas, calls } = makeFakeCanvas();
  handler.handle({ type: 'tokens', tokens: TOKENS });
  handler.handle({ type: 'attach', id: 7, canvas });
  assert.equal(handler.attachedCount, 1);

  handler.handle({ type: 'detach', id: 7 });
  assert.equal(handler.attachedCount, 0);

  handler.handle({ type: 'data', id: 7, summaries: [packChartSummary(1, constantSummary(50)).packed] });
  handler.handle(makeDraw(7));
  while (scheduled.length) scheduled.shift()!();

  assert.equal(calls.length, 0);
});

test('resize applies to canvas backing store before the next draw paints', () => {
  const scheduled: (() => void)[] = [];
  const handler = createTimeChartWorkerHandler({ schedule: fn => scheduled.push(fn) });
  const { canvas, calls } = makeFakeCanvas(100, 50);
  handler.handle({ type: 'tokens', tokens: TOKENS });
  handler.handle({ type: 'attach', id: 2, canvas });
  handler.handle({ type: 'data', id: 2, summaries: [packChartSummary(1, constantSummary(50, 0, 100, 542)).packed] });
  handler.handle(makeDraw(2, {
    backingWidth: 1200,
    backingHeight: 600,
    layoutWidth: 600,
    layoutHeight: 300,
    scaleX: 2,
    scaleY: 2,
    frame: {
      width: 600,
      height: 300,
      padding: { left: 44, right: 14, top: 12, bottom: 24 },
      x: [0, 100],
      y: [0, 100],
      log: false,
      strip: 0,
      timeAxisTop: false,
    },
  }));
  scheduled.shift()!();

  assert.equal(canvas.width, 1200);
  assert.equal(canvas.height, 600);
  assert.ok(calls.length > 0);
  assert.equal(calls[0]!.width, 1200);
  assert.equal(calls[0]!.height, 600);
});

test('worker reports context acquisition and draw failures through onError', () => {
  const errors: string[] = [];
  const scheduled: (() => void)[] = [];
  const handler = createTimeChartWorkerHandler({
    schedule: fn => scheduled.push(fn),
    onError: reason => errors.push(reason),
  });
  handler.handle({ type: 'tokens', tokens: TOKENS });

  const broken = makeFakeCanvas(100, 100, { nullContext: true });
  handler.handle({ type: 'attach', id: 10, canvas: broken.canvas });
  assert.equal(handler.attachedCount, 0);
  assert.equal(errors.length, 1);

  const throwing = makeFakeCanvas(400, 200, { throwOnClear: 'context lost' });
  handler.handle({ type: 'attach', id: 11, canvas: throwing.canvas });
  handler.handle(makeDraw(11));
  scheduled.shift()!();
  assert.equal(errors.length, 2);
  assert.match(errors[1]!, /context lost/);
});
