import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { Lane, type Bitmapped, type LaneView } from '../draw/lane.ts';
import { contextWidgetKey } from '../draw/kit.ts';
import { PlaybackController } from '../dashboard/playback.ts';
import { Pipeline } from '../dashboard/follow/pipeline.ts';
import type { LaneWorker } from '../dashboard/follow/pipeline.ts';
import { datedPanel, datedRecord } from '../dashboard/follow/messages.fixture.ts';
import type { FamilyTheme } from './contract.ts';
import { createLaneView } from './laneRuntime.ts';
import { MotionField, interpolateFamilyValue, type MotionClock } from './motion.ts';
import { registerFamilyProjection } from './registry.ts';
import { barsLane, type FamilyBarsBinding } from './bars/laneView.ts';
import type { BarsModel } from './bars/model.ts';
import { tableLane, type FamilyTableBinding } from './table/laneView.ts';
import type { TableModel } from './table/model.ts';
import { tableDefaults } from './table/settings.ts';
import { timeChartLane } from './time-chart/laneView.ts';
import type { TimeChartBinding } from './time-chart/declaration.ts';
import type { TimeChartModel } from './time-chart/model.ts';
import { RecordedPath2D } from './time-chart/path2d.fixture.ts';
import { knownValue, zeroValue } from './value.ts';

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
  branch: 1,
  settleStartMs: 0,
  settleDurationMs: 0,
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

describe('MotionField invariants', () => {
  it('interpolates strictly within the two-state H0..H1 bracket while playing', () => {
    const field = new MotionField();
    const clock = baseClock({ anchorPosition: 100, anchorTimeMs: 1000, speed: 1 });
    const step1 = field.beginFrame(clock, 1500, 100, 101, false);
    assert.equal(step1.mode, 'playing');
    assert.ok(step1.bracketT !== null && Math.abs(step1.bracketT - 0.5) < 1e-6);
    const v = field.stepScalar('GER', 100, 200, false);
    assert.equal(v, 150);
    assert.equal(field.stats.bracketFrames, 1);
  });

  it('uses arrival chase when H1 is not yet available', () => {
    const field = new MotionField();
    const clock = baseClock({ anchorPosition: 100, anchorTimeMs: 1000, speed: 1440 });
    field.beginFrame(clock, 1000, 100, null, false);
    assert.equal(field.stepScalar('GER', 100, undefined, false), 100);
    field.beginFrame(clock, 1016.67, 124, null, false);
    const chased = field.stepScalar('GER', 200, undefined, false)!;
    assert.ok(chased > 100 && chased < 200, `expected chase between 100 and 200, got ${chased}`);
    assert.ok(field.stats.chaseFrames >= 2);
  });

  it('enforces gap guard on missing reading and never bridges notRecorded', () => {
    const field = new MotionField();
    const clock = baseClock({ anchorPosition: 100, anchorTimeMs: 1000, speed: 1 });
    field.beginFrame(clock, 1000, 100, 101, false);
    assert.equal(field.stepScalar('USA', 50, 100, false), 50);

    field.beginFrame(clock, 1500, 100, 101, false);
    const vMissingH1 = field.stepScalar('USA', 50, null, false);
    assert.equal(vMissingH1, 50);
    assert.ok(field.stats.gapSnaps >= 1);

    field.beginFrame(clock, 2000, 101, 102, false);
    const vMissingH0 = field.stepScalar('USA', null, 120, true);
    assert.equal(vMissingH0, null);
    assert.ok(field.stats.gapSnaps >= 2);

    field.beginFrame(clock, 2500, 102, 103, false);
    const vRecovered = field.stepScalar('USA', 500, undefined, false);
    assert.equal(vRecovered, 500);
  });

  it('enforces gap guard on branch change without bridging across branches', () => {
    const field = new MotionField();
    const c1 = baseClock({ anchorPosition: 100, anchorTimeMs: 1000, speed: 168, branch: 1 });
    field.beginFrame(c1, 1000, 100, null, false);
    assert.equal(field.stepScalar('SOV', 100, undefined, false), 100);
    assert.equal(field.stepY('y:SOV', 40), 40);

    const c2 = baseClock({ anchorPosition: 100, anchorTimeMs: 1016, speed: 168, branch: 2 });
    const step = field.beginFrame(c2, 1016, 100, null, false);
    assert.equal(step.cut, true);
    assert.equal(field.stepScalar('SOV', 999, undefined, false), 999);
    assert.equal(field.stepY('y:SOV', 200), 200);
    assert.ok(field.stats.gapSnaps >= 1);
  });

  it('settles smoothly on pause within 120 ms to exact recorded value', () => {
    const field = new MotionField();
    const playing = baseClock({ anchorPosition: 100, anchorTimeMs: 1000, speed: 1 });
    field.beginFrame(playing, 1500, 100, 101, false);
    assert.equal(field.stepScalar('ENG', 100, 200, false), 150);

    const paused = baseClock({
      playing: false,
      anchorPosition: 100.5,
      anchorTimeMs: 1500,
      speed: 0,
      exactHour: 100,
      settleStartMs: 1500,
      settleDurationMs: 120,
    });
    const mid = field.beginFrame(paused, 1560, 100, null, false);
    assert.equal(mid.mode, 'settling');
    const vMid = field.stepScalar('ENG', 100, undefined, false)!;
    assert.ok(vMid > 100 && vMid < 150, `expected ease between 150 and 100, got ${vMid}`);

    const end = field.beginFrame(paused, 1620, 100, null, false);
    assert.equal(end.mode, 'exact');
    assert.equal(end.justSettled, true);
    assert.equal(field.stepScalar('ENG', 100, undefined, false), 100);
    assert.equal(field.stats.settleCount, 1);
    assert.equal(field.stats.lastSettleMs, 120);
  });

  it('reports inactive when stationary and active while chasing', () => {
    const field = new MotionField();
    const clock = baseClock({ anchorPosition: 100, anchorTimeMs: 1000, speed: 1 });
    field.beginFrame(clock, 1000, 100, 101, false);
    field.stepScalar('GER', 100, 100, false);
    assert.equal(field.active, false);

    field.beginFrame(clock, 1016, 100, 101, false);
    assert.equal(field.active, false);

    field.beginFrame(clock, 1032, 100, null, false);
    field.stepScalar('GER', 200, undefined, false);
    assert.equal(field.active, true);
    field.beginFrame(clock, 1048, 100, null, false);
    assert.equal(field.active, true);
  });

  it('settles bars, table, and time-chart to a picture bit-identical to a cold read', async () => {
    const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
    Object.defineProperty(globalThis, 'OffscreenCanvas', {
      value: class {
        width: number;
        height: number;
        readonly __bpOps: readonly CanvasOp[];
        private readonly ctx: OffscreenCanvasRenderingContext2D;
        constructor(w: number, h: number) {
          this.width = w;
          this.height = h;
          const rec = makeRecordingCanvas(w, h);
          this.__bpOps = (rec.canvas as unknown as { __bpOps: readonly CanvasOp[] }).__bpOps;
          this.ctx = rec.canvas.getContext('2d')!;
        }
        getContext() { return this.ctx; }
      },
      configurable: true,
    });

    try {
      const barsKind = 'test-motion-exact-bars';
      const tableKind = 'test-motion-exact-table';
      const chartKind = 'test-motion-exact-chart';

      registerFamilyProjection({
        family: 'bars',
        kind: barsKind,
        project(view): BarsModel {
          const h = view.hour;
          const a = h >= 101 ? 180 : 100;
          const b = h >= 101 ? 120 : 160;
          const rows = a >= b
            ? [
              { kind: 'data' as const, key: 'GER', label: knownValue('Germany'), value: { value: knownValue(a), format: 'integer' as const }, numeric: a, color: '#4f86f7' },
              { kind: 'data' as const, key: 'USA', label: knownValue('United States'), value: { value: knownValue(b), format: 'integer' as const }, numeric: b, color: '#3fb950' },
            ]
            : [
              { kind: 'data' as const, key: 'USA', label: knownValue('United States'), value: { value: knownValue(b), format: 'integer' as const }, numeric: b, color: '#3fb950' },
              { kind: 'data' as const, key: 'GER', label: knownValue('Germany'), value: { value: knownValue(a), format: 'integer' as const }, numeric: a, color: '#4f86f7' },
            ];
          return { form: 'ordered', hour: h, nextChangeAt: h + 1, animateOrder: true, scaleFormat: 'integer', rows };
        },
      }, 'main');

      registerFamilyProjection({
        family: 'table',
        kind: tableKind,
        project(view): TableModel {
          const h = view.hour;
          const a = h >= 101 ? 240 : 120;
          const b = h >= 101 ? 150 : 210;
          return {
            hour: h,
            nextChangeAt: h + 1,
            columns: [
              { key: 'name', label: 'Country', as: 'name', priority: 0, offered: ['name'] },
              { key: 'score', label: 'Score', as: 'plain', numeric: true, format: 'integer', priority: 1, offered: ['plain'] },
            ],
            rows: [
              { key: 'GER', name: 'Germany', cells: { name: { value: knownValue('Germany') }, score: { value: knownValue(a) } } },
              { key: 'USA', name: 'United States', cells: { name: { value: knownValue('United States') }, score: { value: knownValue(b) } } },
            ],
          };
        },
      }, 'main');

      registerFamilyProjection({
        family: 'time-chart',
        kind: chartKind,
        project(view): TimeChartModel {
          const h = view.hour;
          const v1 = 100 + (h - 90) * 10;
          const v2 = 80 + (h - 90) * 6;
          return {
            kind: chartKind,
            measureTitle: 'Score',
            mode: 'lines',
            hour: h,
            extent: [90, 110],
            yAxis: { format: 'number', min: 0 },
            series: [
              {
                id: 'GER',
                key: 'GER',
                name: 'Germany',
                role: 'primary',
                color: '#4f86f7',
                points: [
                  { hour: 90, value: 100 },
                  { hour: 100, value: 200 },
                  ...(h >= 101 ? [{ hour: h, value: v1 }] : []),
                ],
                headValue: v1,
                headHour: h,
                endLabel: { name: 'Germany', value: String(v1), colorBar: '#4f86f7' },
              },
              {
                id: 'USA',
                key: 'USA',
                name: 'United States',
                role: 'primary',
                color: '#3fb950',
                points: [
                  { hour: 90, value: 80 },
                  { hour: 100, value: 140 },
                  ...(h >= 101 ? [{ hour: h, value: v2 }] : []),
                ],
                headValue: v2,
                headHour: h,
                endLabel: { name: 'United States', value: String(v2), colorBar: '#3fb950' },
              },
            ],
          };
        },
      }, 'main');

      const makeBindings = () => {
        const barsBinding: FamilyBarsBinding = {
          family: 'bars',
          kind: barsKind,
          view: 'main',
          options: {},
          sources: {},
          scrollTop: 0,
          width: 300,
          height: 220,
          theme: TEST_THEME,
        };
        const tableBinding: FamilyTableBinding = {
          family: 'table',
          kind: tableKind,
          view: 'main',
          options: {},
          sources: {},
          settings: tableDefaults(
            [
              { key: 'name', label: 'Country', as: 'name', offered: ['name'] },
              { key: 'score', label: 'Score', as: 'plain', offered: ['plain'] },
            ],
            { density: 'normal', sort: { column: 'score', direction: 'descending' } },
          ),
          scrollTop: 0,
          scrollLeft: 0,
          width: 300,
          height: 220,
          theme: TEST_THEME,
        };
        const chartBinding: TimeChartBinding = {
          family: 'time-chart',
          kind: chartKind,
          view: 'main',
          options: {},
          sources: {},
          scrollTop: 0,
          extent: [90, 110],
          width: 300,
          height: 220,
          theme: TEST_THEME,
        };
        return { barsBinding, tableBinding, chartBinding };
      };

      const errors: string[] = [];
      const mountHarness = (nowFn: () => number, laterQueue: (() => void)[]) => {
        const views = new Map<string, LaneView>();
        views.set('bars', createLaneView(barsLane, () => {}));
        views.set('table', createLaneView(tableLane, () => {}));
        views.set('time-chart', createLaneView(timeChartLane, () => {}));
        const lane = new Lane<Bitmapped & { close(): void }>({
          lane: 0,
          lanes: 1,
          views: name => views.get(name)!,
          crop: async () => [],
          present(p) {
            for (const entry of p.entries) {
              if (entry.mode === 'failed') errors.push(`present ${entry.widget}: ${entry.reason}`);
            }
          },
          release() {},
          fail(reason) { errors.push(`fail: ${reason}`); },
          failWidget(wid, reason) { errors.push(`widget ${wid}: ${reason}`); },
          later(fn) { laterQueue.push(fn); },
          clock: nowFn,
          budget: Infinity,
        });
        const rec = makeRecordingCanvas(960, 240);
        lane.setLaneSurface(rec.canvas, true);
        const { barsBinding, tableBinding, chartBinding } = makeBindings();
        lane.mount(1, [], 'bars', barsBinding);
        lane.mount(2, [], 'table', tableBinding);
        lane.mount(3, [], 'time-chart', chartBinding);
        lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 120 } });
        lane.bind(2, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 120 } });
        lane.bind(3, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 120 } });
        lane.setLaneSlots({
          width: 960,
          height: 240,
          dpr: 1,
          scrollTop: 0,
          scrollLeft: 0,
          surfaceColor: TEST_THEME.surface,
          clipPathFallback: false,
          slots: [
            { widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true },
            { widget: 2, x: 320, y: 0, w: 300, h: 220, visible: true },
            { widget: 3, x: 640, y: 0, w: 300, h: 220, visible: true },
          ],
        });
        return { lane, rec };
      };

      const flushMicrotasks = async () => {
        for (let i = 0; i < 6; i++) await Promise.resolve();
      };

      let coldNow = 1000;
      const coldLater: (() => void)[] = [];
      const cold = mountHarness(() => coldNow, coldLater);
      cold.rec.clearOps();
      cold.lane.frame({
        type: 'frame',
        serial: 1,
        seek: 1,
        target: 101,
        now: 101,
        exact: true,
        speed: 0,
        inspect: [],
        installed: [],
        refused: [],
      });
      await flushMicrotasks();
      assert.deepEqual(errors, []);
      const coldOps = cold.rec.snapshotOps();
      assert.equal(coldOps.length, 3);
      for (const ops of coldOps) assert.ok(ops.length > 0);

      let playNow = 1000;
      const playLater: (() => void)[] = [];
      const play = mountHarness(() => playNow, playLater);
      const origin = performance.timeOrigin;

      play.lane.setClock({
        playing: true,
        anchorPosition: 100,
        anchorTimeMs: origin + 1000,
        speed: 1,
        seek: 1,
        exactHour: 100,
        branch: 1,
        settleStartMs: 0,
        settleDurationMs: 0,
      });
      playLater.splice(0).forEach(fn => fn());

      playNow = 2016;
      play.rec.clearOps();
      playLater.splice(0).forEach(fn => fn());
      const chasingOps = play.rec.snapshotOps();
      assert.equal(chasingOps.length, 1);
      assert.notDeepEqual(chasingOps[0], coldOps[0]);

      playNow = 2050;
      play.lane.setClock({
        playing: false,
        anchorPosition: 101.05,
        anchorTimeMs: origin + 2050,
        speed: 0,
        seek: 1,
        exactHour: 101,
        branch: 1,
        settleStartMs: origin + 2050,
        settleDurationMs: 120,
      });
      play.lane.frame({
        type: 'frame',
        serial: 2,
        seek: 1,
        target: 101,
        now: 101,
        exact: true,
        speed: 0,
        inspect: [],
        installed: [],
        refused: [],
      });
      await flushMicrotasks();

      playNow = 2110;
      play.rec.clearOps();
      playLater.splice(0).forEach(fn => fn());
      const settlingOps = play.rec.snapshotOps();
      assert.equal(settlingOps.length, 1);
      assert.notDeepEqual(settlingOps[0], coldOps[0]);

      playNow = 2180;
      play.rec.clearOps();
      playLater.splice(0).forEach(fn => fn());
      play.rec.clearOps();
      await flushMicrotasks();
      const settledOps = play.rec.snapshotOps();
      assert.equal(settledOps.length, 1);
      assert.deepEqual(settledOps[0], coldOps[0]);
    } finally {
      Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
    }
  });

  it('clamps bracket blend inside [h0, h1] and shifts continuousHour by widget offset', () => {
    const field = new MotionField();
    const c0 = baseClock({ anchorPosition: 100, anchorTimeMs: 1000, speed: 1, exactHour: 100 });
    field.beginFrame(c0, 1000, 100, null, false);
    assert.equal(field.stepScalar('X', 0, undefined, false), 0);

    const step = field.beginFrame(c0, 1500, 100, 101, false);
    assert.equal(step.bracketT, 0.5);
    const val = field.stepScalar('X', 100, 200, false)!;
    assert.ok(val >= 100 && val <= 200, `expected value inside [100, 200], got ${val}`);

    const offsetField = new MotionField();
    const sOff = offsetField.beginFrame(c0, 1500, 110, 111, false, 10);
    assert.equal(sOff.continuousHour, 110.5);
    assert.ok(sOff.bracketT !== null && Math.abs(sOff.bracketT - 0.5) < 1e-6);
    const exactOff = offsetField.beginFrame({ ...c0, playing: false, exactHour: 100, settleDurationMs: 0 }, 1600, 110, null, false, 10);
    assert.equal(exactOff.continuousHour, 110);
  });

  it('keeps pinned widgets painted across shared-canvas resize during playback and updates live hits and paint failures', () => {
    const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
    Object.defineProperty(globalThis, 'OffscreenCanvas', {
      value: class {
        width: number;
        height: number;
        private readonly ctx: OffscreenCanvasRenderingContext2D;
        constructor(w: number, h: number) {
          this.width = w;
          this.height = h;
          this.ctx = makeRecordingCanvas(w, h).canvas.getContext('2d')!;
        }
        getContext() { return this.ctx; }
      },
      configurable: true,
    });

    try {
      const pinnedKind = 'test-motion-pinned-resize';
      const liveKind = 'test-motion-live-hit';
      registerFamilyProjection({
        family: 'bars',
        kind: pinnedKind,
        project(view): BarsModel {
          return {
            form: 'ordered',
            hour: view.hour,
            nextChangeAt: Infinity,
            scaleMax: 200,
            scaleFormat: 'integer',
            rows: [{ kind: 'data', key: 'PIN', label: knownValue('Pinned'), value: { value: knownValue(100), format: 'integer' }, numeric: 100, color: '#4f86f7' }],
          };
        },
      }, 'main');
      registerFamilyProjection({
        family: 'bars',
        kind: liveKind,
        project(view): BarsModel {
          const swapped = view.hour >= 101;
          const rows = swapped
            ? [
              { kind: 'data' as const, key: 'GER', label: knownValue('Germany'), value: { value: knownValue(180), format: 'integer' as const }, numeric: 180, color: '#4f86f7' },
              { kind: 'data' as const, key: 'USA', label: knownValue('United States'), value: { value: knownValue(120), format: 'integer' as const }, numeric: 120, color: '#3fb950' },
            ]
            : [
              { kind: 'data' as const, key: 'USA', label: knownValue('United States'), value: { value: knownValue(160), format: 'integer' as const }, numeric: 160, color: '#3fb950' },
              { kind: 'data' as const, key: 'GER', label: knownValue('Germany'), value: { value: knownValue(100), format: 'integer' as const }, numeric: 100, color: '#4f86f7' },
            ];
          return { form: 'ordered', hour: view.hour, nextChangeAt: view.hour + 1, scaleMax: 200, scaleFormat: 'integer', rows };
        },
      }, 'main');

      let now = 1000;
      const later: (() => void)[] = [];
      const hits: (string | null)[] = [];
      const failures: { widget: number; reason: string }[] = [];
      const views: Record<string, LaneView> = {
        bars: createLaneView(barsLane, () => {}),
        broken: {
          paint() { throw new Error('boom'); },
        },
      };
      const lane = new Lane<Bitmapped & { close(): void }>({
        lane: 0,
        lanes: 1,
        views: name => views[name]!,
        crop: async () => [],
        present() {},
        release() {},
        fail(reason) { throw new Error(reason); },
        failWidget(widget, reason) { failures.push({ widget, reason }); },
        later(fn) { later.push(fn); },
        hit(_widget, key) { hits.push(key); },
        clock: () => now,
        budget: Infinity,
      });

      const rec = makeRecordingCanvas(960, 300);
      lane.setLaneSurface(rec.canvas, true);
      const makeBinding = (kind: string, reducedMotion = false): FamilyBarsBinding => ({
        family: 'bars',
        kind,
        view: 'main',
        options: {},
        sources: {},
        scrollTop: 0,
        theme: TEST_THEME,
        width: 300,
        height: 220,
        reducedMotion,
      });
      lane.mount(1, [], 'bars', makeBinding(pinnedKind));
      lane.mount(2, [], 'bars', makeBinding(liveKind));
      lane.mount(3, [], 'bars', { family: 'bars', kind: 'unknown-broken-kind', view: 'main', options: {}, sources: {}, scrollTop: 0, theme: TEST_THEME, width: 300, height: 220 } satisfies FamilyBarsBinding);
      lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: 100, dated: { h: 100, c: 200 } });
      lane.bind(2, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } });
      lane.bind(3, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } });
      lane.setLaneSlots({
        width: 960,
        height: 240,
        dpr: 1,
        scrollTop: 0,
        scrollLeft: 0,
        originX: 0,
        originY: 0,
        surfaceColor: TEST_THEME.surface,
        clipPathFallback: false,
        slots: [
          { widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true },
          { widget: 2, x: 320, y: 0, w: 300, h: 220, visible: true },
          { widget: 3, x: 640, y: 0, w: 300, h: 220, visible: true },
        ],
      });

      const origin = performance.timeOrigin;
      lane.setClock({
        playing: true,
        anchorPosition: 100,
        anchorTimeMs: origin + 1000,
        speed: 1,
        seek: 1,
        exactHour: 100,
        branch: 1,
        settleStartMs: 0,
        settleDurationMs: 0,
      });
      rec.clearOps();
      later.splice(0).forEach(fn => fn());
      assert.equal(rec.snapshotOps().length, 1);

      lane.pointer(2, 0, [50, 18]);
      assert.equal(hits.at(-1), 'row:USA');

      now = 2100;
      lane.setLaneSlots({
        width: 980,
        height: 240,
        dpr: 1,
        scrollTop: 0,
        scrollLeft: 0,
        originX: 0,
        originY: 0,
        surfaceColor: TEST_THEME.surface,
        clipPathFallback: false,
        slots: [
          { widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true },
          { widget: 2, x: 320, y: 0, w: 300, h: 220, visible: true },
        ],
      });
      rec.clearOps();
      later.splice(0).forEach(fn => fn());
      assert.equal(rec.snapshotOps().length, 1);

      lane.pointer(2, 0, [50, 20]);
      assert.equal(hits.at(-1), 'row:GER');
      for (const t of [2200, 2300, 2400, 2500, 2600]) {
        now = t;
        later.splice(0).forEach(fn => fn());
      }
      lane.pointer(2, 0, [50, 18]);
      assert.equal(hits.at(-1), 'row:GER');
    } finally {
      Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
    }
  });

  it('round-2 regressions: empty-lane pause, fixed-point bar labels, historical time-chart headHour, and playback refusal', async () => {
    const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
    Object.defineProperty(globalThis, 'OffscreenCanvas', {
      value: class {
        width: number;
        height: number;
        private readonly ctx: OffscreenCanvasRenderingContext2D;
        constructor(w: number, h: number) {
          this.width = w;
          this.height = h;
          this.ctx = makeRecordingCanvas(w, h).canvas.getContext('2d')!;
        }
        getContext() { return this.ctx; }
      },
      configurable: true,
    });

    try {
      const fixedKind = 'test-motion-fixed-point-bars';
      const refuseKind = 'test-motion-refuse-live';
      const histChartKind = 'test-motion-hist-chart';
      registerFamilyProjection({
        family: 'bars',
        kind: fixedKind,
        project(view): BarsModel {
          return {
            form: 'ordered',
            hour: view.hour,
            nextChangeAt: view.hour + 1,
            scaleMax: 200,
            rows: [{
              kind: 'data',
              key: 'POOL',
              label: knownValue('Pool'),
              value: { value: knownValue(12345), format: { decimals: 2, fixedDecimals: true } },
              numeric: 123.45,
              color: '#4f86f7',
            }],
          };
        },
      }, 'main');
      registerFamilyProjection({
        family: 'bars',
        kind: refuseKind,
        project(): BarsModel {
          return { form: 'ordered', hour: 100, rows: [] };
        },
      }, 'main');
      registerFamilyProjection({
        family: 'time-chart',
        kind: histChartKind,
        project(view): TimeChartModel {
          return {
            kind: histChartKind,
            measureTitle: 'APM',
            mode: 'lines',
            hour: view.hour,
            extent: [1, 300],
            yAxis: { format: 'number', min: 0 },
            series: [{
              id: 'OLD',
              key: 'OLD',
              name: 'Old APM',
              role: 'primary',
              color: '#4f86f7',
              points: [{ hour: 10, value: 42 }],
              headHour: 10,
              headValue: 42,
              endLabel: { name: 'Old', value: '42', colorBar: '#4f86f7' },
            }],
          };
        },
      }, 'main');

      let now = 1000;
      const later: (() => void)[] = [];
      const presented: { widget: number; mode: string; reason?: string }[] = [];
      const views: Record<string, LaneView> = {
        bars: createLaneView(barsLane, () => {}),
        'time-chart': createLaneView(timeChartLane, () => {}),
        refusedView: {
          paint() {
            return { surfaces: [], fields: {}, motion: false, mode: 'refused', reason: 'not recorded here' };
          },
        },
      };
      const lane = new Lane<Bitmapped & { close(): void }>({
        lane: 0,
        lanes: 1,
        views: name => views[name]!,
        crop: async () => [],
        present(p) {
          for (const e of p.entries) presented.push({ widget: e.widget, mode: e.mode, reason: e.reason });
        },
        release() {},
        fail(reason) { throw new Error(reason); },
        later(fn) { later.push(fn); },
        clock: () => now,
        budget: Infinity,
      });

      const origin = performance.timeOrigin;
      lane.setClock({
        playing: true,
        anchorPosition: 100,
        anchorTimeMs: origin + 1000,
        speed: 1,
        seek: 1,
        exactHour: 100,
        branch: 1,
        settleStartMs: 0,
        settleDurationMs: 0,
      });
      lane.setClock({
        playing: false,
        anchorPosition: 100,
        anchorTimeMs: origin + 1050,
        speed: 0,
        seek: 1,
        exactHour: 100,
        branch: 1,
        settleStartMs: origin + 1050,
        settleDurationMs: 120,
      });

      const wc = makeRecordingCanvas(300, 220);
      lane.setCanvas(1, wc.canvas, true);
      lane.mount(1, [], 'bars', {
        family: 'bars',
        kind: fixedKind,
        view: 'main',
        options: {},
        sources: {},
        scrollTop: 0,
        theme: TEST_THEME,
        width: 300,
        height: 220,
      } satisfies FamilyBarsBinding);
      lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } });
      lane.frame({
        type: 'frame',
        serial: 1,
        seek: 1,
        target: 100,
        now: 100,
        exact: true,
        speed: 0,
        inspect: [],
        installed: [],
        refused: [],
      });
      for (let i = 0; i < 6; i++) await Promise.resolve();
      assert.equal(presented.some(e => e.widget === 1 && e.mode === 'picture'), true);

      const shared = makeRecordingCanvas(960, 260);
      lane.setLaneSurface(shared.canvas, true);
      lane.mount(2, [], 'refusedView', {});
      lane.bind(2, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } });
      lane.mount(3, [], 'time-chart', {
        family: 'time-chart',
        kind: histChartKind,
        view: 'main',
        options: {},
        sources: {},
        scrollTop: 0,
        theme: TEST_THEME,
        extent: [1, 300],
        width: 300,
        height: 220,
      } satisfies TimeChartBinding);
      lane.bind(3, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } });
      lane.setLaneSlots({
        width: 960,
        height: 260,
        dpr: 1,
        scrollTop: 0,
        scrollLeft: 0,
        originX: 0,
        originY: 0,
        surfaceColor: TEST_THEME.surface,
        clipPathFallback: false,
        slots: [
          { widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true },
          { widget: 2, x: 320, y: 0, w: 300, h: 220, visible: true },
          { widget: 3, x: 640, y: 0, w: 300, h: 220, visible: true },
        ],
      });

      now = 2000;
      lane.setClock({
        playing: true,
        anchorPosition: 100,
        anchorTimeMs: origin + 2000,
        speed: 1,
        seek: 1,
        exactHour: 100,
        branch: 1,
        settleStartMs: 0,
        settleDurationMs: 0,
      });
      shared.clearOps();
      later.splice(0).forEach(fn => fn());

      const barTexts = shared.snapshotOps()[0]!.filter(o => o.op === 'fillText').map(o => String(o.args[0]));
      assert.equal(barTexts.some(t => t.includes('123.45')), true, `expected 123.45 in ${barTexts.join(', ')}`);
      assert.equal(barTexts.some(t => t === '1.23'), false);
    } finally {
      Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
    }
  });

  it('broadcasts clock transitions to lazily created lanes', () => {
    const clock = new PlaybackController();
    clock.setBounds({ firstHour: 0, lastHour: 200 });
    clock.seek(10);
    const posted: { lane: number; type: string; playing?: boolean }[] = [];
    let nextIdx = 0;
    const client = {
      available: true,
      openFollow: async () => 'sess',
      product: async () => ({ data: { available: true, route: 1 } }),
    } as never;
    const pipeline = new Pipeline(clock, client, {
      lanes: [],
      laneCapacity: 2,
      createLane(): LaneWorker {
        const laneIndex = nextIdx++;
        return {
          onmessage: null,
          onerror: null,
          postMessage(msg) {
            if (msg.type === 'init') msg.port.close();
            if (msg.type === 'clock') posted.push({ lane: laneIndex, type: msg.type, playing: msg.clock.playing });
          },
        };
      },
      open: async () => 'sess',
      channel: () => new MessageChannel(),
      frameRate: () => 60,
      rate: () => 1,
      close() {},
    });
    try {
      pipeline.acquire('w1|0', { offset: 0, pin: null, preferredLane: 0 });
      pipeline.acquire('w2|0', { offset: 0, pin: null, preferredLane: 1 });
      posted.length = 0;
      clock.setPlaying(true);
      assert.deepEqual(posted, [
        { lane: 0, type: 'clock', playing: true },
        { lane: 1, type: 'clock', playing: true },
      ]);
      posted.length = 0;
      clock.setPlaying(false);
      assert.deepEqual(posted, [
        { lane: 0, type: 'clock', playing: false },
        { lane: 1, type: 'clock', playing: false },
      ]);
    } finally {
      clock.stop();
      pipeline.close();
    }
  });

  it('serves a widget hidden behind a loading overlay during pause settle without stranding it', async () => {
    const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
    Object.defineProperty(globalThis, 'OffscreenCanvas', {
      value: class {
        width: number; height: number;
        private readonly rec: ReturnType<typeof makeRecordingCanvas>;
        constructor(w: number, h: number) { this.width = w; this.height = h; this.rec = makeRecordingCanvas(w, h); }
        getContext() { return this.rec.canvas.getContext('2d'); }
      },
      configurable: true,
    });
    try {
      const kind = 'test-motion-overlay-settle';
      registerFamilyProjection({
        family: 'bars',
        kind,
        project(view): BarsModel {
          return {
            form: 'ordered',
            hour: view.hour,
            nextChangeAt: view.hour + 1,
            scaleMax: 300,
            rows: [{
              kind: 'data',
              key: 'GER',
              label: knownValue('Germany'),
              value: { value: knownValue(view.hour), format: 'integer' },
              color: '#c43e3e',
            }],
          };
        },
      }, 'main');
      const view: LaneView = createLaneView(barsLane, () => {});
      let now = 1000;
      const origin = performance.timeOrigin;
      const later: (() => void)[] = [];
      const presented: { widget: number; lo: number; binding: number }[] = [];
      const lane = new Lane<Bitmapped & { close(): void }>({
        lane: 0,
        lanes: 1,
        views: () => view,
        crop: async sources => sources.map(() => ({ width: 300, height: 220, close() {} })),
        present(p) {
          for (const e of p.entries) presented.push({ widget: e.widget, lo: e.lo, binding: e.binding });
        },
        release() {},
        fail(reason) { throw new Error(reason); },
        failWidget(_w, reason) { throw new Error(reason); },
        later(run) { later.push(run); },
        clock: () => now,
        budget: Infinity,
      });
      const shared = makeRecordingCanvas(640, 260);
      lane.setLaneSurface(shared.canvas, true);
      const binding = {
        family: 'bars',
        kind,
        view: 'main',
        options: {},
        sources: {},
        scrollTop: 0,
        theme: TEST_THEME,
        width: 300,
        height: 220,
      } satisfies FamilyBarsBinding;
      lane.mount(1, [], 'bars', binding);
      lane.mount(2, [], 'bars', binding);
      lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } });
      lane.bind(2, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } });
      lane.setLaneSlots({
        width: 640,
        height: 260,
        dpr: 1,
        scrollTop: 0,
        scrollLeft: 0,
        originX: 0,
        originY: 0,
        surfaceColor: TEST_THEME.surface,
        clipPathFallback: false,
        slots: [
          { widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true },
          { widget: 2, x: 320, y: 0, w: 300, h: 220, visible: true },
        ],
      });
      lane.setClock({
        playing: true,
        anchorPosition: 100,
        anchorTimeMs: origin + 1000,
        speed: 10,
        seek: 1,
        exactHour: 100,
        branch: 1,
        settleStartMs: 0,
        settleDurationMs: 0,
      });
      now = 1100;
      later.splice(0).forEach(fn => fn());

      lane.setClock({
        playing: false,
        anchorPosition: 101,
        anchorTimeMs: origin + 1100,
        speed: 0,
        seek: 1,
        exactHour: 101,
        branch: 1,
        settleStartMs: origin + 1100,
        settleDurationMs: 120,
      });
      lane.setLaneSlots({
        width: 640,
        height: 260,
        dpr: 1,
        scrollTop: 0,
        scrollLeft: 0,
        originX: 0,
        originY: 0,
        surfaceColor: TEST_THEME.surface,
        clipPathFallback: false,
        slots: [
          { widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true },
          { widget: 2, x: 320, y: 0, w: 300, h: 220, visible: false },
        ],
      });
      lane.bind(2, { generation: 2, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 101, c: 200 } });
      lane.frame({ type: 'frame', serial: 2, seek: 1, target: 101, now: 101, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
      for (let i = 0; i < 6; i++) await Promise.resolve();

      now = 1300;
      for (let round = 0; round < 6 && later.length; round++) {
        later.splice(0).forEach(fn => fn());
        for (let i = 0; i < 6; i++) await Promise.resolve();
      }

      assert.equal(presented.some(e => e.widget === 2 && e.binding === 2 && e.lo === 101), true, JSON.stringify(presented));
      assert.equal(presented.some(e => e.widget === 1 && e.lo === 101), true, JSON.stringify(presented));
    } finally {
      Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
    }
  });

  it('keeps serve semantics-only for playing bars without double-painting the shared canvas', async () => {
    const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
    Object.defineProperty(globalThis, 'OffscreenCanvas', {
      value: class {
        width: number; height: number;
        private readonly rec: ReturnType<typeof makeRecordingCanvas>;
        constructor(w: number, h: number) { this.width = w; this.height = h; this.rec = makeRecordingCanvas(w, h); }
        getContext() { return this.rec.canvas.getContext('2d'); }
      },
      configurable: true,
    });
    try {
      const kind = 'test-motion-no-double-paint-while-playing';
      registerFamilyProjection({
        family: 'bars',
        kind,
        project(view): BarsModel {
          return {
            form: 'ordered',
            hour: view.hour,
            nextChangeAt: view.hour + 1,
            scaleMax: 300,
            rows: [{
              kind: 'data',
              key: 'GER',
              label: knownValue('Germany'),
              value: { value: knownValue(view.hour), format: 'integer' },
              color: '#c43e3e',
              numeric: view.hour,
            }],
          };
        },
      }, 'main');
      const now = 1000;
      const later: (() => void)[] = [];
      const presented: { widget: number; lo: number; repainted?: boolean }[] = [];
      const view = createLaneView(barsLane, () => {});
      const lane = new Lane<Bitmapped & { close(): void }>({
        lane: 0,
        lanes: 1,
        views: () => view,
        crop: async () => [],
        present(p) {
          for (const e of p.entries) presented.push({ widget: e.widget, lo: e.lo, repainted: e.repainted });
        },
        release() {},
        fail(reason) { throw new Error(reason); },
        later(fn) { later.push(fn); },
        clock: () => now,
        budget: Infinity,
      });
      const shared = makeRecordingCanvas(340, 260);
      lane.setLaneSurface(shared.canvas, true);
      lane.mount(1, [], 'bars', {
        family: 'bars',
        kind,
        view: 'main',
        options: {},
        sources: {},
        scrollTop: 0,
        theme: TEST_THEME,
        width: 300,
        height: 220,
      } satisfies FamilyBarsBinding);
      lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } });
      lane.setLaneSlots({
        width: 340,
        height: 260,
        dpr: 1,
        scrollTop: 0,
        scrollLeft: 0,
        originX: 0,
        originY: 0,
        surfaceColor: TEST_THEME.surface,
        clipPathFallback: false,
        slots: [{ widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true }],
      });
      lane.frame({ type: 'frame', serial: 1, seek: 1, target: 100, now: 100, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
      for (let i = 0; i < 6; i++) await Promise.resolve();
      assert.equal(presented.length, 1);

      presented.length = 0;
      const origin = performance.timeOrigin;
      lane.setClock({
        playing: true,
        anchorPosition: 120,
        anchorTimeMs: origin + 1000,
        speed: 1440,
        seek: 1,
        exactHour: 120,
        branch: 1,
        settleStartMs: 0,
        settleDurationMs: 0,
      });
      shared.clearOps();
      lane.frame({ type: 'frame', serial: 2, seek: 1, target: 120, now: 120, exact: false, speed: 1440, inspect: [], installed: [], refused: [] });
      for (let i = 0; i < 6; i++) await Promise.resolve();

      assert.equal(shared.snapshotOps().length, 0, 'serve must not paint pixels of a playing bars widget');
      assert.equal(presented.every(e => e.repainted !== true), true, 'semantics-only serve entry must not mark repainted');
    } finally {
      Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
    }
  });
});

describe('Motion rules A-I', () => {
  function withFakeOffscreen<T>(run: () => T): T {
    const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
    Object.defineProperty(globalThis, 'OffscreenCanvas', {
      value: class {
        width: number;
        height: number;
        private readonly ctx: OffscreenCanvasRenderingContext2D;
        constructor(w: number, h: number) {
          this.width = w;
          this.height = h;
          this.ctx = makeRecordingCanvas(w, h).canvas.getContext('2d')!;
        }
        getContext() { return this.ctx; }
      },
      configurable: true,
    });
    const finish = () => {
      Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
    };
    try {
      const result = run();
      const maybePromise = result as unknown as Promise<unknown> | null;
      if (maybePromise && typeof maybePromise.then === 'function') {
        return maybePromise.finally(finish) as unknown as T;
      }
      finish();
      return result;
    } catch (error) {
      finish();
      throw error;
    }
  }

  it('Rule A: widget draw position is capped at confirmed coverage when the stream is behind', () => withFakeOffscreen(() => {
    const seenHours: { hour: number; continuousHour: number | undefined }[] = [];
    let now = 1000;
    const later: (() => void)[] = [];
    const probeView: LaneView = {
      paint(input) {
        seenHours.push({ hour: input.hour, continuousHour: input.continuousHour });
        return { surfaces: [], fields: {}, motion: false, repainted: true };
      },
    };
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => probeView,
      crop: async () => [],
      present() {},
      release() {},
      fail(reason) { throw new Error(reason); },
      later(fn) { later.push(fn); },
      clock: () => now,
      budget: Infinity,
    });
    const rec = makeRecordingCanvas(600, 260);
    lane.setLaneSurface(rec.canvas, true);
    lane.follow(1, 'panel', 'p');
    lane.bindRoute(1, 1, 9, 1, 0);
    lane.mount(1, [1], 'bars', {});
    lane.bind(1, { generation: 1, surfaces: [{ width: 560, height: 220, dpr: 1 }], offset: 0, pin: null });
    lane.setLaneSlots({
      width: 600,
      height: 260,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 0, y: 0, w: 560, h: 220, visible: true }],
    });

    lane.receive('panels', datedPanel(1, 24, { score: 100 }, 1, 9));
    const origin = performance.timeOrigin;
    lane.setClock({
      playing: true,
      anchorPosition: 24,
      anchorTimeMs: origin + 1000,
      speed: 24,
      seek: 1,
      exactHour: 24,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 200,
    });

    now = 3000;
    later.splice(0).forEach(fn => fn());
    assert.ok(seenHours.length > 0, 'expected widget paint call');
    const last = seenHours.at(-1)!;
    assert.equal(last.hour, 24, `expected hour 24, got ${last.hour}`);
    assert.equal(last.continuousHour, 24, `draw position must be capped at confirmed coverage 24 when stream is behind, got ${last.continuousHour}`);
  }));

  it('Rule B: value confirmed through 109 stays flat until 109, moves only across 109..110, and never brackets a carried picture', () => withFakeOffscreen(() => {
    const kind = 'test-rule-b-bracket';
    registerFamilyProjection({
      family: 'bars',
      kind,
      project(view): BarsModel {
        const val = Number((view.panel('s') as { v?: number } | undefined)?.v ?? 0);
        return {
          form: 'ordered',
          hour: view.hour,
          showAxis: false,
          scaleMax: 300,
          scaleFormat: 'integer',
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            value: { value: knownValue(val), format: 'integer' },
            numeric: val,
            color: '#4f86f7',
          }],
        };
      },
    }, 'main');

    let now = 1000;
    const later: (() => void)[] = [];
    const view = createLaneView(barsLane, () => {});
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present() {},
      release() {},
      fail(reason) { throw new Error(reason); },
      later(fn) { later.push(fn); },
      clock: () => now,
      budget: Infinity,
    });
    const rec = makeRecordingCanvas(340, 240);
    lane.setLaneSurface(rec.canvas, true);
    lane.follow(1, 'panel', 'p');
    lane.bindRoute(1, 1, 9, 1, 0);
    lane.mount(1, [1], 'bars', {
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      sources: { s: 1 },
      scrollTop: 0,
      width: 300,
      height: 220,
      theme: TEST_THEME,
    } satisfies FamilyBarsBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null });
    lane.setLaneSlots({
      width: 340,
      height: 240,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true }],
    });

    lane.receive('panels', datedPanel(1, 100, { v: 100 }, 1, 9));
    lane.receive('panels', datedPanel(1, 109, undefined, 1, 9));
    lane.receive('panels', datedPanel(1, 110, { v: 200 }, 1, 9));

    const origin = performance.timeOrigin;
    lane.setClock({
      playing: true,
      anchorPosition: 100,
      anchorTimeMs: origin + 1000,
      speed: 1,
      seek: 1,
      exactHour: 100,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 120,
    });

    now = 6000;
    rec.clearOps();
    later.splice(0).forEach(fn => fn());
    const at105 = rec.snapshotOps()[0]!.filter(o => o.op === 'fillText').map(o => String(o.args[0]));
    assert.equal(at105.includes('100'), true, `expected flat 100 at hour 105 (confirmed through 109), got ${at105.join(', ')}`);

    now = 10500;
    rec.clearOps();
    later.splice(0).forEach(fn => fn());
    const at1095 = rec.snapshotOps()[0]!.filter(o => o.op === 'fillText').map(o => String(o.args[0]));
    assert.equal(at1095.includes('100'), false, `expected motion across 109..110, got ${at1095.join(', ')}`);
    assert.equal(at1095.includes('200'), false, `expected intermediate value across 109..110, got ${at1095.join(', ')}`);

    lane.stream(1, 2, 105);
    lane.receive('panels', datedPanel(1, 110, { v: 260 }, 2, 9));
    now = 10500;
    lane.setClock({
      playing: true,
      anchorPosition: 109.5,
      anchorTimeMs: origin + 10500,
      speed: 1,
      seek: 2,
      exactHour: 109,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 120,
    });
    rec.clearOps();
    later.splice(0).forEach(fn => fn());
    const carriedTexts = (rec.snapshotOps()[0] ?? []).filter(o => o.op === 'fillText').map(o => String(o.args[0]));
    if (carriedTexts.length > 0) {
      assert.equal(carriedTexts.includes('100'), true, `carried picture from seek 1 must not bracket with seek 2: ${carriedTexts.join(', ')}`);
    }
  }));

  it('Rule C: known, missing, known with middle hour skipped cuts and real branch identity cuts without relying on bounds', () => withFakeOffscreen(() => {
    const kind = 'test-rule-c-cut';
    registerFamilyProjection({
      family: 'bars',
      kind,
      project(view): BarsModel {
        const val = Number((view.panel('s') as { v?: number } | undefined)?.v ?? 0);
        return {
          form: 'ordered',
          hour: view.hour,
          scaleMax: 600,
          scaleFormat: 'integer',
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            value: { value: knownValue(val), format: 'integer' },
            numeric: val,
            color: '#4f86f7',
          }],
        };
      },
    }, 'main');

    let now = 1000;
    const later: (() => void)[] = [];
    const view = createLaneView(barsLane, () => {});
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present() {},
      release() {},
      fail(reason) { throw new Error(reason); },
      later(fn) { later.push(fn); },
      clock: () => now,
      budget: Infinity,
    });
    const rec = makeRecordingCanvas(340, 240);
    lane.setLaneSurface(rec.canvas, true);
    lane.follow(1, 'panel', 'p');
    lane.bindRoute(1, 1, 9, 1, 0);
    lane.mount(1, [1], 'bars', {
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      sources: { s: 1 },
      scrollTop: 0,
      width: 300,
      height: 220,
      theme: TEST_THEME,
    } satisfies FamilyBarsBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null });
    lane.setLaneSlots({
      width: 340,
      height: 240,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true }],
    });

    lane.receive('panels', datedPanel(1, 100, { v: 100 }, 1, 9));
    lane.receive('panels', datedRecord(1, 101, 0, 'not recorded at 101', 1, 9));
    lane.receive('panels', datedPanel(1, 102, { v: 500 }, 1, 9));

    const origin = performance.timeOrigin;
    lane.setClock({
      playing: true,
      anchorPosition: 100,
      anchorTimeMs: origin + 1000,
      speed: 200,
      seek: 1,
      exactHour: 100,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 110,
    });
    later.splice(0).forEach(fn => fn());

    now = 1010;
    rec.clearOps();
    later.splice(0).forEach(fn => fn());
    const at102 = rec.snapshotOps()[0]!.filter(o => o.op === 'fillText').map(o => String(o.args[0]));
    assert.equal(at102.includes('500'), true, `expected immediate cut to 500 across skipped missing hour 101, got ${at102.join(', ')}`);

    const clock = new PlaybackController();
    clock.setBounds({ firstHour: 0, lastHour: 200 });
    const clocks: MotionClock[] = [];
    const client = {
      available: true,
      openFollow: async () => 'sess',
      product: async () => ({ data: { available: true, route: 1 } }),
    } as never;
    const pipeline = new Pipeline(clock, client, {
      lanes: [{
        onmessage: null,
        onerror: null,
        postMessage(msg) {
          if (msg.type === 'init') msg.port.close();
          if (msg.type === 'clock') clocks.push(msg.clock);
        },
      }],
      open: async () => 'sess',
      channel: () => new MessageChannel(),
      frameRate: () => 60,
      rate: () => 1,
      close() {},
    });
    try {
      const source = { id: 'src', revision: '1', label: 's' } as never;
      const w = pipeline.mount({ offset: 0, pin: null });
      const tok = {};
      pipeline.subscribe(w, tok, source, 'branch-A', 0, { product: 'p' } as never, 'followed');
      clock.setPlaying(true);
      const beforeGrow = clocks.at(-1)!.branch;
      clock.setBounds({ firstHour: 0, lastHour: 201 }, false);
      clock.setPlaying(true);
      const afterGrow = clocks.at(-1)!.branch;
      assert.equal(afterGrow, beforeGrow, 'live bounds growth on the same branch must not cut motion');
      pipeline.subscribe(w, tok, source, 'branch-B', 0, { product: 'p' } as never, 'followed');
      const afterSwitch = clocks.at(-1)!.branch;
      assert.notEqual(afterSwitch, beforeGrow, 'switching real branch identity with identical bounds must cut motion');
    } finally {
      clock.stop();
      pipeline.close();
    }
  }));

  it('Rule D: missing state and inspection during playback reach the page with no paint', async () => withFakeOffscreen(async () => {
    const kind = 'test-rule-d-semantics';
    registerFamilyProjection({
      family: 'bars',
      kind,
      project(view): BarsModel {
        const raw = view.panel('s') as { empty?: string; v?: number } | undefined;
        if (raw?.empty) {
          return { form: 'ordered', hour: view.hour, rows: [], empty: raw.empty, emptyKind: 'unavailable' };
        }
        const v = raw?.v ?? 100;
        return {
          form: 'ordered',
          hour: view.hour,
          showAxis: false,
          scaleMax: 300,
          scaleFormat: 'integer',
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            value: { value: knownValue(v), format: 'integer' },
            numeric: v,
            color: '#4f86f7',
          }],
        };
      },
    }, 'main');

    let now = 1000;
    const later: (() => void)[] = [];
    const clock = new PlaybackController();
    clock.setBounds({ firstHour: 100, lastHour: 200 });
    clock.seek(100);
    const view = createLaneView(barsLane, () => {});
    let lane!: Lane<Bitmapped & { close(): void }>;
    let worker!: LaneWorker;
    const client = {
      available: true,
      openFollow: async () => 'sess',
      product: async () => ({ data: { available: true, route: 1 } }),
    } as never;
    const pipeline = new Pipeline(clock, client, {
      lanes: [{
        onmessage: null,
        onerror: null,
        postMessage(msg) {
          if (msg.type === 'init') {
            msg.port.close();
            lane = new Lane<Bitmapped & { close(): void }>({
              lane: 0,
              lanes: 1,
              views: () => view,
              crop: async () => [],
              present(p) {
                worker.onmessage?.({ data: { type: 'presentation', ...p } } as never);
              },
              release() {},
              fail(reason) { throw new Error(reason); },
              later(fn) { later.push(fn); },
              clock: () => now,
              budget: Infinity,
            });
            return;
          }
          if (msg.type === 'mount') lane.mount(msg.widget, msg.subs, msg.view, msg.model);
          else if (msg.type === 'bind') lane.bind(msg.widget, msg.binding, msg.model, msg.retry, msg.remounted);
          else if (msg.type === 'clock') lane.setClock(msg.clock);
          else if (msg.type === 'frame') lane.frame(msg);
          else if (msg.type === 'lane-surface') lane.setLaneSurface(msg.canvas, msg.opaque);
          else if (msg.type === 'lane-slots') lane.setLaneSlots(msg);
        },
      }],
      open: async () => 'sess',
      channel: () => new MessageChannel(),
      frameRate: () => 60,
      rate: () => 1,
      close() {},
    });
    worker = (pipeline as unknown as { workers: LaneWorker[] }).workers[0]!;
    try {
      const rec = makeRecordingCanvas(340, 240);
      pipeline.transferLaneSurface(0, rec.canvas, true);
      const wid = pipeline.mount({
        offset: 0,
        pin: null,
        view: 'bars',
        model: {
          family: 'bars',
          kind,
          view: 'main',
          options: {},
          sources: { s: 1 },
          scrollTop: 0,
          width: 300,
          height: 220,
          theme: TEST_THEME,
        } satisfies FamilyBarsBinding,
      });
      lane.follow(1, 'panel', 'p');
      lane.bindRoute(1, 1, 9, clock.getSnapshot().seekSequence, 0);
      lane.mount(wid, [1], 'bars', {
        family: 'bars',
        kind,
        view: 'main',
        options: {},
        sources: { s: 1 },
        scrollTop: 0,
        width: 300,
        height: 220,
        theme: TEST_THEME,
      });
      pipeline.bind(wid, [], { surfaces: [{ width: 300, height: 220, dpr: 1 }] });
      pipeline.sendLaneSlots(0, {
        width: 340,
        height: 240,
        dpr: 1,
        scrollTop: 0,
        scrollLeft: 0,
        surfaceColor: TEST_THEME.surface,
        clipPathFallback: false,
        slots: [{ widget: wid, x: 0, y: 0, w: 300, h: 220, visible: true }],
      });
      lane.receive('panels', datedPanel(1, 100, { v: 150 }, clock.getSnapshot().seekSequence, 9));
      for (let i = 0; i < 6; i++) await Promise.resolve();
      pipeline.page.frame();

      clock.setPlaying(true);
      pipeline.inspect(wid, 'row:GER');
      for (let i = 0; i < 6; i++) await Promise.resolve();
      pipeline.page.frame();
      assert.ok(pipeline.view(wid)?.tooltip !== null, 'inspection tooltip must be served during playback');

      lane.receive('panels', datedPanel(1, 100, { empty: 'Not recorded in this save' }, clock.getSnapshot().seekSequence, 9));
      for (let i = 0; i < 6; i++) await Promise.resolve();
      pipeline.page.frame();
      const meta = pipeline.view(wid)?.fields?.meta as { empty?: string | null } | undefined;
      assert.equal(meta?.empty, 'Not recorded in this save', 'empty/missing state arriving during playback must reach the page without a paint');
    } finally {
      clock.stop();
      pipeline.close();
    }
  }));

  it('Rule E: hidden overlay slot returns credit and shows its reading, empty lane services facts, and late pause finds its exact fact', async () => withFakeOffscreen(async () => {
    const kind = 'test-rule-e-bookkeeping';
    registerFamilyProjection({
      family: 'bars',
      kind,
      project(view): BarsModel {
        const v = Number((view.panel('s') as { v?: number } | undefined)?.v ?? 0);
        return {
          form: 'ordered',
          hour: view.hour,
          showAxis: false,
          scaleMax: 300,
          scaleFormat: 'integer',
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            value: { value: knownValue(v), format: 'integer' },
            numeric: v,
            color: '#4f86f7',
          }],
        };
      },
    }, 'main');

    let now = 1000;
    const later: (() => void)[] = [];
    let released = 0;
    const presented: { widget: number; lo: number; mode: string; id: number; binding: number }[] = [];
    const view = createLaneView(barsLane, () => {});
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async sources => sources.map(() => ({ width: 300, height: 220, close() {} })),
      present(p) {
        for (const e of p.entries) presented.push({ widget: e.widget, lo: e.lo, mode: e.mode, id: e.id, binding: e.binding });
      },
      release(bytes) { released += bytes; },
      fail(reason) { throw new Error(reason); },
      later(fn) { later.push(fn); },
      clock: () => now,
      budget: Infinity,
    });
    const shared = makeRecordingCanvas(640, 240);
    lane.setLaneSurface(shared.canvas, true);
    lane.follow(1, 'panel', 'p1');
    lane.bindRoute(1, 1, 9, 1, 0);
    lane.follow(2, 'panel', 'p2');
    lane.bindRoute(2, 1, 9, 1, 0);
    const makeBinding = (sub: number): FamilyBarsBinding => ({
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      sources: { s: sub },
      scrollTop: 0,
      width: 300,
      height: 220,
      theme: TEST_THEME,
    });
    lane.mount(1, [1], 'bars', makeBinding(1));
    lane.mount(2, [2], 'bars', makeBinding(2));
    lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null });
    lane.bind(2, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null });
    lane.setLaneSlots({
      width: 640,
      height: 240,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [
        { widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true },
        { widget: 2, x: 320, y: 0, w: 300, h: 220, visible: false },
      ],
    });

    lane.receive('panels', datedPanel(1, 100, { v: 100 }, 1, 9));
    lane.receive('panels', datedPanel(1, 104, undefined, 1, 9));
    lane.receive('panels', datedPanel(1, 105, { v: 200 }, 1, 9));
    lane.receive('panels', datedPanel(1, 107, { v: 250 }, 1, 9));
    lane.receive('panels', datedPanel(1, 110, undefined, 1, 9));
    lane.receive('panels', datedPanel(2, 100, { v: 50 }, 1, 9));
    lane.receive('panels', datedPanel(2, 105, { v: 75 }, 1, 9));
    lane.receive('panels', datedPanel(2, 107, { v: 90 }, 1, 9));

    const origin = performance.timeOrigin;
    lane.setClock({
      playing: true,
      anchorPosition: 100,
      anchorTimeMs: origin + 1000,
      speed: 20,
      seek: 1,
      exactHour: 100,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 110,
    });
    lane.frame({ type: 'frame', serial: 1, seek: 1, target: 100, now: 100, exact: false, speed: 20, inspect: [], installed: [], refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    assert.equal(presented.some(e => e.widget === 2 && e.mode === 'picture'), true, 'hidden overlay slot must still show its reading when it arrives');

    now = 1400;
    later.splice(0).forEach(fn => fn());

    lane.setClock({
      playing: false,
      anchorPosition: 102,
      anchorTimeMs: origin + 1400,
      speed: 0,
      seek: 1,
      exactHour: 102,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 110,
    });
    const acks = presented.map(e => ({ widget: e.widget, binding: e.binding, id: e.id }));
    presented.length = 0;
    shared.clearOps();
    lane.frame({ type: 'frame', serial: 2, seek: 1, target: 102, now: 102, exact: true, speed: 0, inspect: [], installed: acks, refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    const pauseTexts = (shared.snapshotOps()[0] ?? []).filter(o => o.op === 'fillText').map(o => String(o.args[0]));
    assert.equal(pauseTexts.includes('100'), true, `late pause at hour 102 must still find its exact fact (100), got ${pauseTexts.join(', ')}`);

    const pauseAcks = presented.map(e => ({ widget: e.widget, binding: e.binding, id: e.id }));
    lane.setLaneSlots({
      width: 640,
      height: 240,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [],
    });
    released = 0;
    lane.frame({ type: 'frame', serial: 3, seek: 1, target: 108, now: 108, exact: false, speed: 20, inspect: [], installed: pauseAcks, refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    const finalAcks = presented.slice(pauseAcks.length).map(e => ({ widget: e.widget, binding: e.binding, id: e.id }));
    lane.frame({ type: 'frame', serial: 4, seek: 1, target: 108, now: 108, exact: false, speed: 20, inspect: [], installed: finalAcks, refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    assert.ok(released > 0, 'hidden slot and empty lane must return send credit when page advances');
  }));

  it('Rule F: seek during settle ends exact and pending animation callback cannot swallow an exact request', async () => withFakeOffscreen(async () => {
    const kind = 'test-rule-f-exact';
    registerFamilyProjection({
      family: 'bars',
      kind,
      project(view): BarsModel {
        const v = view.hour * 10;
        return {
          form: 'ordered',
          hour: view.hour,
          nextChangeAt: view.hour + 1,
          scaleMax: 2000,
          scaleFormat: 'integer',
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            value: { value: knownValue(v), format: 'integer' },
            numeric: v,
            color: '#4f86f7',
          }],
        };
      },
    }, 'main');

    let now = 1000;
    const later: (() => void)[] = [];
    const presented: { widget: number; lo: number; seek: number }[] = [];
    const view = createLaneView(barsLane, () => {});
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present(p) {
        for (const e of p.entries) presented.push({ widget: e.widget, lo: e.lo, seek: e.gen.seek });
      },
      release() {},
      fail(reason) { throw new Error(reason); },
      later(fn) { later.push(fn); },
      clock: () => now,
      budget: Infinity,
    });
    const shared = makeRecordingCanvas(340, 240);
    lane.setLaneSurface(shared.canvas, true);
    lane.mount(1, [], 'bars', {
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      sources: {},
      scrollTop: 0,
      width: 300,
      height: 220,
      theme: TEST_THEME,
    } satisfies FamilyBarsBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 200 } });
    lane.setLaneSlots({
      width: 340,
      height: 240,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true }],
    });

    const origin = performance.timeOrigin;
    lane.setClock({
      playing: true,
      anchorPosition: 100,
      anchorTimeMs: origin + 1000,
      speed: 10,
      seek: 1,
      exactHour: 100,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 200,
    });
    now = 1500;
    later.splice(0).forEach(fn => fn());

    lane.setClock({
      playing: false,
      anchorPosition: 105,
      anchorTimeMs: origin + 1500,
      speed: 0,
      seek: 1,
      exactHour: 105,
      branch: 1,
      settleStartMs: origin + 1500,
      settleDurationMs: 120,
      lastHour: 200,
    });
    assert.ok(later.length > 0, 'settle callback should be queued');

    now = 1540;
    presented.length = 0;
    shared.clearOps();
    lane.setClock({
      playing: false,
      anchorPosition: 101,
      anchorTimeMs: origin + 1540,
      speed: 0,
      seek: 2,
      exactHour: 101,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 200,
    });
    lane.frame({ type: 'frame', serial: 2, seek: 2, target: 101, now: 101, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    assert.equal(presented.some(e => e.widget === 1 && e.lo === 101), true, 'seek during settle must present exact hour 101 immediately');

    shared.clearOps();
    later.splice(0).forEach(fn => fn());
    for (let i = 0; i < 6; i++) await Promise.resolve();
    const afterStaleRaf = (shared.snapshotOps()[0] ?? []).filter(o => o.op === 'fillText').map(o => String(o.args[0]));
    if (afterStaleRaf.length > 0) {
      assert.equal(afterStaleRaf.includes('1,010'), true, `pending rAF must not overwrite sought hour 101: ${afterStaleRaf.join(', ')}`);
    }
  }));

  it('Rule G: worker ticks continue with main thread blocked and stop when nothing moves', () => withFakeOffscreen(() => {
    const kind = 'test-rule-g-worker-frames';
    registerFamilyProjection({
      family: 'bars',
      kind,
      project(view): BarsModel {
        const v = Math.min(105, view.hour);
        return {
          form: 'ordered',
          hour: view.hour,
          nextChangeAt: view.hour < 105 ? view.hour + 1 : Infinity,
          scaleMax: 200,
          scaleFormat: 'integer',
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            value: { value: knownValue(v), format: 'integer' },
            numeric: v,
            color: '#4f86f7',
          }],
        };
      },
    }, 'main');

    let now = 1000;
    const later: (() => void)[] = [];
    let ticks = 0;
    const view = createLaneView(barsLane, () => {});
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present() {},
      served(frame) { if (!frame.exact && frame.changed > 0) ticks++; },
      release() {},
      fail(reason) { throw new Error(reason); },
      later(fn) { later.push(fn); },
      clock: () => now,
      budget: Infinity,
    });
    const shared = makeRecordingCanvas(340, 240);
    lane.setLaneSurface(shared.canvas, true);
    lane.mount(1, [], 'bars', {
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      sources: {},
      scrollTop: 0,
      width: 300,
      height: 220,
      theme: TEST_THEME,
    } satisfies FamilyBarsBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 105 } });
    lane.setLaneSlots({
      width: 340,
      height: 240,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true }],
    });

    const origin = performance.timeOrigin;
    lane.setClock({
      playing: true,
      anchorPosition: 100,
      anchorTimeMs: origin + 1000,
      speed: 24,
      seek: 1,
      exactHour: 100,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 105,
    });
    for (let step = 1; step <= 4; step++) {
      now = 1000 + step * 50;
      later.splice(0).forEach(fn => fn());
    }
    assert.ok(ticks >= 3, `worker ticks must continue without any main-thread ack, got ${ticks}`);

    for (let step = 0; step < 30 && later.length > 0; step++) {
      now += 100;
      later.splice(0).forEach(fn => fn());
    }
    assert.equal(later.length, 0, 'worker animation loop must stop when nothing moves');
  }));

  it('Rule H: every form of bars moves (ordered+target, net+pair, share, stacked, two-sided paired/split-sides/stacked-pair)', () => withFakeOffscreen(() => {
    const pairedKind = 'test-rule-h-bars-paired';
    const splitKind = 'test-rule-h-bars-split';
    const stackedPairKind = 'test-rule-h-bars-stacked-pair';
    const shareKind = 'test-rule-h-bars-share';
    const targetKind = 'test-rule-h-bars-target';

    registerFamilyProjection({
      family: 'bars',
      kind: pairedKind,
      project(view): BarsModel {
        const l = view.hour >= 101 ? 80 : 20;
        const r = view.hour >= 101 ? 200 : 100;
        return {
          form: 'two-sided',
          twoSidedLayout: 'paired',
          hour: view.hour,
          nextChangeAt: view.hour + 1,
          scaleLeftMax: 100,
          scaleRightMax: 200,
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            left: { value: { value: knownValue(l), format: 'integer' }, numeric: l, color: '#4f86f7' },
            right: { value: { value: knownValue(r), format: 'integer' }, numeric: r, color: '#3fb950' },
          }],
        };
      },
    }, 'main');

    registerFamilyProjection({
      family: 'bars',
      kind: splitKind,
      project(view): BarsModel {
        const l = view.hour >= 101 ? 90 : 30;
        const r = view.hour >= 101 ? 180 : 60;
        return {
          form: 'two-sided',
          twoSidedLayout: 'split-sides',
          hour: view.hour,
          nextChangeAt: view.hour + 1,
          scaleLeftMax: 100,
          scaleRightMax: 200,
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            value: { value: knownValue(l), format: 'integer' },
            numeric: l,
            color: '#4f86f7',
            rightSide: {
              key: 'SOV',
              label: knownValue('Soviet Union'),
              value: { value: knownValue(r), format: 'integer' },
              numeric: r,
              color: '#e5534b',
            },
          }],
        };
      },
    }, 'main');

    registerFamilyProjection({
      family: 'bars',
      kind: stackedPairKind,
      project(view): BarsModel {
        const l = view.hour >= 101 ? 70 : 10;
        const r = view.hour >= 101 ? 150 : 50;
        return {
          form: 'two-sided',
          twoSidedLayout: 'stacked-pair',
          hour: view.hour,
          nextChangeAt: view.hour + 1,
          scaleMax: 200,
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            left: { value: { value: knownValue(l), format: 'integer' }, numeric: l, color: '#4f86f7' },
            right: { value: { value: knownValue(r), format: 'integer' }, numeric: r, color: '#3fb950' },
          }],
        };
      },
    }, 'main');

    registerFamilyProjection({
      family: 'bars',
      kind: shareKind,
      project(view): BarsModel {
        const a = view.hour >= 101 ? 0.8 : 0.2;
        return {
          form: 'share',
          hour: view.hour,
          nextChangeAt: view.hour + 1,
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            segments: [
              { key: 'a', label: 'A', share: a, value: knownValue(Math.round(a * 100)), color: '#4f86f7' },
              { key: 'b', label: 'B', share: 1 - a, value: knownValue(Math.round((1 - a) * 100)), color: '#3fb950' },
            ],
          }],
        };
      },
    }, 'main');

    registerFamilyProjection({
      family: 'bars',
      kind: targetKind,
      project(view): BarsModel {
        const delivered = view.hour >= 101 ? 80 : 20;
        const need = view.hour >= 101 ? 160 : 100;
        return {
          form: 'ordered',
          hour: view.hour,
          nextChangeAt: view.hour + 1,
          scaleMax: 200,
          rows: [{
            kind: 'data',
            key: 'GER',
            label: knownValue('Germany'),
            value: { value: knownValue(delivered), format: 'integer' },
            numeric: delivered,
            target: { numeric: need, value: knownValue(need), format: 'integer' },
            color: '#4f86f7',
          }],
        };
      },
    }, 'main');

    let now = 1000;
    const later: (() => void)[] = [];
    const view = createLaneView(barsLane, () => {});
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present() {},
      release() {},
      fail(reason) { throw new Error(reason); },
      later(fn) { later.push(fn); },
      clock: () => now,
      budget: Infinity,
    });
    const shared = makeRecordingCanvas(1600, 240);
    lane.setLaneSurface(shared.canvas, true);
    const kinds = [pairedKind, splitKind, stackedPairKind, shareKind, targetKind];
    kinds.forEach((kind, idx) => {
      const wid = idx + 1;
      lane.mount(wid, [], 'bars', { family: 'bars', kind, view: 'main', options: {}, sources: {}, scrollTop: 0, width: 300, height: 220, theme: TEST_THEME } satisfies FamilyBarsBinding);
      lane.bind(wid, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 120 } });
    });
    lane.setLaneSlots({
      width: 1600,
      height: 240,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: kinds.map((_, idx) => ({ widget: idx + 1, x: idx * 320, y: 0, w: 300, h: 220, visible: true })),
    });

    const origin = performance.timeOrigin;
    lane.setClock({
      playing: true,
      anchorPosition: 100,
      anchorTimeMs: origin + 1000,
      speed: 1,
      seek: 1,
      exactHour: 100,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 120,
    });
    shared.clearOps();
    later.splice(0).forEach(fn => fn());
    const opsAt100 = shared.snapshotOps();

    now = 1500;
    shared.clearOps();
    later.splice(0).forEach(fn => fn());
    const opsAt1005 = shared.snapshotOps();
    assert.equal(opsAt1005.length, kinds.length);
    for (let i = 0; i < kinds.length; i++) {
      assert.notDeepEqual(opsAt1005[i], opsAt100[i], `bars form ${kinds[i]} must interpolate between hour 100 and 101`);
    }
  }));

  it('Rule I: bigint crossing zero in bars formats cleanly with its recorded type', () => withFakeOffscreen(() => {
    const barsKind = 'test-rule-i-bars-bigint';

    registerFamilyProjection({
      family: 'bars',
      kind: barsKind,
      project(view): BarsModel {
        const v = view.hour >= 101 ? 10n : -10n;
        const z = view.hour >= 101 ? 20n : 0n;
        return {
          form: 'net',
          hour: view.hour,
          nextChangeAt: view.hour + 1,
          scaleMax: 30,
          scaleFormat: 'integer',
          rows: [
            { kind: 'data', key: 'CROSS', label: knownValue('Cross'), value: { value: knownValue(v), format: 'integer' }, numeric: Number(v), color: '#4f86f7' },
            { kind: 'data', key: 'ZERO', label: knownValue('Zero'), value: { value: z === 0n ? zeroValue(0n) : knownValue(z), format: 'integer' }, numeric: Number(z), color: '#3fb950' },
          ],
        };
      },
    }, 'main');

    let now = 1000;
    const later: (() => void)[] = [];
    const failures: string[] = [];
    const view = createLaneView(barsLane, () => {});
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present() {},
      release() {},
      fail(reason) { failures.push(reason); },
      failWidget(_w, reason) { failures.push(reason); },
      later(fn) { later.push(fn); },
      clock: () => now,
      budget: Infinity,
    });
    const shared = makeRecordingCanvas(340, 240);
    lane.setLaneSurface(shared.canvas, true);
    lane.mount(1, [], 'bars', { family: 'bars', kind: barsKind, view: 'main', options: {}, sources: {}, scrollTop: 0, width: 300, height: 220, theme: TEST_THEME } satisfies FamilyBarsBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 220, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 120 } });
    lane.setLaneSlots({
      width: 340,
      height: 240,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 0, y: 0, w: 300, h: 220, visible: true }],
    });

    assert.deepEqual(interpolateFamilyValue(knownValue(-10n), 0), { kind: 'zero', value: 0n });
    assert.deepEqual(interpolateFamilyValue(zeroValue(0n), 10), { kind: 'known', value: 10n });

    const origin = performance.timeOrigin;
    lane.setClock({
      playing: true,
      anchorPosition: 100,
      anchorTimeMs: origin + 1000,
      speed: 0.01,
      seek: 1,
      exactHour: 100,
      branch: 1,
      settleStartMs: 0,
      settleDurationMs: 0,
      lastHour: 120,
    });
    later.splice(0).forEach(fn => fn());

    for (const t of [50800, 50900, 51000]) {
      now = t;
      shared.clearOps();
      later.splice(0).forEach(fn => fn());
    }
    assert.deepEqual(failures, [], `bigint crossing zero must not throw formatter errors: ${failures.join('; ')}`);
    const barsTexts = shared.snapshotOps()[0]!.filter(o => o.op === 'fillText').map(o => String(o.args[0]));
    assert.equal(barsTexts.includes('0'), true, `expected 0 in bars crossing zero, got ${barsTexts.join(', ')}`);
    assert.equal(barsTexts.includes('10'), true, `expected 10 in bars moving 0n -> 20n, got ${barsTexts.join(', ')}`);
  }));
});
