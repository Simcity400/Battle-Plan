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
  readonly strokeStyle?: string;
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
    calls.push({ op, args, width: canvas.width, height: canvas.height, strokeStyle: String(ctx.strokeStyle) });
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
  handler.handle(makeDraw(1, { layers: [{ kind: 'line-summary', summaryRev: 1, color: '#ff0000', held: false }] }));
  handler.handle({
    type: 'data',
    id: 1,
    summaries: [packChartSummary(2, constantSummary(80)).packed],
    keepSummaries: [2],
  });
  scheduled.shift()!();
  assert.ok(calls.some(c => c.op === 'moveTo' && Math.abs(Number(c.args[1]) - y20) < 0.5));

  calls.length = 0;
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

test('worker client survives StrictMode effect replay, catches post failures, and strips non-cloneable badge errors', async () => {
  const { plotFrame } = await import('./frame.ts');
  const { lineSummaryLayer } = await import('./renderTimeChart.ts');
  const { TimeChartWorkerPool } = await import('./timeChartWorkerClient.ts') as unknown as {
    TimeChartWorkerPool?: new () => {
      attach(el: HTMLCanvasElement, onFallback: (reason: string) => void): {
        id: number;
        draw(scene: unknown): boolean;
        detach(): void;
      } | null;
    };
  };
  assert.ok(TimeChartWorkerPool, 'TimeChartWorkerPool should be constructible for isolated client tests');
  const posted: unknown[] = [];
  let failNextPost: Error | null = null;
  class FakeWorker {
    onmessage: ((event: MessageEvent) => void) | null = null;
    onerror: ((event: ErrorEvent) => void) | null = null;
    onmessageerror: (() => void) | null = null;
    postMessage(msg: unknown) {
      if (failNextPost) {
        const err = failNextPost;
        failNextPost = null;
        throw err;
      }
      if ((msg as { type?: string }).type !== 'attach') structuredClone(msg);
      posted.push(msg);
    }
    terminate() {}
  }
  class FakeOffscreenCanvas {}
  class FakeHTMLCanvasElement {
    transferControlToOffscreen() { return new FakeOffscreenCanvas(); }
  }
  const makeFakeDom = (fontFamily?: string) => ({
    document: {
      documentElement: { dataset: {} },
      body: { append() {} },
      createElement: () => ({ style: { display: '', color: '' }, remove() {} }),
      styleSheets: fontFamily ? [{
        cssRules: [{
          type: 5,
          style: {
            getPropertyValue: (prop: string) => prop === 'font-family' ? `"${fontFamily}"` : prop === 'src' ? 'url("https://example.invalid/test.woff2")' : '',
          },
        }],
      }] : [],
    },
    getComputedStyle: () => ({
      color: 'rgb(20, 24, 30)',
      getPropertyValue: (prop: string) => prop === '--font-ui' ? (fontFamily ?? 'sans-serif') : '',
    }),
    window: { addEventListener() {}, removeEventListener() {} },
  });
  const fakeDom = makeFakeDom('TestSans');
  fakeDom.document.styleSheets = [];
  const saved = new Map(['Worker', 'OffscreenCanvas', 'HTMLCanvasElement', 'document', 'getComputedStyle', 'window']
    .map(k => [k, Object.getOwnPropertyDescriptor(globalThis, k)]));
  Object.defineProperty(globalThis, 'Worker', { configurable: true, value: FakeWorker });
  Object.defineProperty(globalThis, 'OffscreenCanvas', { configurable: true, value: FakeOffscreenCanvas });
  Object.defineProperty(globalThis, 'HTMLCanvasElement', { configurable: true, value: FakeHTMLCanvasElement });
  Object.defineProperty(globalThis, 'document', { configurable: true, value: fakeDom.document });
  Object.defineProperty(globalThis, 'getComputedStyle', { configurable: true, value: fakeDom.getComputedStyle });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: fakeDom.window });
  try {
    const pool = new TimeChartWorkerPool!();
    let transfers = 0;
    const makeCanvas = () => ({
      transferControlToOffscreen() {
        if (transfers > 0) throw new DOMException('Cannot transfer control from a canvas for more than one time.', 'InvalidStateError');
        transfers++;
        return new FakeOffscreenCanvas();
      },
      closest: () => null,
    }) as unknown as HTMLCanvasElement;

    const el = makeCanvas();
    const first = pool.attach(el, () => {});
    assert.ok(first);
    first!.detach();
    const second = pool.attach(el, () => {});
    assert.ok(second);
    assert.equal(transfers, 1);
    await Promise.resolve();
    assert.equal(posted.filter(m => (m as { type?: string }).type === 'detach').length, 0);

    const frame = plotFrame(400, 200, { left: 44, right: 14, top: 12, bottom: 24 }, [0, 100], [0, 100], false, 0, false);
    const badError = Object.assign(new Error('failed fact'), { cause: () => 'uncloneable' });
    const okWithBadge = second!.draw({
      backingWidth: 400, backingHeight: 200, layoutWidth: 400, layoutHeight: 200, scaleX: 1, scaleY: 1, frame,
      layers: [lineSummaryLayer({ summary: () => constantSummary(50), color: '#ff0000' })],
      playhead: 100, playback: 'draw-in', until: 100,
      badges: [{ hour: 50, glyph: '!', label: 'event', failure: badError }],
    });
    assert.equal(okWithBadge, true);

    failNextPost = new DOMException('could not clone', 'DataCloneError');
    const failedDraw = second!.draw({
      backingWidth: 400, backingHeight: 200, layoutWidth: 400, layoutHeight: 200, scaleX: 1, scaleY: 1, frame,
      layers: [lineSummaryLayer({ summary: () => constantSummary(60), color: '#ff0000' })],
      playhead: 100, playback: 'draw-in', until: 100,
    });
    assert.equal(failedDraw, false);

    let transfers2 = 0;
    const el2 = {
      transferControlToOffscreen() { transfers2++; return new FakeOffscreenCanvas(); },
      closest: () => null,
    } as unknown as HTMLCanvasElement;
    failNextPost = new DOMException('attach post failed', 'DataCloneError');
    const failedAttach = pool.attach(el2, () => {});
    assert.equal(failedAttach, null);
    assert.equal(transfers2, 1);
  } finally {
    for (const [k, desc] of saved) {
      if (desc) Object.defineProperty(globalThis, k, desc); else Reflect.deleteProperty(globalThis, k);
    }
  }
});

test('worker client retries font loading once and falls back to synchronous drawing on repeated failure', async () => {
  const { TimeChartWorkerPool } = await import('./timeChartWorkerClient.ts');
  const posted: unknown[] = [];
  let workerInstance: { onmessage: ((event: MessageEvent) => void) | null } | null = null;
  class FakeWorker {
    onmessage: ((event: MessageEvent) => void) | null = null;
    onerror: ((event: ErrorEvent) => void) | null = null;
    onmessageerror: (() => void) | null = null;
    constructor() { workerInstance = this; }
    postMessage(msg: unknown) { posted.push(msg); }
    terminate() {}
  }
  class FakeOffscreenCanvas {}
  class FakeHTMLCanvasElement {
    transferControlToOffscreen() { return new FakeOffscreenCanvas(); }
  }
  const fakeDoc = {
    documentElement: { dataset: {} },
    body: { append() {} },
    createElement: () => ({ style: { display: '', color: '' }, remove() {} }),
    styleSheets: [{
      cssRules: [{
        type: 5,
        style: {
          getPropertyValue: (prop: string) => prop === 'font-family' ? '"TestSans"' : prop === 'src' ? 'url("https://example.invalid/test.woff2")' : '',
        },
      }],
    }],
  };
  const listeners = new Map<string, Set<() => void>>();
  const fakeWin = {
    addEventListener(type: string, fn: () => void) {
      let set = listeners.get(type);
      if (!set) { set = new Set(); listeners.set(type, set); }
      set.add(fn);
    },
    removeEventListener(type: string, fn: () => void) { listeners.get(type)?.delete(fn); },
  };
  const saved = new Map(['Worker', 'OffscreenCanvas', 'HTMLCanvasElement', 'document', 'getComputedStyle', 'window', 'CSSRule']
    .map(k => [k, Object.getOwnPropertyDescriptor(globalThis, k)]));
  Object.defineProperty(globalThis, 'Worker', { configurable: true, value: FakeWorker });
  Object.defineProperty(globalThis, 'OffscreenCanvas', { configurable: true, value: FakeOffscreenCanvas });
  Object.defineProperty(globalThis, 'HTMLCanvasElement', { configurable: true, value: FakeHTMLCanvasElement });
  Object.defineProperty(globalThis, 'CSSRule', { configurable: true, value: { FONT_FACE_RULE: 5 } });
  Object.defineProperty(globalThis, 'document', { configurable: true, value: fakeDoc });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: fakeWin });
  Object.defineProperty(globalThis, 'getComputedStyle', {
    configurable: true,
    value: () => ({
      color: 'rgb(20, 24, 30)',
      getPropertyValue: (prop: string) => prop === '--font-ui' ? 'TestSans' : '',
    }),
  });
  try {
    const pool = new TimeChartWorkerPool();
    const fallbacks: string[] = [];
    const el = {
      transferControlToOffscreen: () => new FakeOffscreenCanvas(),
      closest: () => null,
    } as unknown as HTMLCanvasElement;
    const handle = pool.attach(el, reason => fallbacks.push(reason));
    assert.ok(handle);
    const fontMessages = () => posted.filter(m => (m as { type?: string }).type === 'fonts');
    assert.equal(fontMessages().length, 1);
    workerInstance!.onmessage!({ data: { type: 'fontsResult', fontKey: 'TestSans', ok: false, failed: ['TestSans'] } } as MessageEvent);
    assert.equal(fontMessages().length, 2);
    assert.equal(fallbacks.length, 0);
    workerInstance!.onmessage!({ data: { type: 'fontsResult', fontKey: 'TestSans', ok: false, failed: ['TestSans'] } } as MessageEvent);
    assert.equal(fallbacks.length, 1);
    assert.match(fallbacks[0]!, /TestSans/);
  } finally {
    for (const [k, desc] of saved) {
      if (desc) Object.defineProperty(globalThis, k, desc); else Reflect.deleteProperty(globalThis, k);
    }
  }
});

test('worker prunes superseded revisions even when a later draw adds a revision without a new keep set', () => {
  const scheduled: (() => void)[] = [];
  const handler = createTimeChartWorkerHandler({ schedule: fn => scheduled.push(fn) });
  handler.handle({ type: 'tokens', tokens: TOKENS });
  const { canvas, calls } = makeFakeCanvas(400, 200);
  handler.handle({ type: 'attach', id: 30, canvas });

  const makeLine = (rev: number, color: string) => ({ kind: 'line-summary' as const, summaryRev: rev, color, held: false });
  handler.handle(makeDraw(30, {
    summaries: [packChartSummary(1, constantSummary(25)).packed],
    layers: [makeLine(1, '#111111')],
    headValues: [null],
  }));
  scheduled.shift()!();
  assert.ok(calls.some(c => c.op === 'stroke' && c.strokeStyle === '#111111'));

  for (let step = 0; step < 5; step++) {
    const revB = 10 + step * 2;
    const revC = revB + 1;
    handler.handle(makeDraw(30, {
      summaries: [packChartSummary(revB, constantSummary(40)).packed],
      keepSummaries: [revB],
      layers: [makeLine(revB, '#222222')],
      headValues: [null],
    }));
    handler.handle(makeDraw(30, {
      summaries: [packChartSummary(revC, constantSummary(60)).packed],
      layers: [makeLine(revB, '#222222'), makeLine(revC, '#333333')],
      headValues: [null, null],
    }));
    scheduled.shift()!();
  }

  calls.length = 0;
  handler.handle(makeDraw(30, {
    layers: [makeLine(1, '#aa0000')],
    headValues: [null],
  }));
  scheduled.shift()!();
  assert.equal(calls.some(c => c.op === 'stroke' && c.strokeStyle === '#aa0000'), false);
});

test('worker holds pending draws while fonts are loading and emits one painted ack after fonts resolve', async () => {
  const scheduled: (() => void)[] = [];
  const acks: number[] = [];
  let resolveFonts!: (result: { added: boolean; failed: readonly string[] }) => void;
  const handler = createTimeChartWorkerHandler({
    schedule: fn => scheduled.push(fn),
    loadFonts: () => new Promise(resolve => { resolveFonts = resolve; }),
    onPainted: (_id, frameId) => { acks.push(frameId); },
  });
  handler.handle({ type: 'tokens', tokens: TOKENS });
  handler.handle({ type: 'fonts', fontKey: 'TestSans', faces: [{ family: 'TestSans', source: 'url("x.woff2")' }] });
  const { canvas } = makeFakeCanvas(400, 200);
  handler.handle({ type: 'attach', id: 31, canvas });

  handler.handle(makeDraw(31, { frameId: 1 }));
  handler.handle(makeDraw(31, { frameId: 2 }));
  handler.drain();
  assert.deepEqual(acks, []);

  resolveFonts({ added: true, failed: [] });
  await Promise.resolve();
  assert.equal(scheduled.length, 1);
  scheduled.shift()!();
  assert.deepEqual(acks, [2]);
});

test('worker client skips resending equal descriptors across new layer arrays and stops resending cleared optional fields', async () => {
  const { plotFrame } = await import('./frame.ts');
  const { lineSummaryLayer } = await import('./renderTimeChart.ts');
  const { TimeChartWorkerPool } = await import('./timeChartWorkerClient.ts');
  const posted: Record<string, unknown>[] = [];
  class FakeWorker {
    onmessage: ((event: MessageEvent) => void) | null = null;
    onerror: ((event: ErrorEvent) => void) | null = null;
    onmessageerror: (() => void) | null = null;
    postMessage(msg: unknown) { posted.push(msg as Record<string, unknown>); }
    terminate() {}
  }
  class FakeOffscreenCanvas {}
  class FakeHTMLCanvasElement { transferControlToOffscreen() { return new FakeOffscreenCanvas(); } }
  const fakeDoc = {
    documentElement: { dataset: {} },
    body: { append() {} },
    createElement: () => ({ style: { display: '', color: '' }, remove() {} }),
    styleSheets: [],
  };
  const saved = new Map(['Worker', 'OffscreenCanvas', 'HTMLCanvasElement', 'document', 'getComputedStyle', 'window']
    .map(k => [k, Object.getOwnPropertyDescriptor(globalThis, k)]));
  Object.defineProperty(globalThis, 'Worker', { configurable: true, value: FakeWorker });
  Object.defineProperty(globalThis, 'OffscreenCanvas', { configurable: true, value: FakeOffscreenCanvas });
  Object.defineProperty(globalThis, 'HTMLCanvasElement', { configurable: true, value: FakeHTMLCanvasElement });
  Object.defineProperty(globalThis, 'document', { configurable: true, value: fakeDoc });
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { addEventListener() {}, removeEventListener() {} } });
  Object.defineProperty(globalThis, 'getComputedStyle', {
    configurable: true,
    value: () => ({ color: 'rgb(20, 24, 30)', getPropertyValue: () => 'sans-serif' }),
  });
  try {
    const pool = new TimeChartWorkerPool();
    const el = { transferControlToOffscreen: () => new FakeOffscreenCanvas(), closest: () => null } as unknown as HTMLCanvasElement;
    const handle = pool.attach(el, () => {})!;
    const frame = plotFrame(400, 200, { left: 44, right: 14, top: 12, bottom: 24 }, [0, 100], [0, 100], false, 0, false);
    const sharedSummary = constantSummary(50);

    handle.draw({
      frameId: handle.nextFrameId(),
      backingWidth: 400, backingHeight: 200, layoutWidth: 400, layoutHeight: 200, scaleX: 1, scaleY: 1, frame,
      layers: [lineSummaryLayer({ summary: () => sharedSummary, color: '#ff0000', options: { area: true } })],
      playhead: 100, playback: 'draw-in', until: 100,
      badges: [{ hour: 10, glyph: '*', label: 'ev' }],
      gaps: [{ firstHour: 20, lastHour: 30 }],
      rowLabels: ['A'],
      formatValue: 'powerAxis',
    });
    handle.draw({
      frameId: handle.nextFrameId(),
      backingWidth: 400, backingHeight: 200, layoutWidth: 400, layoutHeight: 200, scaleX: 1, scaleY: 1, frame,
      layers: [lineSummaryLayer({ summary: () => sharedSummary, color: '#ff0000', options: { area: true } })],
      playhead: 100, playback: 'draw-in', until: 100,
      badges: undefined,
      gaps: undefined,
      rowLabels: undefined,
      formatValue: undefined,
    });
    handle.draw({
      frameId: handle.nextFrameId(),
      backingWidth: 400, backingHeight: 200, layoutWidth: 400, layoutHeight: 200, scaleX: 1, scaleY: 1, frame,
      layers: [lineSummaryLayer({ summary: () => sharedSummary, color: '#ff0000', options: { area: true } })],
      playhead: 100, playback: 'draw-in', until: 100,
      badges: undefined,
      gaps: undefined,
      rowLabels: undefined,
      formatValue: undefined,
    });
    const draws = posted.filter(m => m.type === 'draw');
    assert.equal(draws.length, 3);
    assert.ok(draws[0]!.layers !== undefined);
    assert.equal(draws[1]!.layers, undefined);
    assert.equal(draws[1]!.badges, null);
    assert.equal(draws[1]!.gaps, null);
    assert.equal(draws[1]!.rowLabels, null);
    assert.equal(draws[1]!.formatValue, null);
    assert.equal(draws[2]!.layers, undefined);
    assert.equal(draws[2]!.badges, undefined);
    assert.equal(draws[2]!.gaps, undefined);
    assert.equal(draws[2]!.rowLabels, undefined);
    assert.equal(draws[2]!.formatValue, undefined);
  } finally {
    for (const [k, desc] of saved) {
      if (desc) Object.defineProperty(globalThis, k, desc); else Reflect.deleteProperty(globalThis, k);
    }
  }
});
