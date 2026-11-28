import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { Lane, type Bitmapped, type LaneHost, type Presented } from '../draw/lane.ts';
import { contextWidgetKey } from '../draw/kit.ts';
import { datedPanel, datedRecord } from '../dashboard/follow/messages.fixture.ts';
import type { FamilyTheme } from './contract.ts';
import { createLaneView } from './laneRuntime.ts';
import { type MotionClock } from './motion.ts';
import { registerFamilyProjection } from './registry.ts';
import { barsLane, type FamilyBarsBinding } from './bars/laneView.ts';
import type { BarsModel } from './bars/model.ts';
import { RecordedPath2D } from './time-chart/path2d.fixture.ts';
import { knownValue, notRecorded } from './value.ts';
import { defaultArtFrame, readyArt } from './art/types.ts';

const TEST_THEME: FamilyTheme = {
  surface: '#11161d',
  raised: '#1a222d',
  dark: true,
  text: '#eef2f6',
  muted: '#8d9bb0',
  line: '#2a3444',
  lineSoft: '#202836',
  accent: '#4f86f7',
  blue: '#4f86f7',
  red: '#e5534b',
  shortage: '#ff6b6f',
  amber: '#d29922',
  green: '#3fb950',
  grey: '#8d9bb0',
  hatch: '#6e7681',
  hatchEdge: '#484f58',
  font: 'Inter',
  fontSize: { small: 11, body: 12 },
  lineWidth: 1,
  hatchSpacing: 6,
};

const baseClock = (overrides: Partial<MotionClock> = {}): MotionClock => ({
  playing: true,
  anchorPosition: 100,
  anchorTimeMs: 1000,
  speed: 1,
  seek: 1,
  exactHour: 100,
  ...overrides,
});

interface CanvasOp {
  readonly op: string;
  readonly args: readonly unknown[];
  readonly fillStyle: unknown;
  readonly strokeStyle: unknown;
  readonly font: string;
  readonly globalAlpha: number;
  readonly textAlign: string;
  readonly textBaseline: string;
  readonly tx: number;
  readonly ty: number;
}

function makeRecordingCanvas(width = 960, height = 300) {
  const opsByWidget = new Map<object, CanvasOp[]>();
  const ownOps: CanvasOp[] = [];
  const currentKey = () => {
    const key = contextWidgetKey(ctx as unknown as CanvasRenderingContext2D);
    return key === ctx ? undefined : key;
  };
  const record = (op: string, args: readonly unknown[]) => {
    const entry: CanvasOp = {
      op,
      args,
      fillStyle: ctx.fillStyle,
      strokeStyle: ctx.strokeStyle,
      font: ctx.font,
      globalAlpha: ctx.globalAlpha,
      textAlign: ctx.textAlign,
      textBaseline: ctx.textBaseline,
      tx: transform.e,
      ty: transform.f,
    };
    const key = clipped ? currentKey() : undefined;
    if (!key) {
      ownOps.push(entry);
      return;
    }
    let list = opsByWidget.get(key);
    if (!list) opsByWidget.set(key, list = []);
    list.push(entry);
  };
  let clipped = false;
  let transform = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
  const stack: {
    fillStyle: unknown;
    strokeStyle: unknown;
    font: string;
    globalAlpha: number;
    lineWidth: number;
    textAlign: CanvasTextAlign;
    textBaseline: CanvasTextBaseline;
    transform: typeof transform;
    clipped: boolean;
  }[] = [];
  const gradient = () => ({ addColorStop() {} });
  const ctx = {
    font: '400 12px Inter',
    fillStyle: '' as unknown,
    strokeStyle: '' as unknown,
    lineWidth: 1,
    lineJoin: 'miter' as CanvasLineJoin,
    lineCap: 'butt' as CanvasLineCap,
    globalAlpha: 1,
    textAlign: 'left' as CanvasTextAlign,
    textBaseline: 'alphabetic' as CanvasTextBaseline,
    save() {
      stack.push({
        fillStyle: this.fillStyle,
        strokeStyle: this.strokeStyle,
        font: this.font,
        globalAlpha: this.globalAlpha,
        lineWidth: this.lineWidth,
        textAlign: this.textAlign,
        textBaseline: this.textBaseline,
        transform: { ...transform },
        clipped,
      });
    },
    restore() {
      const prev = stack.pop();
      if (prev) {
        this.fillStyle = prev.fillStyle;
        this.strokeStyle = prev.strokeStyle;
        this.font = prev.font;
        this.globalAlpha = prev.globalAlpha;
        this.lineWidth = prev.lineWidth;
        this.textAlign = prev.textAlign;
        this.textBaseline = prev.textBaseline;
        transform = prev.transform;
        clipped = prev.clipped;
      }
    },
    setTransform(a: number, b: number, c: number, d: number, e: number, f: number) {
      transform = { a, b, c, d, e, f };
    },
    resetTransform() {
      transform = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
    },
    getTransform: () => ({ ...transform }),
    translate(x: number, y: number) {
      transform = { ...transform, e: transform.e + x * transform.a, f: transform.f + y * transform.d };
    },
    scale(x: number, y: number) {
      transform = { ...transform, a: transform.a * x, d: transform.d * y };
    },
    clearRect(x: number, y: number, w: number, h: number) { record('clearRect', [x, y, w, h]); },
    fillRect(x: number, y: number, w: number, h: number) { record('fillRect', [x, y, w, h]); },
    strokeRect(x: number, y: number, w: number, h: number) { record('strokeRect', [x, y, w, h]); },
    beginPath() { record('beginPath', []); },
    closePath() { record('closePath', []); },
    rect(x: number, y: number, w: number, h: number) { record('rect', [x, y, w, h]); },
    roundRect(...args: unknown[]) { record('roundRect', args); },
    clip() { clipped = true; },
    moveTo(x: number, y: number) { record('moveTo', [x, y]); },
    lineTo(x: number, y: number) { record('lineTo', [x, y]); },
    quadraticCurveTo(...args: number[]) { record('quadraticCurveTo', args); },
    arc(...args: number[]) { record('arc', args); },
    arcTo(...args: number[]) { record('arcTo', args); },
    fill(path?: unknown) { record('fill', path instanceof RecordedPath2D ? [path.ops] : []); },
    stroke(path?: unknown) { record('stroke', path instanceof RecordedPath2D ? [path.ops] : []); },
    setLineDash(dash: number[]) { record('setLineDash', [dash]); },
    drawImage(...args: unknown[]) {
      const src = args[0] as { __bpOps?: readonly CanvasOp[] } | undefined;
      record('drawImage', src?.__bpOps ? [src.__bpOps.slice(), ...args.slice(1)] : args.slice(1));
    },
    fillText(text: string, x: number, y: number) { record('fillText', [text, x, y]); },
    strokeText(text: string, x: number, y: number) { record('strokeText', [text, x, y]); },
    createLinearGradient: gradient,
    createRadialGradient: gradient,
    createPattern: () => ({}),
    measureText(text: string) {
      const px = Number.parseFloat(/\b(\d+(?:\.\d+)?)px/.exec(this.font)?.[1] ?? '12') || 12;
      return { width: text.length * px * 0.55 };
    },
  };
  const canvas = {
    width,
    height,
    __bpOps: ownOps,
    getContext: () => ctx as unknown as OffscreenCanvasRenderingContext2D,
  };
  return {
    canvas: canvas as unknown as OffscreenCanvas,
    clearOps() { opsByWidget.clear(); ownOps.length = 0; },
    snapshotOps(): CanvasOp[][] {
      return [...opsByWidget.values()].map(list => list.slice());
    },
  };
}


const KIND = 'worker-motion-test';
const BLUE = '#4f86f7';
type Sample = { value: number | null; key?: string; form?: BarsModel['form']; target?: boolean; scale?: number; whole?: boolean; art?: boolean };

registerFamilyProjection({
  family: 'bars',
  kind: KIND,
  project(view, options): BarsModel {
    const sample = view.panel('s') as Sample;
    const numeric = sample?.value == null ? null : sample.value * Number(options ?? 1);
    const value = numeric === null ? notRecorded('Missing') : knownValue(sample.whole && numeric !== 0 ? BigInt(numeric) : numeric);
    return {
      form: sample?.form ?? 'ordered',
      hour: view.hour,
      showAxis: true,
      scaleMax: sample?.scale ?? 400,
      rows: [{
        kind: 'data',
        key: sample?.key ?? 'GER',
        label: knownValue('Germany'),
        value: { value },
        numeric,
        color: BLUE,
        ...(sample?.art ? { media: { kind: 'flag', tag: 'GER',
          art: readyArt('art://motion-fixture', { frame: defaultArtFrame(20, 13) }) } } : {}),
        ...(sample?.target ? { target: { numeric: 400, value: knownValue(400) } } : {}),
      }],
    };
  },
}, 'main');

type TestBitmap = Bitmapped & { close(): void };

async function withLane(run: (value: ReturnType<typeof fixture>) => Promise<void>, budget = Infinity) {
  const prior = Object.getOwnPropertyDescriptor(globalThis, 'OffscreenCanvas');
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number;
      height: number;
      readonly getContext: () => OffscreenCanvasRenderingContext2D | null;
      constructor(w: number, h: number) {
        this.width = w;
        this.height = h;
        const { canvas } = makeRecordingCanvas(w, h);
        this.getContext = () => canvas.getContext('2d');
      }
    },
    configurable: true,
  });
  try { await run(fixture(budget)); }
  finally {
    if (prior) Object.defineProperty(globalThis, 'OffscreenCanvas', prior);
    else Reflect.deleteProperty(globalThis, 'OffscreenCanvas');
  }
}

function fixture(budget: number) {
  const later: (() => void)[] = [];
  const presented: Presented<TestBitmap>[] = [];
  const served: Parameters<NonNullable<LaneHost<TestBitmap>['served']>>[0][] = [];
  const hits: (string | null)[] = [];
  const released: number[] = [];
  const sampled: number[] = [];
  const painted: number[] = [];
  let selectionCost = 0;
  let selections = 0;
  let ready = true;
  let paintCost = 0;
  let now = 1000;
  let settleArtwork!: (bitmap: ImageBitmap) => void;
  const artwork = new Promise<ImageBitmap>(resolve => { settleArtwork = resolve; });
  const view = createLaneView(barsLane, widget => lane.touch(widget), { fetchBitmap: () => artwork });
  const workerView = {
    ...view,
    paint(...args: Parameters<typeof view.paint>) {
      if (!ready) return { surfaces: [], fields: {}, motion: false, waiting: true as const };
      painted.push(args[0].widget);
      const result = view.paint(...args);
      const playback = result.playback;
      return playback ? { ...result, playback: (...visual: Parameters<typeof playback>) => {
        now += paintCost;
        sampled.push(args[0].widget);
        return playback(...visual);
      } } : result;
    },
  };
  const lane = new Lane<TestBitmap>({
    lane: 0, lanes: 1,
    views: kind => kind === 'bars' ? workerView : { paint: input => {
      painted.push(input.widget);
      return { surfaces: [], fields: {}, motion: false };
    } },
     crop: async () => [], release: bytes => released.push(bytes),
    present: reply => presented.push(reply), served: reply => served.push(reply),
    hit: (_id, key) => hits.push(key), fail: assert.fail, later: fn => later.push(fn),
    clock: () => now, budget,
  });
  const window = lane.facts.window.bind(lane.facts);
  lane.facts.window = (...args) => {
    now += selectionCost;
    selections++;
    return window(...args);
  };
  const canvas = makeRecordingCanvas(480, 240);
  lane.setLaneSurface(canvas.canvas, true);
  lane.follow(1, 'panel', 'p');
  lane.bindRoute(1, 1, 9, 1, 0);
  const bind = (measure = 1, id = 1) => {
    lane.payloads({ type: 'payloads', put: [[1, { s: 1 }], [10 + measure, measure]] });
    lane.bind(id, {
      generation: measure, owner: 'track', surfaces: [{ width: 480, height: 240, dpr: 1 }], offset: 0, pin: null,
    }, {
      family: 'bars', kind: KIND, view: 'main', options: measure, sources: { s: 1 }, scrollTop: 0,
      width: 480, height: 240, theme: TEST_THEME, audit: true,
      revisions: { options: 10 + measure, sources: 1 },
    } satisfies FamilyBarsBinding);
  };
  const mount = (count: number, sibling = false) => {
    const slots = Array.from({ length: count }, (_, i) => {
      const widget = i + 1;
      lane.mount(widget, [1], sibling && widget === count ? 'other' : 'bars', {});
      bind(1, widget);
      return { widget, x: 480 * i, y: 0, w: 480, h: 240, visible: true };
    });
    lane.setLaneSlots({
      width: 480 * count, height: 240, dpr: 1, scrollTop: 0, scrollLeft: 0, surfaceColor: TEST_THEME.surface,
      cornerRadius: 8, clipPathFallback: false, slots,
    });
  };
  mount(1);
  const record = (hour: number, value?: Sample, seek = 1) => lane.receive('panels', datedPanel(1, hour, value, seek, 9));
  let serial = 0;
  const drain = async () => { for (let i = 0; i < 16; i++) await Promise.resolve(); };
  const request = async (hour: number, speed = 0, seek = 1, acknowledge = false) => {
    const installed = acknowledge ? (presented.at(-1)?.entries ?? []).map(({ widget, binding, id }) => ({ widget, binding, id })) : [];
    lane.frame({ type: 'frame', serial: ++serial, seek, now: hour, target: hour, exact: speed === 0, speed, installed, refused: [], inspect: [[1, 'row:GER']] });
    await drain();
  };
  const clock = (anchorPosition: number, speed: number, seek = 1) => lane.setClock(baseClock({
    anchorPosition, exactHour: Math.floor(anchorPosition), anchorTimeMs: performance.timeOrigin + now,
    speed, playing: speed > 0, seek,
  }));
  const tick = (elapsed: number) => { now += elapsed; later.splice(0).forEach(run => run()); };
  const operations = () => canvas.snapshotOps().flat();
  const bar = () => {
    const paint = operations().reverse().find(op => (op.op === 'fillRect' || op.op === 'roundRect') && op.fillStyle === BLUE);
    assert.ok(paint, 'expected a painted bar');
    return paint;
  };
  const barWidth = () => Number(bar().args[2]);
  return {
    lane, canvas, record, request, clock, tick, operations, bar, barWidth, presented, served, hits, released, bind, mount, sampled, painted, drain,
    selectionCost: (value: number) => { selectionCost = value; selections = 0; },
    selections: () => selections,
    finishArtwork: () => settleArtwork({ width: 20, height: 13, close() {} } as ImageBitmap),
    ready: (value: boolean) => { ready = value; },
    paintCost: (value: number) => { paintCost = value; },
  };
}

describe('Ordered bar playback', () => {
  it('uses a cached confirmed bracket at maximum speed and immediately paints the paused hour', async () => withLane(async f => {
    f.record(100, { value: 100 });
    f.record(109);
    f.record(110, { value: 200 });
    await f.request(100);
    const start = f.barWidth();
    f.clock(100, 1440);
    f.tick(4);
    assert.equal(f.barWidth(), start);
    f.tick(9.5 / 1.44 - 4);
    assert.ok(f.barWidth() > start && f.barWidth() < start * 2);
    f.clock(109.5, 0);
    f.canvas.clearOps();
    await f.request(109);
    assert.equal(f.barWidth(), start);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '100'));
    f.canvas.clearOps();
    f.tick(16);
    assert.equal(f.operations().length, 0);
  }));

  it('cuts at a missing reading or changed row and never chases an unproved endpoint', async () => withLane(async f => {
    f.record(100, { value: 100 });
    f.record(101, { value: null });
    f.record(102, { value: 200 });
    f.record(103, { value: 400, key: 'ENG' });
    f.clock(100.5, 168);
    await f.request(100, 168);
    const start = f.barWidth();
    f.tick(0);
    assert.equal(f.barWidth(), start);
    f.tick(2 / .168);
    assert.equal(f.barWidth(), start * 2);
    f.canvas.clearOps();
    f.tick(1 / .168);
    await f.drain();
    const widths = f.operations().filter(op => (op.op === 'fillRect' || op.op === 'roundRect') && op.fillStyle === BLUE).map(op => op.args[2]);
    assert.deepEqual(widths, [start * 4], 'the first paint at the boundary uses the new rows without a page request');
  }));

  it('keeps the recorded labels, tooltip, mirror and audit through ordinary presentation', async () => withLane(async f => {
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    f.clock(100.5, 1);
    await f.request(100, 1);
    const first = f.presented.flatMap(reply => reply.entries).at(-1);
    assert.ok(first?.repainted, 'the ordinary page request paints the direct slot');
    assert.ok(first?.fields);
    const exact = JSON.stringify(first.fields);
    assert.match(exact, /100/);
    assert.match(exact, /mirror/);
    assert.match(exact, /tooltip/);
    assert.match(exact, /audit/);
    assert.doesNotMatch(exact, /150/);
    const credit = [...f.released];
    f.tick(0);
    assert.deepEqual(f.released, credit);
    assert.ok(f.served.some(frame => frame.motion && frame.changed === 1));
    const latest = f.presented.flatMap(reply => reply.entries).at(-1);
    assert.equal(latest, first);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '100'));
  }));

  it('leaves target bars and the exact page path alone', async () => withLane(async f => {
    f.record(100, { value: 100, target: true });
    f.record(101, { value: 200, target: true });
    f.clock(100.5, 1);
    await f.request(100, 1);
    assert.ok(f.presented.flatMap(reply => reply.entries).at(-1)?.repainted);
    const width = f.barWidth();
    f.tick(0);
    assert.equal(f.barWidth(), width);
  }));

  it('cuts when coverage is absent or a source refuses the next hour', async () => withLane(async f => {
    f.record(100, { value: 100 });
    f.lane.receive('panels', datedRecord(1, 101, 0, 'Unavailable'));
    f.record(103, { value: 300 });
    await f.request(100);
    const start = f.barWidth();
    f.clock(100.5, 1);
    f.tick(0);
    assert.equal(f.barWidth(), start);
    f.clock(102.5, 1);
    f.tick(0);
    assert.equal(f.barWidth(), start);
    f.clock(103.5, 1);
    f.tick(0);
    assert.equal(f.barWidth(), start * 3);
    f.clock(105.5, 1);
    f.tick(0);
    assert.equal(f.barWidth(), start * 3);
  }));

  it('repaints a changed measure exactly when the old bracket is no longer confirmed', async () => withLane(async f => {
    f.record(100, { value: 50 });
    f.record(101, { value: 100 });
    await f.request(100);
    const single = f.barWidth();
    f.clock(100.5, 1);
    f.tick(0);
    assert.ok(f.barWidth() > single);
    f.bind(3);
    f.lane.stream(1, 2, 100);
    f.record(100, { value: 50 }, 2);
    f.clock(100.5, 1, 2);
    await f.request(100, 1, 2);
    assert.equal(f.barWidth(), single * 3);
    f.tick(0);
    assert.equal(f.barWidth(), single * 3);
  }));

  it('wakes the normal direct painter when a loading projection becomes ready', async () => withLane(async f => {
    f.ready(false);
    f.clock(100.5, 1);
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    await f.request(100, 1);
    assert.equal(f.operations().length, 0);
    f.ready(true);
    f.lane.touch(1);
    for (let i = 0; i < 12; i++) await Promise.resolve();
    const exact = f.barWidth();
    f.tick(0);
    assert.ok(f.barWidth() > exact);
  }));

  it('gives subsequent slots their turn when a repaint consumes the worker budget', async () => withLane(async f => {
    f.mount(3);
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    await f.request(100);
    f.paintCost(11);
    f.clock(100.2, 1);
    f.tick(0);
    assert.deepEqual(f.sampled, [1]);
    f.tick(16);
    assert.deepEqual(f.sampled, [1, 2]);
    f.tick(16);
    assert.deepEqual(f.sampled, [1, 2, 3]);
  }, 10));

  it('uses extended date proof even when an unchanged entry is still pending', async () => withLane(async f => {
    f.lane.supply(1, { dated: { h: 100, c: 100 } });
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    f.clock(100.5, 1);
    await f.request(100, 1);
    const exact = f.barWidth();
    f.lane.supply(1, { dated: { h: 100, c: 101 } });
    f.tick(0);
    assert.ok(f.barWidth() > exact);
    await f.drain();
  }));

  it('restores the picture kept by the page when the paused hour is uncovered', async () => withLane(async f => {
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    f.clock(100.5, 1);
    await f.request(100, 1);
    const exact = f.barWidth();
    await f.request(100, 1, 1, true);
    f.tick(0);
    assert.ok(f.barWidth() > exact);
    const count = f.presented.flatMap(p => p.entries).length;
    f.clock(102, 0);
    await f.request(102);
    assert.equal(f.barWidth(), exact);
    assert.equal(f.presented.flatMap(p => p.entries).length, count);
  }));

  it('services one artwork wake while a matching presentation awaits acknowledgement', async () => withLane(async f => {
    f.record(100, { value: 100, art: true });
    f.clock(100, 1);
    await f.request(100, 1);
    assert.ok(!f.operations().some(op => op.op === 'drawImage'));
    f.painted.length = 0;
    f.finishArtwork();
    await f.drain();
    assert.deepEqual(f.painted, [1]);
    assert.equal(f.operations().filter(op => op.op === 'drawImage').length, 1);
    assert.equal(typeof f.presented.at(-1)?.entries[0]?.fields.meta, 'object');
    f.tick(16);
    await f.drain();
    assert.deepEqual(f.painted, [1]);
  }));

  it('does not serve a target bar or its dirty sibling between page requests', async () => withLane(async f => {
    f.mount(2, true);
    f.record(100, { value: 100, target: true });
    f.clock(100, 1);
    await f.request(100, 1);
    const served = f.served.length;
    f.painted.length = 0;
    f.lane.touch(2);
    f.lane.touch(1);
    await f.drain();
    assert.equal(f.served.length, served);
    assert.deepEqual(f.painted, []);
  }));

  it('services eligible artwork without serving a dirty sibling', async () => withLane(async f => {
    f.mount(2, true);
    f.record(100, { value: 100 });
    f.clock(100, 1);
    await f.request(100, 1);
    await f.request(100, 1, 1, true);
    f.painted.length = 0;
    f.lane.touch(2);
    f.lane.touch(1);
    await f.drain();
    assert.deepEqual(f.painted, [1]);
    await f.request(100, 1);
    assert.deepEqual(f.painted, [1, 2]);
  }));

  it('gives a sibling its ordinary paint before spending the pause budget on pending bar restorations', async () => withLane(async f => {
    f.mount(4, true);
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    f.clock(100.5, 1);
    await f.request(100, 1);
    f.tick(0);
    assert.deepEqual(f.sampled, [1, 2, 3]);
    f.painted.length = 0;
    f.selectionCost(6);
    f.clock(101, 0);
    await f.request(101);
    assert.equal(f.painted[0], 4);
    assert.ok(f.selections() <= 2, 'a pause slice cannot select every previewed bar before yielding');
    for (let i = 0; i < 4; i++) {
      f.tick(16);
      await f.drain();
    }
    assert.deepEqual(f.painted, [4, 1, 2, 3]);
  }, 10));

  it('interpolates a recorded number zero to a recorded whole-number bigint', async () => withLane(async f => {
    f.record(100, { value: 0, whole: true });
    f.record(101, { value: 200, whole: true });
    f.clock(100.5, 1);
    await f.request(100, 1);
    f.tick(0);
    const between = f.barWidth();
    f.clock(101, 0);
    await f.request(101);
    assert.equal(f.barWidth(), between * 2);
  }));

  it('uses the same interpolated scale for the width and the axis', async () => withLane(async f => {
    f.record(100, { value: 60, scale: 200 });
    f.record(101, { value: 100, scale: 400 });
    await f.request(100);
    f.canvas.clearOps();
    f.clock(100.5, 1);
    f.tick(0);
    const tick = f.operations().find(op => op.op === 'fillText' && op.args[0] === '100');
    assert.ok(tick);
    const oneHundred = Number(tick.args[1]) - Number(f.bar().args[0]);
    assert.ok(Math.abs(f.barWidth() / oneHundred - .8) < .02);
  }));
});
