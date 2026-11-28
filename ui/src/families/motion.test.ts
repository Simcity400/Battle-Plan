import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { Lane, type Bitmapped, type LaneHost, type Presented } from '../draw/lane.ts';
import { contextWidgetKey } from '../draw/kit.ts';
import { datedPanel, datedRecord, message } from '../dashboard/follow/messages.fixture.ts';
import { Page, type Ack, type Refusal } from '../dashboard/follow/page.ts';
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
    stagedCopies() {
      return [...ownOps, ...[...opsByWidget.values()].flat()].filter(op => op.op === 'drawImage' && Array.isArray(op.args[0]));
    },
  };
}


const KIND = 'worker-motion-test';
const BLUE = '#4f86f7';
type Sample = { value: number | null; key?: string; form?: BarsModel['form']; target?: boolean; segments?: boolean; scale?: number; whole?: boolean; art?: boolean; renames?: readonly number[]; france?: number };

const testProjection = registerFamilyProjection({
  family: 'bars',
  kind: KIND,
  singleValueBars: () => true,
  project(view, options): BarsModel {
    const panel = view.panel('s') as Sample | { rows: readonly unknown[]; refused: string | null } | undefined;
    const refused = view.refused('s') ?? (panel && 'rows' in panel && !panel.rows.length ? panel.refused : null);
    if (refused) return { form: 'ordered', hour: view.hour, rows: [], empty: refused, emptyKind: 'unavailable' };
    const sample = panel as Sample | undefined;
    const nextRename = sample?.renames?.find(at => view.hour < at);
    const name = sample?.renames?.filter(at => view.hour >= at).length;
    const numeric = sample?.value == null ? null : sample.value * Number(options ?? 1);
    const value = numeric === null ? notRecorded('Missing') : knownValue(sample?.whole && numeric !== 0 ? BigInt(numeric) : numeric);
    const rows: BarsModel['rows'][number][] = [{
      kind: 'data',
      key: sample?.key ?? 'GER',
      label: knownValue(name ? `Renamed ${name}` : 'Germany'),
      value: { value },
      numeric,
      color: BLUE,
      ...(sample?.art ? { media: { kind: 'flag', tag: 'GER',
        art: readyArt('art://motion-fixture', { frame: defaultArtFrame(20, 13) }) } } : {}),
      ...(sample?.target ? { target: { numeric: 400, value: knownValue(400) } } : {}),
      ...(sample?.segments ? { segments: [{ key: 'part', label: 'Part', share: 1, color: BLUE, value }] } : {}),
    }];
    if (sample?.france !== undefined) {
      const france = sample.france;
      rows.push({ kind: 'data', key: 'FRA', label: knownValue('France'), value: { value: knownValue(france) }, numeric: france, color: BLUE });
      if (numeric !== null && france > numeric) rows.reverse();
    }
    return {
      form: sample?.form ?? 'ordered',
      hour: view.hour,
      showAxis: true,
      scaleMax: sample?.scale ?? 400,
      ...(nextRename !== undefined ? { nextChangeAt: nextRename } : {}),
      rows,
    };
  },
}, 'main');
registerFamilyProjection({ ...testProjection, kind: `${KIND}-mixed`, singleValueBars: () => false }, 'main');

type TestBitmap = Bitmapped & { close(): void };

async function withLane(run: (value: ReturnType<typeof fixture>) => Promise<void>, budget = Infinity) {
  const prior = Object.getOwnPropertyDescriptor(globalThis, 'OffscreenCanvas');
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number;
      height: number;
      readonly getContext: () => OffscreenCanvasRenderingContext2D | null;
      readonly transferToImageBitmap: () => ImageBitmap;
      constructor(w: number, h: number) {
        this.width = w;
        this.height = h;
        const { canvas, snapshotOps, clearOps } = makeRecordingCanvas(w, h);
        this.getContext = () => canvas.getContext('2d');
        this.transferToImageBitmap = () => {
          const bitmap = { width: this.width, height: this.height, __bpOps: snapshotOps().flat(), close() {} };
          clearOps();
          return bitmap as ImageBitmap;
        };
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
  let siblingCost = 0;
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
      return playback ? { ...result, playback: {
        ...playback,
        paint(...visual: Parameters<typeof playback.paint>) {
          now += paintCost;
          sampled.push(args[0].widget);
          return playback.paint(...visual);
        },
      } } : result;
    },
  };
  const lane = new Lane<TestBitmap>({
    lane: 0, lanes: 1,
    views: kind => kind === 'bars' ? workerView : { paint: input => {
      now += siblingCost;
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
  const bind = (measure = 1, id = 1, reducedMotion = false, mixed = false) => {
    lane.payloads({ type: 'payloads', put: [[1, { s: 1 }], [10 + measure, measure]] });
    lane.bind(id, {
      generation: measure, owner: 'track', surfaces: [{ width: 480, height: 240, dpr: 1 }], offset: 0, pin: null,
    }, {
      family: 'bars', kind: mixed ? `${KIND}-mixed` : KIND, view: 'main', options: measure, sources: { s: 1 }, scrollTop: 0,
      width: 480, height: 240, theme: TEST_THEME, audit: true, reducedMotion,
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
  const request = async (hour: number, speed = 0, seek = 1, acknowledge: boolean | Ack[] = false, exact = speed === 0, target = hour, refused: Refusal[] = []) => {
    const installed = Array.isArray(acknowledge) ? acknowledge
      : acknowledge ? (presented.at(-1)?.entries ?? []).map(({ widget, binding, id }) => ({ widget, binding, id })) : [];
    lane.frame({ type: 'frame', serial: ++serial, seek, now: hour, target, exact, speed, installed, refused, inspect: [[1, 'row:GER']] });
    await drain();
  };
  const clock = (anchorPosition: number, speed: number, seek = 1, diagnostics = false) => lane.setClock(baseClock({
    anchorPosition, exactHour: Math.floor(anchorPosition), anchorTimeMs: performance.timeOrigin + now,
    speed, playing: speed > 0, seek,
    ...(diagnostics ? { diagnostics: true } : {}),
  }));
  const tick = (elapsed: number) => { now += elapsed; later.splice(0).forEach(run => run()); };
  const spendOnFactSelection = (cost: number) => {
    const select = lane.facts.select.bind(lane.facts);
    lane.facts.select = (...args) => {
      now += cost;
      return select(...args);
    };
  };
  const operations = () => canvas.snapshotOps().flat();
  const bar = () => {
    const paint = operations().reverse().find(op => (op.op === 'fillRect' || op.op === 'roundRect') && op.fillStyle === BLUE);
    assert.ok(paint, 'expected a painted bar');
    return paint;
  };
  const barWidth = () => Number(bar().args[2]);
  return {
    lane, canvas, record, request, clock, tick, operations, bar, barWidth, presented, served, hits, released, bind, mount, sampled, painted, drain, spendOnFactSelection,
    selectionCost: (value: number) => { selectionCost = value; selections = 0; },
    selections: () => selections,
    finishArtwork: () => settleArtwork({ width: 20, height: 13, close() {} } as ImageBitmap),
    ready: (value: boolean) => { ready = value; },
    paintCost: (value: number) => { paintCost = value; },
    siblingCost: (value: number) => { siblingCost = value; },
  };
}

describe('Ordered bar playback', () => {
  it('holds the recorded picture when factory playback outruns the delivered proof', async () => withLane(async f => {
    const base = 60788000;
    f.record(base + 28, { value: 30 });
    f.record(base + 50);
    f.clock(base + 47.008, 1440, 1, true);
    await f.request(base + 47, 1440, 1, false, true);
    const unit = f.barWidth() / 30;
    let recorded = 30;
    for (const sample of [
      { position: 61.984, target: 75, expected: 30 },
      { position: 77.248, target: 93, expected: 30 },
      { position: 92.224, target: 107, delivered: [77, 98, 60], expected: 60 },
      { position: 107.488, target: 122, expected: 60 },
      { position: 122.752, target: 137, expected: 60 },
      { position: 137.872, target: 152, delivered: [115, 122, 60], expected: 60 },
      { position: 152.992, target: 167, delivered: [123, 146, 100], expected: 100 },
    ] as const) {
      if (sample.delivered) {
        const [first, through, value] = sample.delivered;
        f.record(base + first, { value });
        f.record(base + through);
      }
      const hour = base + Math.floor(sample.position);
      f.clock(base + sample.position, 1440, 1, true);
      await f.request(hour, 1440, 1, true, false, base + sample.target);
      await f.request(hour, 1440, 1, true, false, base + sample.target);
      assert.ok(Math.abs(f.barWidth() / unit - recorded) < .01, 'the page leaves playing pixels to the tick');
      f.tick(0);
      assert.ok(Math.abs(f.barWidth() / unit - sample.expected) < .01);
      recorded = sample.expected;
    }
    const trace = f.served.filter(frame => frame.motionDiagnostics);
    assert.equal(trace.length, 7);
    assert.equal(trace.reduce((count, frame) => count + frame.changed, 0), 0);
    assert.equal(trace.reduce((count, frame) => count + (frame.motionDiagnostics?.skipped.unproven ?? 0), 0), 6);
  }));

  it('does not restore pixels after a refusal of a metadata-only entry', async () => withLane(async f => {
    for (const [hour, value] of [[100, 30], [101, 60], [102, 100]] as const) f.record(hour, { value });
    f.clock(100, 1);
    await f.request(100, 1);
    const exact = f.barWidth();
    await f.request(100, 1, 1, true);
    f.clock(101.5, 1);
    f.tick(0);
    await f.request(101, 1);
    const metadata = f.presented.at(-1)!.entries[0]!;
    assert.ok(!metadata.repainted);
    await f.request(101, 1, 1, false, false, 101, [{ widget: 1, id: metadata.id, reason: 'superseded' }]);
    assert.ok(Math.abs(f.barWidth() / exact - 80 / 30) < .01);
  }));

  it('paints exact values at recorded hours, including the last endpoint, without waiting for the page', async () => withLane(async f => {
    for (const [hour, value] of [[100, 30], [101, 60], [102, 100]] as const) f.record(hour, { value });
    f.clock(100, 1);
    await f.request(100, 1);
    const unit = f.barWidth() / 30;
    for (const [elapsed, expected, label] of [[500, 45, '30'], [500, 60, '60'], [500, 80, '60'], [500, 100, '100']] as const) {
      f.canvas.clearOps();
      f.tick(elapsed);
      assert.ok(Math.abs(f.barWidth() / unit - expected) < .01);
      assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === label));
    }
  }));

  it('paints a covered pause after an older eligible stage is acknowledged over a direct picture', async () => withLane(async f => {
    f.record(100, { value: 30 });
    f.record(123);
    f.record(124, { value: 60 });
    f.clock(100, 1);
    await f.request(100, 1, 1, false, false, 124);
    const staged = f.presented.at(-1)!.entries[0]!;
    assert.equal(staged.lo, 124);
    await f.request(100, 1, 1, false, true);
    const exact = f.barWidth();
    f.clock(124, 0);
    await f.request(124, 0, 1, [staged]);
    assert.ok(Math.abs(f.barWidth() / exact - 2) < .01);
  }));

  it('resumes a matching renderer from a picture accepted while paused', async () => withLane(async f => {
    f.record(124, { value: 60 });
    f.record(147);
    f.record(148, { value: 100 });
    f.clock(100, 1);
    await f.request(101, 1, 1, false, false, 124);
    const staged = f.presented.at(-1)!.entries[0]!;
    assert.equal(staged.lo, 124);
    f.clock(124, 0);
    await f.request(124, 0, 1, [staged]);
    const unit = f.barWidth() / 60;
    f.clock(136, 1);
    await f.request(136, 1, 1, true);
    f.tick(0);
    assert.ok(Math.abs(f.barWidth() / unit - 80) < .01);
  }));

  it('rebuilds the exact paused picture and renderer for direct, staged, extended and never-owned slots', async () => {
    let exactWidth: number | undefined;
    for (const history of ['direct', 'staged', 'extended', 'never-owned']) await withLane(async f => {
      f.record(100, { value: 30 });
      f.record(123);
      f.record(124, { value: 60 });
      f.record(147);
      f.record(148, { value: 100 });
      if (history === 'extended') f.lane.supply(1, { dated: { h: 100, c: 148 } });
      f.clock(100, 1);
      if (history === 'direct') await f.request(124, 1);
      else await f.request(100, 1, 1, false, false, 124);
      const ready = f.presented.at(-1)!.entries[0]!;
      assert.equal(ready.lo, 124);
      if (history === 'extended') await f.request(100, 1, 1, false, true);
      if (history === 'extended') f.lane.supply(1, { dated: { h: 100, c: 149 } });
      f.canvas.clearOps();
      f.clock(124, 0);
      await f.request(124, 0, 1, history === 'never-owned' ? [] : [ready]);
      const widths = f.operations()
        .filter(op => (op.op === 'fillRect' || op.op === 'roundRect') && op.fillStyle === BLUE)
        .map(op => Number(op.args[2]));
      assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '60'), history);
      assert.equal(widths.length, 1, history);
      exactWidth ??= widths[0]!;
      assert.equal(widths[0], exactWidth, history);
      f.clock(136, 1);
      await f.request(136, 1, 1, true);
      f.tick(0);
      assert.ok(Math.abs(f.barWidth() / exactWidth - 80 / 60) < .01, history);
    });
  });

  it('settles to the proven endpoint when the clock skips beyond it before any acknowledgement', async () => withLane(async f => {
    f.record(100, { value: 30 });
    f.record(101, { value: 60 });
    f.clock(100, 1);
    await f.request(100, 1);
    const unit = f.barWidth() / 30;
    await f.request(100, 1, 1, true);
    f.tick(500);
    assert.ok(Math.abs(f.barWidth() / unit - 45) < .01);
    f.tick(1500);
    assert.ok(Math.abs(f.barWidth() / unit - 60) < .01);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '60'));
  }));

  it('animates the accepted interval even while a later staged picture awaits acknowledgement', async () => withLane(async f => {
    f.record(124, { value: 60 });
    f.record(147);
    f.record(148, { value: 100 });
    f.clock(100, 1);
    await f.request(101, 1, 1, false, false, 124);
    const first = f.presented.at(-1)!.entries[0]!;
    await f.request(102, 1, 1, false, false, 148);
    const later = f.presented.at(-1)!.entries[0]!;
    assert.deepEqual([first.lo, later.lo], [124, 148]);
    f.clock(124, 1);
    await f.request(124, 1, 1, [first], false, 148);
    f.tick(0);
    const unit = f.barWidth() / 60;
    f.clock(136, 1);
    f.tick(0);
    assert.ok(Math.abs(f.barWidth() / unit - 80) < .01);
    await f.request(148, 1, 1, [later], false, 148);
    assert.ok(Math.abs(f.barWidth() / unit - 80) < .01);
    assert.equal(f.canvas.stagedCopies().length, 0);
  }));

  it('starts the bracket at the last model boundary regardless of which ticks ran', async () => {
    for (const prior of [[], [105.5, 111.5]]) await withLane(async f => {
      f.record(100, { value: 30, renames: [105, 110] });
      f.record(123);
      f.record(124, { value: 100, renames: [105, 110] });
      f.clock(100, 1);
      await f.request(100, 1);
      const unit = f.barWidth() / 30;
      for (const position of [...prior, 117]) {
        f.clock(position, 1);
        f.tick(0);
      }
      assert.ok(Math.abs(f.barWidth() / unit - 65) < .01);
      assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === 'Renamed 2'));
    });
  });

  it('lets a reduced-motion binding resume shared look-ahead on its first request', async () => withLane(async f => {
    f.record(100, { value: 30 });
    f.record(123);
    f.record(124, { value: 60 });
    f.clock(100, 1);
    await f.request(100, 1);
    await f.request(100, 1, 1, true);
    f.tick(500);
    f.bind(2, 1, true);
    await f.request(101, 1, 1, false, false, 124);
    assert.equal(f.presented.at(-1)?.entries[0]?.lo, 124);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '60'));
  }));

  it('publishes the visible hour when a supply update restores playback ownership during look-ahead', async () => withLane(async f => {
    f.record(100, { value: 30 });
    f.record(101, { value: 60 });
    f.record(102, { value: 100 });
    f.clock(100, 1);
    await f.request(100, 1);
    const unit = f.barWidth() / 30;
    await f.request(100, 1, 1, true);
    f.lane.supply(1, { statics: {} });
    f.clock(101.5, 1);
    await f.request(101, 1, 1, false, false, 102);
    const entry = f.presented.at(-1)?.entries[0];
    assert.equal(entry?.lo, 101);
    assert.equal(entry?.repainted, undefined);
    assert.ok(Math.abs(f.barWidth() / unit - 60) < .01);
    f.tick(0);
    assert.ok(Math.abs(f.barWidth() / unit - 80) < .01);
  }));

  it('hands a staged slot to playback at the accepted hour', async () => withLane(async f => {
    f.record(124, { value: 60 });
    f.record(147);
    f.record(148, { value: 100 });
    f.clock(100.5, 1);
    f.canvas.clearOps();
    await f.request(101, 1, 1, false, false, 124);
    const staged = f.presented.at(-1)?.entries[0];
    assert.equal(staged?.lo, 124);
    f.tick(0);
    assert.deepEqual(f.sampled, []);
    assert.equal(f.canvas.stagedCopies().length, 0);
    f.clock(124, 1);
    await f.request(124, 1, 1, [staged!], false, 148);
    assert.equal(f.canvas.stagedCopies().length, 0);
    f.tick(0);
    assert.equal(f.sampled.length, 1);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '60'));
    const exact = f.barWidth();
    f.clock(136, 1);
    f.tick(0);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '60'));
    assert.ok(Math.abs(f.barWidth() / exact - 80 / 60) < .01);
    f.clock(148, 0);
    await f.request(148);
    assert.ok(Math.abs(f.barWidth() / exact - 100 / 60) < .01);
    assert.equal(f.canvas.stagedCopies().length, 0);
  }));

  it('does not copy a delayed staged acknowledgement over a newer live bar', async () => withLane(async f => {
    f.record(100, { value: 30 });
    f.record(123);
    f.record(124, { value: 60 });
    f.record(147);
    f.record(148, { value: 100 });
    f.clock(100.5, 1);
    await f.request(100, 1, 1, false, false, 124);
    const staged = f.presented.at(-1)?.entries[0];
    assert.equal(staged?.lo, 124);
    await f.request(100, 1, 1, false, true);
    f.clock(136, 1);
    f.tick(0);
    const width = f.barWidth();
    await f.request(124, 1, 1, [staged!], false, 148);
    assert.equal(f.canvas.stagedCopies().length, 0);
    assert.equal(f.barWidth(), width);
  }));

  it('draws the accepted first picture on the tick even when the shared slot was dirty', async () => withLane(async f => {
    f.record(124, { value: 60 });
    f.record(147);
    f.record(148, { value: 100 });
    f.clock(100, 1);
    await f.request(100, 1, 1, false, false, 124);
    const staged = f.presented.at(-1)?.entries[0];
    assert.equal(staged?.lo, 124);
    f.clock(124, 1);
    await f.request(124, 1, 1, [staged!], false, 148);
    f.tick(0);
    const exact = f.barWidth();
    assert.equal(f.canvas.stagedCopies().length, 0);
    f.tick(12000);
    assert.ok(Math.abs(f.barWidth() / exact - 80 / 60) < .01);
  }));

  it('uses the visible hour for an active bar even when the page requests a different future picture', async () => withLane(async f => {
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    f.record(103, { value: 400, key: 'ENG' });
    f.clock(100.5, 1);
    await f.request(100, 1, 1, false, true);
    const exact = f.barWidth();
    f.lane.touch(1);
    await f.request(100, 1, 1, true, false, 103);
    f.tick(0);
    assert.ok(Math.abs(f.barWidth() - exact * 1.5) < 1);
    assert.equal(f.presented.at(-1)?.entries[0]?.lo, 100);
    f.clock(101.5, 0);
    await f.request(101);
    assert.ok(Math.abs(f.barWidth() - exact * 2) < 1);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '200'));
  }));

  it('uses a cached confirmed bracket at maximum speed and immediately paints the paused hour', async () => withLane(async f => {
    f.record(100, { value: 100 });
    f.record(108);
    f.record(109, { value: 100 });
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

  it('repaints every maximum-speed tick between changing confirmed samples across native step intervals', async () => withLane(async f => {
    for (let at = 0; at <= 6; at++) {
      const hour = 100 + 24 * at;
      f.record(hour, { value: 40 + 40 * at });
      f.record(hour + 23);
    }
    f.clock(100, 1440, 1, true);
    await f.request(100, 1440);
    const presents = f.presented.length;
    const unitWidth = f.barWidth() / 40;
    let previous = f.barWidth();
    for (let tick = 0; tick < 8; tick++) {
      f.canvas.clearOps();
      f.tick(1000 / 95);
      const width = f.barWidth();
      const elapsedHours = (tick + 1) * 1440 / 95;
      const expected = 40 + elapsedHours * 40 / 24;
      const recorded = 40 + Math.floor(elapsedHours / 24) * 40;
      assert.ok(width > previous, `tick ${tick + 1} advances the visible bar`);
      assert.ok(Math.abs(width / unitWidth - expected) < .01);
      const rowLabels = f.operations().filter(op => op.op === 'fillText' && Math.abs(Number(op.args[2]) - Number(f.bar().args[1])) < 20);
      assert.ok(rowLabels.some(op => op.args[0] === String(recorded)));
      assert.ok(!rowLabels.some(op => op.args[0] === String(Math.round(expected))));
      previous = width;
    }
    assert.equal(f.served.filter(frame => frame.motionDiagnostics).reduce((count, frame) => count + frame.changed, 0), 8);
    assert.equal(f.presented.length, presents);
    f.clock(244, 1440, 1, true);
    await f.request(244, 1440);
    f.tick(0);
    assert.ok(Math.abs(f.barWidth() / unitWidth - 280) < .01);
    f.clock(244, 0, 1, true);
    await f.request(244);
    f.tick(1000 / 95);
    assert.equal(f.served.filter(frame => frame.motionDiagnostics).reduce((count, frame) => count + frame.changed, 0), 8);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '280'));
  }));

  it('does not interpolate a long confirmed value toward a reading beyond an unproven hour', async () => withLane(async f => {
    f.record(100, { value: 60 });
    f.record(123);
    f.record(126, { value: 100 });
    f.clock(100, 1440, 1, true);
    await f.request(100, 1440);
    const exact = f.barWidth();
    f.tick(1000 / 95);
    assert.equal(f.barWidth(), exact);
    assert.ok(f.served.some(frame => frame.motionDiagnostics?.skipped.noBracket));
    assert.ok(!f.served.some(frame => frame.motionDiagnostics && frame.changed));
    f.clock(126, 1440);
    await f.request(126, 1440);
    f.tick(0);
    assert.ok(Math.abs(f.barWidth() / exact - 100 / 60) < .01);
  }));

  it('draws exact missing readings and row boundaries without interpolating across either', async () => withLane(async f => {
    f.record(100, { value: 100 });
    f.record(101, { value: null });
    f.record(102, { value: 200 });
    f.record(103, { value: 400, key: 'ENG' });
    f.clock(100.5, 168, 1, true);
    await f.request(100, 168);
    const start = f.barWidth();
    f.canvas.clearOps();
    f.tick(0);
    assert.equal(f.barWidth(), start);
    f.clock(101.5, 168, 1, true);
    f.tick(0);
    assert.ok(f.operations().some(op => op.op === 'fillText' && String(op.args[0]).includes('recorded')));
    f.clock(102.5, 168, 1, true);
    f.tick(0);
    assert.equal(f.barWidth(), start * 2);
    f.canvas.clearOps();
    f.clock(103.5, 168, 1, true);
    f.tick(0);
    assert.equal(f.barWidth(), start * 4);
    assert.ok(!f.served.some(frame => frame.motionDiagnostics && frame.changed));
  }));

  it('withholds the installed row hit while the canvas shows a newly proven order', async () => withLane(async f => {
    f.record(100, { value: 100, france: 50 });
    f.record(101, { value: 100, france: 200 });
    f.clock(100, 1);
    await f.request(100, 1);
    const first = f.presented.at(-1)!.entries[0]!;
    await f.request(100, 1, 1, true);
    const [[, , , , , , top, height]] = first.fields.nav as [string, string, number, number, string, unknown, number, number][];
    const point: [number, number] = [32, top + height / 2];
    f.lane.pointer(1, 0, point, first.id);
    assert.deepEqual(f.hits, ['row:GER']);

    f.clock(101, 1);
    f.canvas.clearOps();
    f.tick(0);
    const labels = f.operations().filter(op => op.op === 'fillText' && ['Germany', 'France'].includes(String(op.args[0])));
    assert.deepEqual(labels.map(op => op.args[0]), ['France', 'Germany']);
    f.lane.pointer(1, 0, point, first.id);
    assert.deepEqual(f.hits, ['row:GER', null]);

    await f.request(101, 1);
    const second = f.presented.at(-1)!.entries[0]!;
    await f.request(101, 1, 1, true);
    f.lane.pointer(1, 0, point, second.id);
    assert.deepEqual(f.hits, ['row:GER', null, 'row:FRA']);
  }));

  it('does not paint or publish newly segmented rows between page requests', async () => withLane(async f => {
    f.bind(1, 1, false, true);
    f.record(100, { value: 100 });
    f.record(101, { value: 200, segments: true });
    f.record(102, { value: 300, segments: true });
    f.clock(100.5, 1);
    await f.request(100, 1);
    const before = f.presented.length;
    f.canvas.clearOps();
    f.clock(101.5, 1);
    f.tick(0);
    await f.drain();
    assert.equal(f.operations().length, 0);
    assert.equal(f.presented.length, before);
    await f.request(101, 1);
    assert.equal(f.presented.at(-1)?.entries[0]?.lo, 101);
  }));

  it('keeps a mixed-form binding on the page path regardless of intervening ticks', async () => {
    for (const visitSegments of [false, true]) await withLane(async f => {
      f.bind(1, 1, false, true);
      f.record(100, { value: 100 });
      f.record(101, { value: 200, segments: true });
      f.record(102, { value: 300 });
      f.record(103, { value: 400 });
      f.clock(100, 1);
      await f.request(100, 1);
      const width = f.barWidth();
      if (visitSegments) {
        f.clock(101.5, 1);
        f.tick(0);
      }
      f.clock(102.5, 1);
      f.tick(0);
      assert.equal(f.barWidth(), width);
      assert.deepEqual(f.sampled, []);
      await f.request(101, 1, 1, true);
      assert.equal(f.presented.at(-1)?.entries[0]?.lo, 101);
    });
  });

  it('reports an invalid single-value declaration and keeps that binding on the ordinary path', async t => withLane(async f => {
    const errors = t.mock.method(console, 'error', () => {});
    f.record(100, { value: 100, target: true });
    f.record(101, { value: 200 });
    f.record(102, { value: 300 });
    f.clock(100, 1);
    await f.request(100, 1);
    assert.equal(errors.mock.callCount(), 1);
    assert.match(String(errors.mock.calls[0]?.arguments[0]), /single-value guarantee/);
    f.clock(101.5, 1);
    await f.request(101, 1, 1, true);
    const width = f.barWidth();
    f.tick(0);
    assert.equal(f.barWidth(), width);
    assert.deepEqual(f.sampled, []);
  }));

  it('releases a binding once an adjacent endpoint contradicts its single-value guarantee', async t => withLane(async f => {
    const errors = t.mock.method(console, 'error', () => {});
    f.record(100, { value: 100 });
    f.record(101, { value: 200, segments: true });
    f.record(102, { value: 300 });
    f.record(103, { value: 400 });
    f.clock(100.5, 1);
    await f.request(100, 1);
    f.tick(0);
    assert.equal(errors.mock.callCount(), 1);
    assert.match(String(errors.mock.calls[0]?.arguments[0]), /single-value guarantee/);

    f.clock(102.5, 1);
    f.canvas.clearOps();
    f.tick(0);
    assert.equal(f.operations().length, 0);
    await f.request(102, 1, 1, true);
    assert.equal(f.presented.at(-1)?.entries[0]?.repainted, true);
    f.tick(0);
    assert.deepEqual(f.sampled, [1]);
    assert.equal(errors.mock.callCount(), 1);
  }));

  it('retains ordinary pause priority for a target-marked bar', async () => withLane(async f => {
    f.mount(2, true);
    f.bind(1, 1, false, true);
    f.record(100, { value: 100, target: true });
    f.record(101, { value: 200, target: true });
    f.clock(100, 1);
    await f.request(100, 1);
    f.painted.length = 0;
    f.clock(101, 0);
    await f.request(101, 0, 1, true);
    assert.equal(f.painted[0], 1);
  }));

  it('builds a renderer when an uncovered pause restores an accepted stage', async () => withLane(async f => {
    f.lane.supply(1, { dated: { h: 124, c: 124 } });
    f.record(124, { value: 60 });
    f.record(147);
    f.record(148, { value: 100 });
    f.clock(100, 1);
    await f.request(100, 1, 1, false, false, 124);
    const staged = f.presented.at(-1)?.entries[0];
    assert.equal(staged?.lo, 124);
    f.clock(125, 0);
    await f.request(125, 0, 1, [staged!]);
    const exact = f.barWidth();
    f.lane.supply(1, { dated: { h: 124, c: 148 } });
    f.clock(136, 1);
    await f.request(136, 1, 1, true);
    f.tick(0);
    assert.ok(Math.abs(f.barWidth() / exact - 80 / 60) < .01);
  }));

  it('keeps an uncovered pause pending until the accepted picture can actually draw', async () => withLane(async f => {
    f.record(124, { value: 60 });
    f.clock(100, 1);
    await f.request(100, 1, 1, false, false, 124);
    const staged = f.presented.at(-1)?.entries[0];
    assert.equal(staged?.lo, 124);
    f.canvas.clearOps();
    f.ready(false);
    f.clock(125, 0);
    await f.request(125, 0, 1, [staged!]);
    assert.ok(!f.operations().some(op => op.op === 'fillText' && op.args[0] === '60'));
    f.ready(true);
    await f.request(125);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '60'));
    f.canvas.clearOps();
    await f.request(125);
    assert.equal(f.operations().length, 0);
  }));

  it('keeps the recorded labels, tooltip, mirror and audit through ordinary presentation', async () => withLane(async f => {
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    f.clock(100.5, 1, 1, true);
    await f.request(100, 1);
    const first = f.presented.flatMap(reply => reply.entries).at(-1);
    assert.ok(first?.repainted, 'the ordinary page request paints the direct slot');
    assert.ok(first?.fields);
    const { mirror, tooltip, audit } = first.fields;
    const exact = JSON.stringify({ mirror, tooltip, audit });
    assert.match(exact, /100/);
    assert.match(exact, /mirror/);
    assert.match(exact, /tooltip/);
    assert.match(exact, /audit/);
    assert.doesNotMatch(exact, /150/);
    const credit = [...f.released];
    f.tick(0);
    assert.deepEqual(f.released, credit);
    assert.ok(f.served.some(frame => frame.motionDiagnostics && frame.changed === 1));
    const latest = f.presented.flatMap(reply => reply.entries).at(-1);
    assert.equal(latest, first);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '100'));
  }));

  it('leaves target bars and the exact page path alone', async () => withLane(async f => {
    f.bind(1, 1, false, true);
    f.record(100, { value: 100, target: true });
    f.record(101, { value: 200, target: true });
    f.clock(100.5, 1);
    await f.request(100, 1);
    assert.ok(f.presented.flatMap(reply => reply.entries).at(-1)?.repainted);
    const width = f.barWidth();
    f.tick(0);
    assert.equal(f.barWidth(), width);
  }));

  it('draws exact transport and rank refusals within their proof and cuts across adjacent gaps', async () => {
    for (const origin of ['transport', 'rank']) await withLane(async f => {
      f.record(100, { value: 100 });
      f.lane.receive('panels', origin === 'transport' ? datedRecord(1, 101, 0, 'Unavailable') : datedPanel(1, 101, {
        branchId: '1', hour: 101, from: 101, until: 101, measures: [{ name: 'ownerDivisions', decimals: 0 }],
        refused: 'The ranking is incomplete', rows: [],
      }));
      f.record(103, { value: 300 });
      await f.request(100);
      const start = f.barWidth();
      for (const [position, unavailable] of [[100.5, false], [101.5, true], [102.5, false]] as const) {
        f.canvas.clearOps();
        f.clock(position, 1, 1, true);
        f.tick(0);
        const barDrawn = f.operations().some(op => (op.op === 'fillRect' || op.op === 'roundRect') && op.fillStyle === BLUE);
        assert.equal(barDrawn, !unavailable, `${origin} at ${position}`);
        if (unavailable) assert.equal(f.served.at(-1)?.motionDiagnostics?.recorded, 1);
        else assert.equal(f.barWidth(), start);
      }
      f.clock(103.5, 1);
      f.tick(0);
      assert.equal(f.barWidth(), start * 3);
      assert.ok(!f.served.some(frame => frame.motionDiagnostics && frame.changed));
    });
  });

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

  it('holds the seek-carried reading through and beyond its proof until fresh facts arrive', async () => withLane(async f => {
    f.record(100, { value: 50 });
    f.record(123);
    f.record(124, { value: 100 });
    f.clock(100, 1);
    await f.request(100, 1);
    const exact = f.barWidth();
    await f.request(100, 1, 1, true);
    f.lane.stream(1, 2, 110);
    assert.ok(f.lane.facts.select(1, 110)?.carried);
    f.clock(110, 0, 2);
    await f.request(110, 0, 2);
    assert.equal(f.barWidth(), exact);

    f.clock(110.5, 1, 2, true);
    await f.request(110, 1, 2, true);
    f.canvas.clearOps();
    f.tick(0);
    assert.equal(f.barWidth(), exact);
    f.clock(124.5, 1, 2, true);
    f.canvas.clearOps();
    f.tick(0);
    assert.equal(f.barWidth(), exact);
    assert.equal(f.served.at(-1)?.motionDiagnostics?.skipped.unproven, 1);
    f.record(124, { value: 90 }, 2);
    f.clock(122.5, 1, 2, true);
    f.canvas.clearOps();
    f.tick(0);
    assert.equal(f.barWidth(), exact);
    assert.equal(f.served.at(-1)?.changed, 0);
  }));

  it('paints a newly ready projection on the next ordinary playing request', async () => withLane(async f => {
    f.ready(false);
    f.clock(100.5, 1);
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    await f.request(100, 1);
    assert.equal(f.operations().length, 0);
    f.ready(true);
    f.lane.touch(1);
    await f.drain();
    assert.equal(f.operations().length, 0);
    await f.request(100, 1);
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

  it('budgets fact selection even if each playing slot already shows its exact value', async () => withLane(async f => {
    f.mount(3);
    f.record(100, { value: 100 });
    f.clock(100, 1);
    await f.request(100, 1);
    f.spendOnFactSelection(11);
    f.tick(0);
    assert.deepEqual(f.sampled, [1]);
    f.tick(0);
    assert.deepEqual(f.sampled, [1, 2]);
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

  it('restores the recorded picture on the next playing request when selection disappears', async () => withLane(async f => {
    f.lane.supply(1, { dated: { h: 100, c: 101 } });
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    f.clock(100.5, 1);
    await f.request(100, 1);
    const exact = f.barWidth();
    await f.request(100, 1, 1, true);
    f.tick(0);
    assert.ok(f.barWidth() > exact);
    f.lane.supply(1, { dated: { h: 103, c: 103 } });
    f.clock(102.5, 1);
    f.tick(0);
    await f.request(102, 1);
    assert.equal(f.barWidth(), exact);
  }));

  it('paints ready artwork on the next tick despite a matching unacknowledged page entry', async () => withLane(async f => {
    f.record(100, { value: 100, art: true });
    f.clock(100, 1);
    await f.request(100, 1);
    assert.ok(!f.operations().some(op => op.op === 'drawImage'));
    f.painted.length = 0;
    f.finishArtwork();
    await f.drain();
    assert.deepEqual(f.painted, []);
    f.tick(0);
    assert.equal(f.operations().filter(op => op.op === 'drawImage').length, 1);
    await f.request(100, 1);
    assert.deepEqual(f.painted, [1]);
    assert.equal(f.operations().filter(op => op.op === 'drawImage').length, 1);
    assert.equal(typeof f.presented.at(-1)?.entries[0]?.fields.meta, 'object');
    await f.request(100, 1);
    assert.deepEqual(f.painted, [1]);
  }));

  it('keeps the later preview when artwork completes before an older page request', async () => withLane(async f => {
    f.record(100, { value: 30, art: true });
    f.record(101, { value: 60, art: true });
    f.record(102, { value: 100, art: true });
    f.clock(100.5, 1);
    await f.request(100, 1);
    const width = f.barWidth();
    f.clock(101.5, 1);
    f.tick(0);
    const preview = f.barWidth();
    assert.ok(Math.abs(preview / width - 80 / 30) < .01);
    const entries = f.presented.length;
    f.finishArtwork();
    await f.drain();
    assert.equal(f.presented.length, entries);
    await f.request(100, 1);
    assert.equal(f.barWidth(), preview);
    f.tick(0);
    assert.equal(f.barWidth(), preview);
    assert.ok(f.operations().some(op => op.op === 'drawImage'));
  }));

  it('publishes exact recorded metadata without replacing a later preview on an ordinary request', async () => withLane(async f => {
    f.record(100, { value: 30 });
    f.record(101, { value: 60 });
    f.record(102, { value: 100 });
    f.clock(100, 1);
    await f.request(100, 1);
    const width = f.barWidth();
    f.clock(101.5, 1);
    f.tick(0);
    const preview = f.barWidth();
    assert.ok(Math.abs(preview / width - 80 / 30) < .01);
    await f.request(101, 1);
    assert.equal(f.barWidth(), preview);
    const entry = f.presented.at(-1)?.entries[0];
    assert.equal(entry?.lo, 101);
    assert.match(JSON.stringify(entry?.fields.audit), /60/);
    assert.ok(!entry?.repainted);
    assert.equal(entry?.recordedPicture, true);
    assert.notEqual(entry?.content[0], f.presented[0]?.entries[0]?.content[0]);
  }));

  it('several expensive artwork wakes do no work between exact playing page requests', async () => withLane(async f => {
    f.mount(3, true);
    f.record(100, { value: 100, art: true });
    f.record(101, { value: 200, art: true });
    f.clock(100.5, 1);
    await f.request(100, 1, 1, false, true);
    f.selectionCost(11);
    f.painted.length = 0;
    f.finishArtwork();
    await f.drain();
    assert.deepEqual(f.painted, []);
    assert.equal(f.selections(), 0);
  }, 10));

  it('selects the first static picture on the next same-hour page publish after dated supply', async () => withLane(async f => {
    f.lane.mount(1, [], 'bars', {});
    f.bind();
    f.clock(100, 1);
    const page = new Page<TestBitmap>({ lanes: 1, seek: 1, post: (_lane, request) => {
      if (request.type === 'frame') f.lane.frame(request);
    } });
    page.mount(1, { route: 1, lane: 0, surfaces: [] });
    page.publish(100, { playing: true, speed: 1 });
    await f.drain();
    assert.deepEqual(f.painted, []);
    f.lane.supply(1, { dated: { h: 100, c: 100 } });
    await f.drain();
    assert.deepEqual(f.painted, []);
    page.publish(100);
    await f.drain();
    assert.deepEqual(f.painted, [1]);
    assert.equal(f.presented.at(-1)?.entries[0]?.lo, 100);
  }));

  it('includes the affected route reset with the first entry after artwork becomes ready', async () => withLane(async f => {
    f.record(100, { value: 100, art: true });
    f.clock(100, 1);
    await f.request(100, 1);
    const before = f.presented.length;
    f.lane.receive('follow', message(100, [], true, 9, 1), 1);
    f.finishArtwork();
    await f.drain();
    await f.request(100, 1);
    const first = f.presented.slice(before).find(p => p.entries.some(entry => entry.gen.reset === 1));
    assert.ok(first);
    assert.ok(first.resets?.some(([route, reset]) => route === 1 && reset === 1));
  }));

  it('does not serve a target bar or its dirty sibling between page requests', async () => withLane(async f => {
    f.mount(2, true);
    f.bind(1, 1, false, true);
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

  it('keeps eligible artwork and a dirty sibling pending for the next page request', async () => withLane(async f => {
    f.mount(2, true);
    f.record(100, { value: 100 });
    f.clock(100, 1);
    await f.request(100, 1);
    await f.request(100, 1, 1, true);
    f.painted.length = 0;
    f.lane.touch(2);
    f.lane.touch(1);
    await f.drain();
    assert.deepEqual(f.painted, []);
    await f.request(100, 1);
    assert.deepEqual(f.painted, [2, 1]);
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

  it('gives a ready pause its turn even when uncovered bars use the earlier budget slices', async () => withLane(async f => {
    f.mount(3);
    for (const id of [1, 2]) f.lane.supply(id, { dated: { h: 102, c: 102 } });
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    f.clock(100.5, 1);
    await f.request(100, 1);
    f.tick(0);
    assert.deepEqual(f.sampled, [3]);
    f.painted.length = 0;
    f.spendOnFactSelection(11);
    f.clock(101, 0);
    await f.request(101);
    for (let i = 0; i < 3; i++) {
      f.tick(16);
      await f.drain();
    }
    assert.deepEqual(f.painted, [3]);
    assert.ok(f.operations().some(op => op.op === 'fillText' && op.args[0] === '200'));
    const served = f.served.length;
    f.tick(16);
    await f.drain();
    assert.equal(f.served.length, served, 'uncovered bars retry when data or another request arrives');
  }, 10));

  it('gives a sibling its ordinary paint before selecting bars due on the first Play request', async () => withLane(async f => {
    f.mount(4, true);
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    await f.request(100);
    await f.request(100, 0, 1, true);
    f.clock(101.5, 1);
    f.lane.touch(4);
    f.painted.length = 0;
    f.selectionCost(6);
    await f.request(101, 1);
    assert.equal(f.painted[0], 4);
    assert.ok(f.selections() <= 2);
  }, 10));

  it('restores a deferred playback renderer even when a sibling fills each lane slice', async () => withLane(async f => {
    f.mount(2, true);
    f.record(100, { value: 100 });
    f.record(101, { value: 200 });
    f.clock(100, 1);
    await f.request(100, 1);
    const exact = f.barWidth();
    await f.request(100, 1, 1, true);
    f.lane.supply(1, { statics: {} });
    f.painted.length = 0;
    f.siblingCost(11);
    for (let i = 0; i < 4; i++) {
      f.lane.touch(2);
      await f.request(100, 1, 1, true);
    }
    assert.equal(f.painted[0], 2);
    assert.equal(f.painted.filter(id => id === 1).length, 1);
    f.clock(100.5, 1);
    f.tick(0);
    assert.ok(Math.abs(f.barWidth() / exact - 1.5) < .01);
  }, 10));

  it('draws without posting a playback message when performance recording is off', async () => withLane(async f => {
    f.record(100, { value: 30 });
    f.record(101, { value: 60 });
    f.clock(100, 1);
    await f.request(100, 1);
    const start = f.barWidth();
    const served = f.served.length;
    f.tick(500);
    assert.ok(f.barWidth() > start);
    assert.equal(f.served.length, served);
  }));

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
