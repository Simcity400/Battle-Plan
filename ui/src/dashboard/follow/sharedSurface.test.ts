import assert from 'node:assert/strict';
import test from 'node:test';
import { snapCanvasText } from '../../draw/kit.ts';
import { Lane, type Bitmapped, type LaneView } from '../../draw/lane.ts';
import type { LaneSlotsPayload } from '../../draw/laneMessages.ts';
import type { FamilyTheme } from '../../families/contract.ts';
import { createLaneView, type LaneBinding, type LaneFamily } from '../../families/laneRuntime.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { PAGES } from '../pages.ts';
import { PlaybackController } from '../playback.ts';
import { Page, type FrameRequest } from './page.ts';
import { Pipeline } from './pipeline.ts';
import { computeSharedLaneGeometry, computeSpatialLaneMap, isImageBackdropActive, SharedSurfaceHost } from './sharedSurface.ts';
import { laneCount, lanesHost } from './useFollowed.ts';

function fakeContext2D() {
  const ops: string[] = [];
  const ctx: Record<string, unknown> = {
    fillStyle: '#000',
    strokeStyle: '#000',
    font: '12px sans-serif',
    save() { ops.push('save'); },
    restore() { ops.push('restore'); },
    beginPath() { ops.push('beginPath'); },
    rect(x: number, y: number, w: number, h: number) { ops.push(`rect:${x},${y},${w},${h}`); },
    clip() { ops.push('clip'); },
    fillRect(x: number, y: number, w: number, h: number) { ops.push(`fillRect:${x},${y},${w},${h}`); },
    clearRect(x: number, y: number, w: number, h: number) { ops.push(`clearRect:${x},${y},${w},${h}`); },
    drawImage() { ops.push('drawImage'); },
    fillText(text: string, x: number, y: number) { ops.push(`fillText:${text}@${x},${y}`); },
    measureText(text: string) { return { width: text.length * 7 }; },
    setTransform(a?: unknown, b?: unknown, c?: unknown, d?: unknown, e?: unknown, f?: unknown) {
      ops.push(`setTransform:${String(a)},${String(b)},${String(c)},${String(d)},${String(e)},${String(f)}`);
    },
    resetTransform() { ops.push('resetTransform'); },
    getTransform() { return { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }; },
  };
  return { ctx: ctx as unknown as OffscreenCanvasRenderingContext2D, ops };
}

function fakeOffscreenCanvas(width = 1, height = 1) {
  const { ctx, ops } = fakeContext2D();
  const canvas = {
    width,
    height,
    getContext() { return ctx; },
  };
  return { canvas: canvas as unknown as OffscreenCanvas, ctx, ops };
}

const TEST_THEME: FamilyTheme = {
  surface: '#11161d',
  dark: true,
  text: '#eef2f7',
  muted: '#8894a6',
  line: '#2a3342',
  lineSoft: '#1f2733',
  accent: '#4ea1ff',
  blue: '#4ea1ff',
  red: '#e5534b',
  amber: '#d29922',
  green: '#3fb950',
  grey: '#6e7681',
  hatch: '#30394a',
  font: 'Inter, sans-serif',
  fontSize: { small: 11, body: 13 },
  lineWidth: 1.5,
  hatchSpacing: 6,
};

test('laneCount keeps the 4-lane cap from hardwareConcurrency and spatial partition bounds by widget count', () => {
  const navDesc = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  const setHw = (cores: number) => {
    Object.defineProperty(globalThis, 'navigator', { value: { hardwareConcurrency: cores }, configurable: true });
  };
  try {
    setHw(2);
    assert.equal(laneCount(), 1);
    setHw(4);
    assert.equal(laneCount(), 2);
    setHw(8);
    assert.equal(laneCount(), 4);
    setHw(16);
    assert.equal(laneCount(), 4);
  } finally {
    if (navDesc) Object.defineProperty(globalThis, 'navigator', navDesc);
    else delete (globalThis as { navigator?: unknown }).navigator;
  }

  const three = [
    { id: 'a', x: 0, y: 0, w: 6, h: 8 },
    { id: 'b', x: 6, y: 0, w: 6, h: 8 },
    { id: 'c', x: 0, y: 8, w: 12, h: 8 },
  ];
  const part = computeSpatialLaneMap(three, 4);
  assert.equal(part.hasCleanCut, true);
  assert.equal(new Set(part.laneById.values()).size, 3);
});

test('no-straight-cut pinwheel layout activates clip-path path(...) fallback while all shipped layouts cut cleanly', () => {
  const pinwheel = [
    { id: 'top', x: 0, y: 0, w: 8, h: 4 },
    { id: 'right', x: 8, y: 0, w: 4, h: 8 },
    { id: 'bottom', x: 4, y: 8, w: 8, h: 4 },
    { id: 'left', x: 0, y: 4, w: 4, h: 8 },
  ];
  const part = computeSpatialLaneMap(pinwheel, 2);
  assert.equal(part.hasCleanCut, false);

  const scale = 100;
  const geom = computeSharedLaneGeometry(
    pinwheel.map((item, idx) => ({
      widget: idx + 1,
      lane: part.laneById.get(item.id)!,
      rect: { x: item.x * scale, y: item.y * scale, w: item.w * scale, h: item.h * scale },
      inView: true,
      active: true,
    })),
    { x: 0, y: 0, w: 1200, h: 1200 },
    1,
  );
  assert.equal(geom.clipPathFallback, true);
  assert.equal(geom.lanes.size, 2);
  for (const lane of geom.lanes.values()) {
    assert.equal(lane.clipPathFallback, true);
    assert.equal(lane.width, 1200);
    assert.equal(lane.height, 1200);
    assert.match(lane.clipPath, /^path\("M\d+ \d+H\d+V\d+H\d+Z( M\d+ \d+H\d+V\d+H\d+Z)*"\)$/);
    assert.equal(lane.slots.length, 2);
  }

  for (const k of [2, 4, 8]) {
    for (const page of PAGES) {
      const items = page.panels.map(([, x, y, w, h], idx) => ({ id: `${page.id}-${idx}`, x, y, w, h }));
      const p = computeSpatialLaneMap(items, k);
      assert.equal(p.hasCleanCut, true, `${page.id} should partition cleanly at ${k} lanes`);
      const g = computeSharedLaneGeometry(
        items.map((item, idx) => ({
          widget: idx + 1,
          lane: p.laneById.get(item.id)!,
          rect: { x: item.x * 100, y: item.y * 30, w: item.w * 100, h: item.h * 30 },
          inView: true,
          active: true,
        })),
        { x: 0, y: 0, w: 1200, h: 6000 },
        1,
      );
      assert.equal(g.clipPathFallback, false, `${page.id} should not need clip-path at ${k} lanes`);
    }
  }
});

test('image backdrop disables shared surface host and restores it when cleared', () => {
  const origDoc = globalThis.document;
  const origHtmlCanvas = (globalThis as { HTMLCanvasElement?: unknown }).HTMLCanvasElement;
  const origMO = globalThis.MutationObserver;
  const origRO = globalThis.ResizeObserver;
  const origRaf = globalThis.requestAnimationFrame;
  const origCancelRaf = globalThis.cancelAnimationFrame;
  const origWindow = (globalThis as { window?: unknown }).window;
  const origGetComputedStyle = globalThis.getComputedStyle;

  let moCallback: (() => void) | null = null;
  const transferredLanes: number[] = [];
  const sentSlots: { lane: number; payload: LaneSlotsPayload }[] = [];

  class FakeElement {
    className = '';
    dataset: Record<string, string> = {};
    style: Record<string, string> = {};
    children: FakeElement[] = [];
    firstChild: FakeElement | null = null;
    scrollTop = 0;
    scrollLeft = 0;
    width = 0;
    height = 0;
    setAttribute() {}
    appendChild(child: FakeElement) {
      this.children.push(child);
      if (!this.firstChild) this.firstChild = child;
      return child;
    }
    insertBefore(child: FakeElement) {
      this.children.unshift(child);
      this.firstChild = child;
      return child;
    }
    addEventListener() {}
    removeEventListener() {}
    remove() {}
    closest() { return null; }
    getBoundingClientRect() { return { left: 0, top: 0, right: 800, bottom: 600, width: 800, height: 600 }; }
    transferControlToOffscreen() { return fakeOffscreenCanvas().canvas; }
  }

  const docEl = new FakeElement();
  docEl.dataset.backdrop = 'solid';

  Object.defineProperty(globalThis, 'HTMLCanvasElement', { value: FakeElement, configurable: true });
  Object.defineProperty(globalThis, 'document', {
    value: {
      documentElement: docEl,
      createElement() { return new FakeElement(); },
    },
    configurable: true,
  });
  Object.defineProperty(globalThis, 'window', { value: { devicePixelRatio: 1 }, configurable: true });
  Object.defineProperty(globalThis, 'getComputedStyle', {
    value: () => ({ getPropertyValue: () => '#11161d', backgroundColor: '#11161d' }),
    configurable: true,
  });
  Object.defineProperty(globalThis, 'ResizeObserver', { value: undefined, configurable: true });
  Object.defineProperty(globalThis, 'requestAnimationFrame', { value: (fn: FrameRequestCallback) => { fn(0); return 1; }, configurable: true });
  Object.defineProperty(globalThis, 'cancelAnimationFrame', { value: () => {}, configurable: true });
  Object.defineProperty(globalThis, 'MutationObserver', {
    value: class {
      constructor(cb: () => void) { moCallback = cb; }
      observe() {}
      disconnect() {}
    },
    configurable: true,
  });

  try {
    const fakePipeline = {
      laneCount: 4,
      transferLaneSurface(lane: number) { transferredLanes.push(lane); },
      sendLaneSlots(lane: number, payload: LaneSlotsPayload) { sentSlots.push({ lane, payload }); },
    };
    const scrollEl = new FakeElement();
    const host = new SharedSurfaceHost(fakePipeline as never, scrollEl as unknown as HTMLElement);
    assert.equal(isImageBackdropActive(), false);
    assert.equal(host.enabled, true);
    assert.deepEqual(transferredLanes, []);

    const bodyEl = new FakeElement();
    const frameEl = new FakeElement();
    let notifications = 0;
    host.subscribe(() => { notifications++; });
    host.register(1, 0, bodyEl as unknown as HTMLElement, frameEl as unknown as HTMLElement);
    assert.deepEqual(transferredLanes, [0]);
    assert.equal(frameEl.dataset.sharedSurface, '1');

    docEl.dataset.backdrop = 'image';
    moCallback!();
    assert.equal(isImageBackdropActive(), true);
    assert.equal(host.enabled, false);
    assert.equal(notifications, 1);
    assert.equal(frameEl.dataset.sharedSurface, undefined);
    assert.equal(sentSlots.at(-1)?.payload.slots.length, 0);

    docEl.dataset.backdrop = 'solid';
    moCallback!();
    assert.equal(isImageBackdropActive(), false);
    assert.equal(host.enabled, true);
    assert.equal(notifications, 2);
    assert.equal(frameEl.dataset.sharedSurface, '1');
    assert.equal(frameEl.dataset.sharedLane, '0');
    host.dispose();
    assert.equal(frameEl.dataset.sharedLane, undefined);
  } finally {
    Object.defineProperty(globalThis, 'document', { value: origDoc, configurable: true });
    Object.defineProperty(globalThis, 'HTMLCanvasElement', { value: origHtmlCanvas, configurable: true });
    Object.defineProperty(globalThis, 'MutationObserver', { value: origMO, configurable: true });
    Object.defineProperty(globalThis, 'ResizeObserver', { value: origRO, configurable: true });
    Object.defineProperty(globalThis, 'requestAnimationFrame', { value: origRaf, configurable: true });
    Object.defineProperty(globalThis, 'cancelAnimationFrame', { value: origCancelRaf, configurable: true });
    Object.defineProperty(globalThis, 'window', { value: origWindow, configurable: true });
    Object.defineProperty(globalThis, 'getComputedStyle', { value: origGetComputedStyle, configurable: true });
  }
});

test('stationary widget costs zero repaints with shared surface on while active sibling repaints', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number;
      height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) {
        this.width = w;
        this.height = h;
        this.ctx = fakeContext2D().ctx;
      }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });

  try {
    const stationaryKind = 'test-stationary-shared-surface';
    const activeKind = 'test-active-shared-surface';
    registerFamilyProjection({
      family: 'bars',
      kind: stationaryKind,
      project(view) { return { hour: view.hour, stable: true }; },
    }, 'main');
    registerFamilyProjection({
      family: 'bars',
      kind: activeKind,
      project(view) { return { hour: view.hour, stable: false }; },
    }, 'main');

    const paintCounts = new Map<string, number>();
    const family: LaneFamily<LaneBinding, { readonly hour: number; readonly stable: boolean }, { readonly key: string }> = {
      family: 'bars',
      boundary: (model, hour) => (model.stable ? Infinity : hour + 1),
      redraw: () => Infinity,
      status: () => 'picture',
      view: [],
      layout: (model) => ({ key: model.stable ? 'stable' : `h:${model.hour}` }),
      paint(_ctx, frame) {
        const k = frame.binding.kind;
        paintCounts.set(k, (paintCounts.get(k) ?? 0) + 1);
        return false;
      },
      hit: () => null,
      tooltip: () => null,
      structure: () => ({}),
      mirror: null,
      audit: () => [],
      visibleArt: () => [],
    };

    const view: LaneView = createLaneView(family, () => {});
    const presented: { id: number; widget: number; binding: number }[] = [];
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present(p) {
        for (const e of p.entries) presented.push({ id: e.id, widget: e.widget, binding: e.binding });
      },
      release() {},
      fail(reason) { throw new Error(reason); },
      later() {},
      clock: () => 0,
      budget: Infinity,
    });

    const sharedCanvas = fakeOffscreenCanvas(800, 600);
    lane.setLaneSurface(sharedCanvas.canvas, true);

    const bindingFor = (kind: string): LaneBinding => ({
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      revisions: {},
      sources: {},
      scrollTop: 0,
      theme: TEST_THEME,
    });

    lane.mount(1, [], 'bars', bindingFor(stationaryKind));
    lane.mount(2, [], 'bars', bindingFor(activeKind));
    lane.bind(1, { generation: 1, surfaces: [{ width: 380, height: 240, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 100 } });
    lane.bind(2, { generation: 1, surfaces: [{ width: 380, height: 240, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 100 } });
    lane.setLaneSlots({
      width: 800,
      height: 260,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: '#11161d',
      clipPathFallback: false,
      slots: [
        { widget: 1, x: 0, y: 0, w: 380, h: 240, visible: true },
        { widget: 2, x: 400, y: 0, w: 380, h: 240, visible: true },
      ],
    });

    const tick = async (hour: number) => {
      const acks = presented.splice(0).map(p => ({ id: p.id, widget: p.widget, binding: p.binding }));
      lane.frame({
        type: 'frame',
        serial: hour,
        seek: 1,
        target: hour,
        now: hour,
        exact: false,
        speed: 24,
        inspect: [],
        installed: acks,
        refused: [],
      });
      for (let i = 0; i < 6; i++) await Promise.resolve();
    };

    await tick(1);
    assert.equal(paintCounts.get(stationaryKind), 1);
    assert.equal(paintCounts.get(activeKind), 1);

    for (const h of [2, 3, 4, 5]) {
      await tick(h);
    }
    assert.equal(paintCounts.get(stationaryKind), 1);
    assert.equal(paintCounts.get(activeKind), 5);

    lane.setLaneSlots({
      width: 800,
      height: 260,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      surfaceColor: '#11161d',
      clipPathFallback: false,
      slots: [
        { widget: 1, x: 10, y: 0, w: 380, h: 240, visible: true },
        { widget: 2, x: 400, y: 0, w: 380, h: 240, visible: true },
      ],
    });
    await tick(6);
    assert.equal(paintCounts.get(stationaryKind), 2);
    await tick(7);
    assert.equal(paintCounts.get(stationaryKind), 2);
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('shared targets paint look-ahead across in-flight moves and worker fallback paints the current hour', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number;
      height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) {
        this.width = w;
        this.height = h;
        this.ctx = fakeContext2D().ctx;
      }
      getContext() { return this.ctx; }
      transferToImageBitmap() { return { width: this.width, height: this.height, close() {} }; }
    },
    configurable: true,
  });

  try {
    const kind = 'test-direct-lookahead-shared';
    registerFamilyProjection({
      family: 'bars',
      kind,
      project(view) { return { hour: view.hour }; },
    }, 'main');

    const painted: { hour: number; target: 'shared' | 'worker' }[] = [];
    const sharedCanvas = fakeOffscreenCanvas(800, 600);
    const workerCanvas = fakeOffscreenCanvas(380, 240);

    const family: LaneFamily<LaneBinding, { readonly hour: number }, { readonly hour: number }> = {
      family: 'bars',
      boundary: (_model, hour) => hour + 1,
      redraw: () => Infinity,
      status: () => 'picture',
      view: [],
      layout: (model) => ({ hour: model.hour }),
      paint(ctx, frame) {
        painted.push({
          hour: frame.hour,
          target: ctx === workerCanvas.ctx ? 'worker' : 'shared',
        });
        return false;
      },
      hit: () => null,
      tooltip: () => null,
      structure: () => ({}),
      mirror: null,
      audit: () => [],
      visibleArt: () => [],
    };

    const view: LaneView = createLaneView(family, () => {});
    const presented: { id: number; widget: number; binding: number }[] = [];
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present(p) {
        for (const e of p.entries) presented.push({ id: e.id, widget: e.widget, binding: e.binding });
      },
      release() {},
      fail(reason) { throw new Error(reason); },
      later() {},
      clock: () => 0,
      budget: Infinity,
    });

    lane.setLaneSurface(sharedCanvas.canvas, true);
    lane.setCanvas(1, workerCanvas.canvas, false);
    lane.mount(1, [], 'bars', {
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      sources: {},
      scrollTop: 0,
      theme: TEST_THEME,
    } satisfies LaneBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 380, height: 240, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 100 } });
    lane.setLaneSlots({
      width: 800,
      height: 260,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 0, y: 0, w: 380, h: 240, visible: true }],
    });

    const sendFrame = async (serial: number, now: number, target: number, ack = true) => {
      const acks = ack ? presented.splice(0).map(p => ({ id: p.id, widget: p.widget, binding: p.binding })) : [];
      lane.frame({
        type: 'frame',
        serial,
        seek: 1,
        target,
        now,
        exact: false,
        speed: 24,
        inspect: [],
        installed: acks,
        refused: [],
      });
      for (let i = 0; i < 6; i++) await Promise.resolve();
    };

    await sendFrame(1, 1, 3, false);
    assert.deepEqual(painted, [{ hour: 3, target: 'shared' }]);

    lane.setLaneSlots({
      width: 800,
      height: 260,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 24, y: 0, w: 380, h: 240, visible: true }],
    });
    await sendFrame(2, 1, 3, false);
    assert.equal(painted.length, 2);
    assert.deepEqual(painted.at(-1), { hour: 3, target: 'shared' });

    lane.setLaneSlots({
      width: 0,
      height: 0,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [],
    });
    await sendFrame(3, 1, 3, false);
    assert.equal(painted.length, 3);
    assert.deepEqual(painted.at(-1), { hour: 1, target: 'worker' });

    lane.disposeLaneSurface();
    assert.equal(sharedCanvas.canvas.width, 1);
    assert.equal(sharedCanvas.canvas.height, 1);
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('inactive lane in pinwheel layout does not emit an unclipped viewport canvas', () => {
  const geom = computeSharedLaneGeometry(
    [
      { widget: 1, lane: 0, rect: { x: 0, y: 0, w: 800, h: 400 }, inView: true, active: true },
      { widget: 2, lane: 0, rect: { x: 400, y: 800, w: 800, h: 400 }, inView: true, active: true },
      { widget: 3, lane: 1, rect: { x: 800, y: 0, w: 400, h: 800 }, inView: true, active: false },
      { widget: 4, lane: 1, rect: { x: 0, y: 400, w: 400, h: 800 }, inView: true, active: false },
    ],
    { x: 0, y: 0, w: 1200, h: 1200 },
    1,
  );
  assert.equal(geom.lanes.has(1), false);
  assert.equal(geom.lanes.has(0), true);
  assert.equal(geom.clipPathFallback, false);
});

test('round-2 regressions: shared look-ahead, uncovered pause, bitmap fallback, scroll blit, and repainted telemetry', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number;
      height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) {
        this.width = w;
        this.height = h;
        this.ctx = fakeContext2D().ctx;
      }
      getContext() { return this.ctx; }
      transferToImageBitmap() { return { width: this.width, height: this.height, close() {} }; }
    },
    configurable: true,
  });

  try {
    const kind = 'test-round2-direct-regressions';
    registerFamilyProjection({
      family: 'bars',
      kind,
      project(view) { return { hour: view.hour }; },
    }, 'main');

    const paintedHours: number[] = [];
    const directPaintedHours: number[] = [];
    const sharedCanvas = fakeOffscreenCanvas(800, 600);
    const glassCanvas = fakeOffscreenCanvas(380, 240);
    const family: LaneFamily<LaneBinding, { readonly hour: number }, { readonly hour: number }> = {
      family: 'bars',
      boundary: (_model, hour) => hour + 1,
      redraw: () => Infinity,
      status: () => 'picture',
      view: [],
      layout: (model) => ({ hour: model.hour }),
      paint(ctx, frame) {
        paintedHours.push(frame.hour);
        if (ctx === sharedCanvas.ctx || ctx === glassCanvas.ctx) directPaintedHours.push(frame.hour);
        return false;
      },
      hit: () => null,
      tooltip: () => null,
      structure: () => ({}),
      mirror: null,
      audit: () => [],
      visibleArt: () => [],
    };

    const view: LaneView = createLaneView(family, () => {});
    const presented: { id: number; widget: number; binding: number; repainted?: boolean; surfaces: number }[] = [];
    let surfaceFailed = 0;
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async (sources) => sources.map(s => ({ width: s.width, height: s.height, close() {} })),
      present(p) {
        for (const e of p.entries) {
          presented.push({ id: e.id, widget: e.widget, binding: e.binding, repainted: e.repainted, surfaces: e.surfaces.length });
        }
      },
      release() {},
      fail(reason) { throw new Error(reason); },
      failLaneSurface() { surfaceFailed++; },
      later() {},
      clock: () => 0,
      budget: Infinity,
    });

    lane.setLaneSurface({ width: 1, height: 1, getContext: () => null } as unknown as OffscreenCanvas, true);
    assert.equal(surfaceFailed, 1);

    lane.setLaneSurface(sharedCanvas.canvas, true);
    lane.setCanvas(1, glassCanvas.canvas, false);
    lane.mount(1, [], 'bars', {
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      sources: {},
      scrollTop: 0,
      theme: TEST_THEME,
    } satisfies LaneBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 380, height: 240, dpr: 1 }], offset: 0, pin: null, dated: { h: 12, c: 19 } });
    lane.setLaneSlots({
      width: 800,
      height: 300,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 0, y: 20, w: 380, h: 240, visible: true }],
    });

    const tick = async (serial: number, now: number, target: number, exact: boolean, retireEarly = false) => {
      const previous = presented.splice(0);
      lane.frame({
        type: 'frame',
        serial,
        seek: 1,
        target,
        now,
        exact,
        speed: exact ? 0 : 24,
        inspect: [],
        installed: retireEarly ? [] : previous.map(({ id, widget, binding }) => ({ id, widget, binding })),
        refused: retireEarly ? previous.map(({ id, widget }) => ({ id, widget, reason: 'retired' })) : [],
      });
      for (let i = 0; i < 6; i++) await Promise.resolve();
    };

    await tick(1, 10, 11, false);
    assert.deepEqual(paintedHours, []);

    await tick(2, 10, 14, false);
    assert.deepEqual(paintedHours, [14]);
    assert.deepEqual(directPaintedHours, []);
    assert.equal(sharedCanvas.ops.includes('drawImage'), false);
    assert.equal(presented.at(-1)?.repainted, true);

    await tick(3, 12, 12, true, true);
    assert.deepEqual(paintedHours, [14, 12]);
    assert.deepEqual(directPaintedHours, [12]);
    assert.equal(presented.at(-1)?.repainted, true);

    await tick(4, 20, 20, true);
    assert.deepEqual(paintedHours, [14, 12]);

    sharedCanvas.ops.length = 0;
    lane.setLaneSlots({
      width: 800,
      height: 300,
      dpr: 1,
      scrollTop: 10,
      scrollLeft: 0,
      originX: 0,
      originY: 10,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 0, y: 10, w: 380, h: 240, visible: true }],
    });
    assert.ok(sharedCanvas.ops.includes('drawImage'));
    assert.equal(sharedCanvas.ops.includes('fillRect:0,10,380,240'), false);

    lane.setLaneSlots({
      width: 0,
      height: 0,
      dpr: 1,
      scrollTop: 10,
      scrollLeft: 0,
      originX: 0,
      originY: 10,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [],
    });
    lane.clearCanvas(1);
    await tick(5, 15, 15, true);
    assert.equal(presented.at(-1)?.surfaces, 1);

    lane.useCanvas(1);
    await tick(6, 16, 16, true);
    assert.equal(presented.at(-1)?.surfaces, 0);
    assert.equal(paintedHours.at(-1), 16);
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('round-3 regressions: uncovered refusal restores installed model while parked, unmount clears slot rect, and multi-widget lane clips gutters', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number;
      height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) {
        this.width = w;
        this.height = h;
        this.ctx = fakeContext2D().ctx;
      }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });

  try {
    const geom = computeSharedLaneGeometry(
      [
        { widget: 1, lane: 0, rect: { x: 0, y: 0, w: 300, h: 200 }, inView: true, active: true },
        { widget: 2, lane: 0, rect: { x: 320, y: 0, w: 300, h: 200 }, inView: true, active: true },
      ],
      { x: 0, y: 0, w: 800, h: 600 },
      1,
    );
    assert.equal(geom.clipPathFallback, false);
    assert.equal(geom.lanes.get(0)!.clipPath, '');

    const kind = 'test-round3-uncovered-restore';
    registerFamilyProjection({
      family: 'bars',
      kind,
      project(view, options) {
        return { hour: view.hour, tag: (options as { tag?: string }).tag ?? 'a' };
      },
    }, 'main');

    const painted: { hour: number; tag: string }[] = [];
    const family: LaneFamily<LaneBinding, { readonly hour: number; readonly tag: string }, { readonly key: string }> = {
      family: 'bars',
      boundary: (_model, hour) => hour + 1,
      redraw: () => Infinity,
      status: () => 'picture',
      view: [],
      layout: (model) => ({ key: `${model.hour}:${model.tag}` }),
      paint(_ctx, frame) {
        painted.push({ hour: frame.hour, tag: frame.model.tag });
        return false;
      },
      hit: () => null,
      tooltip: () => null,
      structure: () => ({}),
      mirror: null,
      audit: () => [],
      visibleArt: () => [],
    };

    const view: LaneView = createLaneView(family, () => {});
    const presented: { id: number; widget: number; binding: number }[] = [];
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present(p) {
        for (const e of p.entries) presented.push({ id: e.id, widget: e.widget, binding: e.binding });
      },
      release() {},
      fail(reason) { throw new Error(reason); },
      later() {},
      clock: () => 0,
      budget: Infinity,
    });

    const sharedCanvas = fakeOffscreenCanvas(800, 600);
    lane.setLaneSurface(sharedCanvas.canvas, true);
    lane.mount(1, [], 'bars', {
      family: 'bars',
      kind,
      view: 'main',
      options: { tag: 'installed' },
      sources: {},
      scrollTop: 0,
      theme: TEST_THEME,
    } satisfies LaneBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 5 } });
    lane.setLaneSlots({
      width: 800,
      height: 300,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 40, y: 20, w: 300, h: 200, visible: true }],
    });

    const sendFrame = async (serial: number, now: number, exact: boolean, installed: { id: number; widget: number; binding: number }[], refused: { id: number; widget: number; reason: string }[]) => {
      lane.frame({
        type: 'frame',
        serial,
        seek: 1,
        target: now,
        now,
        exact,
        speed: exact ? 0 : 24,
        inspect: [],
        installed,
        refused,
      });
      for (let i = 0; i < 6; i++) await Promise.resolve();
    };

    await sendFrame(1, 1, true, [], []);
    assert.deepEqual(painted, [{ hour: 1, tag: 'installed' }]);
    const entry1 = presented.splice(0);

    await sendFrame(2, 5, false, entry1, []);
    assert.deepEqual(painted.at(-1), { hour: 5, tag: 'installed' });
    const entry5 = presented.splice(0);

    lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 5 } }, {
      family: 'bars',
      kind,
      view: 'main',
      options: { tag: 'rebound' },
      sources: {},
      scrollTop: 0,
      theme: TEST_THEME,
    } satisfies LaneBinding);

    await sendFrame(3, 6, true, [], [{ id: entry5[0]!.id, widget: 1, reason: 'uncovered' }]);
    assert.deepEqual(painted.at(-1), { hour: 1, tag: 'installed' });

    sharedCanvas.ops.length = 0;
    lane.unmount(1);
    assert.ok(sharedCanvas.ops.includes('clearRect:40,20,300,200'));
    assert.ok(!sharedCanvas.ops.some(op => op.startsWith('fillRect:')));
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

function pixelCanvas(initialW: number, initialH: number) {
  let w = initialW;
  let h = initialH;
  const fills: { x: number; y: number; w: number; h: number; style: string }[] = [];
  let transform = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
  const stack: { transform: typeof transform; fillStyle: string }[] = [];
  const ctx: Record<string, unknown> = {
    fillStyle: 'transparent',
    strokeStyle: 'transparent',
    font: '12px sans-serif',
    save() { stack.push({ transform: { ...transform }, fillStyle: String(ctx.fillStyle) }); },
    restore() {
      const top = stack.pop();
      if (top) { transform = top.transform; ctx.fillStyle = top.fillStyle; }
    },
    beginPath() {},
    rect() {},
    clip() {},
    fillRect(x: number, y: number, rw: number, rh: number) {
      const x0 = transform.a * x + transform.e;
      const y0 = transform.d * y + transform.f;
      const x1 = transform.a * (x + rw) + transform.e;
      const y1 = transform.d * (y + rh) + transform.f;
      fills.push({
        x: Math.min(x0, x1),
        y: Math.min(y0, y1),
        w: Math.abs(x1 - x0),
        h: Math.abs(y1 - y0),
        style: String(ctx.fillStyle),
      });
    },
    clearRect(x: number, y: number, rw: number, rh: number) {
      const x0 = transform.a * x + transform.e;
      const y0 = transform.d * y + transform.f;
      const x1 = transform.a * (x + rw) + transform.e;
      const y1 = transform.d * (y + rh) + transform.f;
      fills.push({
        x: Math.min(x0, x1),
        y: Math.min(y0, y1),
        w: Math.abs(x1 - x0),
        h: Math.abs(y1 - y0),
        style: 'cleared',
      });
    },
    drawImage(image: { pixels?: typeof fills }, x: number, y: number) {
      for (const fill of image.pixels ?? []) fills.push({ ...fill, x: x + fill.x, y: y + fill.y });
    },
    fillText() {},
    measureText(text: string) { return { width: text.length * 7 }; },
    setTransform(a?: unknown, b?: unknown, c?: unknown, d?: unknown, e?: unknown, f?: unknown) {
      if (typeof a === 'number') {
        transform = { a, b: Number(b ?? 0), c: Number(c ?? 0), d: Number(d ?? 1), e: Number(e ?? 0), f: Number(f ?? 0) };
      } else if (a && typeof a === 'object') {
        const m = a as { a?: number; b?: number; c?: number; d?: number; e?: number; f?: number };
        transform = { a: m.a ?? 1, b: m.b ?? 0, c: m.c ?? 0, d: m.d ?? 1, e: m.e ?? 0, f: m.f ?? 0 };
      } else {
        transform = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 };
      }
    },
    resetTransform() { transform = { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }; },
    getTransform() { return { ...transform }; },
  };
  const canvas = {
    get width() { return w; },
    set width(v: number) { w = v; fills.length = 0; },
    get height() { return h; },
    set height(v: number) { h = v; fills.length = 0; },
    getContext() { return ctx; },
    transferToImageBitmap() {
      const pixels = fills.splice(0);
      return { width: w, height: h, pixels, close() { pixels.length = 0; } };
    },
  };
  const pixelAt = (px: number, py: number): string => {
    for (let i = fills.length - 1; i >= 0; i--) {
      const f = fills[i]!;
      if (px >= f.x && px < f.x + f.w && py >= f.y && py < f.y + f.h) return f.style;
    }
    return 'empty';
  };
  return { canvas: canvas as unknown as OffscreenCanvas, ctx: ctx as unknown as OffscreenCanvasRenderingContext2D, pixelAt, painted: (style: string) => fills.some(fill => fill.style === style) };
}

function makeDirectHarness(kind: string) {
  registerFamilyProjection({
    family: 'bars',
    kind,
    project(view, options) {
      return { hour: view.hour, tag: (options as { tag?: string }).tag ?? 'installed' };
    },
  }, 'main');

  let paintCalls = 0;
  const family: LaneFamily<LaneBinding, { readonly hour: number; readonly tag: string }, { readonly key: string }> = {
    family: 'bars',
    boundary: (_model, hour) => hour + 1,
    redraw: () => Infinity,
    status: () => 'picture',
    view: [],
    layout: (model) => ({ key: `${model.hour}:${model.tag}` }),
    paint(ctx, frame) {
      paintCalls++;
      ctx.fillStyle = `pixel:h${frame.hour}:${frame.model.tag}`;
      ctx.fillRect(0, 0, frame.surface.width, frame.surface.height);
      return false;
    },
    hit: () => null,
    tooltip: () => null,
    structure: () => ({}),
    mirror: null,
    audit: () => [],
    visibleArt: () => [],
  };

  const view: LaneView = createLaneView(family, () => {});
  const requests: FrameRequest[] = [];
  const page = new Page<Bitmapped & { close(): void }>({
    lanes: 1,
    seek: 1,
    post(_lane, message) {
      if (message.type === 'frame') requests.push(message);
    },
  });
  page.mount(1, { route: 0, lane: 0, offset: 0, pin: null, surfaces: [] });

  const heldPresentations: Parameters<typeof page.offer>[0][] = [];
  const lane = new Lane<Bitmapped & { close(): void }>({
    lane: 0,
    lanes: 1,
    views: () => view,
    crop: async () => [],
    present(p) { heldPresentations.push(p); },
    release() {},
    fail(reason) { throw new Error(reason); },
    later() {},
    clock: () => 0,
    budget: Infinity,
  });

  const surface = pixelCanvas(800, 300);
  lane.setLaneSurface(surface.canvas, true);
  const bindingModel = (tag: string): LaneBinding => ({
    family: 'bars',
    kind,
    view: 'main',
    options: { tag },
    sources: {},
    scrollTop: 0,
    theme: TEST_THEME,
  });
  lane.mount(1, [], 'bars', bindingModel('installed'));
  lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 5 } });
  lane.setLaneSlots({
    width: 800,
    height: 300,
    dpr: 1,
    scrollTop: 0,
    scrollLeft: 0,
    originX: 0,
    originY: 0,
    surfaceColor: TEST_THEME.surface,
    clipPathFallback: false,
    slots: [{ widget: 1, x: 40, y: 20, w: 300, h: 200, visible: true }],
  });

  const drainLane = async () => {
    for (const req of requests.splice(0)) lane.frame(req);
    for (let i = 0; i < 6; i++) await Promise.resolve();
  };
  const deliverToPage = () => {
    for (const p of heldPresentations.splice(0)) page.offer(p);
  };

  return { page, lane, surface, bindingModel, drainLane, deliverToPage, requests, heldPresentations, paints: () => paintCalls };
}

test('shared playback paints look-ahead and copies pixels only when the page accepts the hour', async () => {
  const original = globalThis.OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class { constructor(width: number, height: number) { return pixelCanvas(width, height).canvas; } }, configurable: true,
  });
  try {
    const h = makeDirectHarness('test-direct-look-ahead');
    h.lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 20 } });
    h.page.publish(2, { playing: true, seek: 1, speed: 24, step: 4 });
    await h.drainLane();
    h.deliverToPage();
    h.page.frame();
    await h.drainLane();
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h2:installed');

    h.page.publish(3);
    const lookAhead = h.requests.at(-1)!.target;
    assert.ok(lookAhead > h.page.T);
    await h.drainLane();
    assert.equal(h.heldPresentations.at(-1)?.entries[0]?.lo, lookAhead);
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h2:installed');
    h.deliverToPage();
    h.page.frame();
    assert.equal(h.page.view(1).painted, 2);

    h.page.publish(lookAhead);
    await h.drainLane();
    assert.equal(h.page.view(1).painted, lookAhead);
    assert.equal(h.surface.pixelAt(80, 60), `pixel:h${lookAhead}:installed`);

    h.page.publish(lookAhead + 1, { playing: false, seek: 2, speed: 0 });
    await h.drainLane();
    h.deliverToPage();
    h.page.frame();
    assert.equal(h.page.view(1).painted, lookAhead + 1);
    assert.equal(h.surface.pixelAt(80, 60), `pixel:h${lookAhead + 1}:installed`);
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: original, configurable: true });
  }
});

test('a delayed acknowledgement restores a staged picture only when the seek keeps it', async () => {
  const original = globalThis.OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class { constructor(width: number, height: number) { return pixelCanvas(width, height).canvas; } }, configurable: true,
  });
  try {
    for (const kept of [true, false]) {
      const h = makeDirectHarness(`test-staged-ack-after-seek-${kept}`);
      h.lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 20 } });
      h.page.publish(2, { playing: true, seek: 1, speed: 24, step: 4 });
      await h.drainLane();
      h.deliverToPage();
      h.page.frame();
      await h.drainLane();

      h.page.publish(3);
      const lookAhead = h.requests.at(-1)!.target;
      await h.drainLane();
      h.deliverToPage();
      h.page.publish(lookAhead);
      const delayed = h.requests.pop()!.installed;
      assert.equal(delayed.length, 1);
      const pausedHour = kept ? lookAhead : 2;
      h.page.publish(pausedHour, { playing: false, seek: 2, speed: 0 });
      assert.equal(h.page.view(1).painted, kept ? lookAhead : undefined);
      const owned = pixelCanvas(300, 200);
      if (!kept) {
        h.lane.disposeLaneSurface();
        h.lane.setCanvas(1, owned.canvas, false);
      }
      const exact = h.requests.pop()!;
      h.requests.push({ ...exact, installed: delayed });
      await h.drainLane();
      const visible = kept ? h.surface : owned;
      assert.equal(visible.pixelAt(80, 60), `pixel:h${pausedHour}:installed`);
      if (!kept) assert.equal(owned.painted(`pixel:h${lookAhead}:installed`), false);
      h.deliverToPage();
      h.page.frame();
      assert.equal(h.page.view(1).mode, 'picture');
      assert.equal(h.page.view(1).painted, pausedHour);
    }
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: original, configurable: true });
  }
});

test('direct-surface rules: content identity rejects uncovered pause replacement and restores installed pixels past parked entry', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number; height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) { this.width = w; this.height = h; this.ctx = fakeContext2D().ctx; }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });
  try {
    const h = makeDirectHarness('test-rule-uncovered-parked');
    h.page.publish(1, { playing: false, seek: 1, speed: 0, step: 0 });
    await h.drainLane();
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h1:installed');
    h.deliverToPage();
    h.page.frame();
    assert.equal(h.page.view(1).through, 1);

    h.page.publish(5, { playing: true, seek: 1, speed: 24, step: 4 });
    await h.drainLane();
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h5:installed');

    h.deliverToPage();
    h.page.publish(6, { playing: false, seek: 1, speed: 0, step: 1 });
    assert.equal(h.page.stats.rejected.uncovered, 1);
    assert.equal(h.page.view(1).through, 1);

    await h.drainLane();
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h1:installed');

    const paintsBefore = h.paints();
    h.page.publish(6, { playing: false, seek: 1, speed: 0, step: 0 });
    await h.drainLane();
    assert.equal(h.paints(), paintsBefore);
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h1:installed');
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('direct-surface rules: refusal of an old-generation entry after rebind restores installed pixels with installed model', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number; height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) { this.width = w; this.height = h; this.ctx = fakeContext2D().ctx; }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });
  try {
    const h = makeDirectHarness('test-rule-old-gen-binding');
    h.page.publish(1, { playing: false, seek: 1, speed: 0, step: 0 });
    await h.drainLane();
    h.deliverToPage();
    h.page.frame();
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h1:installed');

    h.page.publish(5, { playing: true, seek: 1, speed: 24, step: 4 });
    await h.drainLane();
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h5:installed');

    h.page.rebind(1);
    const nextGen = h.page.widgets.get(1)!.binding;
    h.lane.bind(1, { generation: nextGen, surfaces: [{ width: 300, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 5 } }, h.bindingModel('rebound'));

    h.deliverToPage();
    h.page.publish(6, { playing: false, seek: 1, speed: 0, step: 1 });
    assert.equal(h.page.stats.rejected.binding, 1);
    assert.equal(h.page.view(1).through, 1);

    await h.drainLane();
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h1:installed');
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('direct-surface rules: resize while awaiting a first reading restores held installed pixels with installed model', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number; height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) { this.width = w; this.height = h; this.ctx = fakeContext2D().ctx; }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });
  try {
    const h = makeDirectHarness('test-rule-resize-awaiting');
    h.page.publish(1, { playing: false, seek: 1, speed: 0, step: 0 });
    await h.drainLane();
    h.deliverToPage();
    h.page.frame();
    await h.drainLane();
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h1:installed');

    h.page.rebind(1);
    const nextGen = h.page.widgets.get(1)!.binding;
    h.lane.bind(1, { generation: nextGen, surfaces: [{ width: 360, height: 240, dpr: 1 }], offset: 0, pin: null, dated: { h: 100, c: 100 } }, h.bindingModel('rebound'));
    h.lane.setLaneSlots({
      width: 900,
      height: 400,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 40, y: 20, w: 360, h: 240, visible: true }],
    });
    assert.equal(h.surface.pixelAt(80, 60), TEST_THEME.surface);

    await h.drainLane();
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h1:installed');
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('direct-surface rules: lane skips painting a known uncovered replacement while paused', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number; height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) { this.width = w; this.height = h; this.ctx = fakeContext2D().ctx; }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });
  try {
    const h = makeDirectHarness('test-rule-skip-known-refusal');
    h.page.publish(1, { playing: false, seek: 1, speed: 0, step: 0 });
    await h.drainLane();
    h.deliverToPage();
    h.page.frame();
    await h.drainLane();
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h1:installed');

    const paintsBefore = h.paints();
    h.page.publish(6, { playing: false, seek: 1, speed: 0, step: 5 });
    await h.drainLane();
    assert.equal(h.paints(), paintsBefore);
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h1:installed');
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('direct-surface rules: dirty slot without kept picture stays blank while loading, paints on first entry, and counts restore failures', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number; height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) { this.width = w; this.height = h; this.ctx = fakeContext2D().ctx; }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });
  try {
    let failRestore = false;
    const h = makeDirectHarness('test-rule-no-kept-and-restore-failure');
    h.lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 10, c: 20 } });
    h.surface.ctx.fillStyle = 'pixel:stale-unaccepted';
    h.surface.ctx.fillRect(40, 20, 300, 200);
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:stale-unaccepted');
    h.page.publish(1, { playing: false, seek: 1, speed: 0, step: 0 });
    await h.drainLane();
    assert.equal(h.page.view(1).mode, 'reading');
    assert.equal(h.surface.pixelAt(80, 60), TEST_THEME.surface);
    assert.equal(h.paints(), 0);

    h.page.publish(10, { playing: false, seek: 1, speed: 0, step: 9 });
    await h.drainLane();
    h.deliverToPage();
    h.page.frame();
    await h.drainLane();
    assert.equal(h.page.view(1).mode, 'picture');
    assert.equal(h.surface.pixelAt(80, 60), 'pixel:h10:installed');

    const failKind = 'test-rule-restore-failure-counter';
    registerFamilyProjection({ family: 'bars', kind: failKind, project(v) { return { hour: v.hour }; } }, 'main');
    const failView: LaneView = createLaneView({
      family: 'bars',
      boundary: (_m, hour) => hour + 1,
      redraw: () => Infinity,
      status: () => 'picture',
      view: [],
      layout: (m: { hour: number }) => ({ hour: m.hour }),
      paint(ctx, frame) {
        if (failRestore) throw new Error('restore boom');
        ctx.fillStyle = `pixel:h${frame.hour}`;
        ctx.fillRect(0, 0, frame.surface.width, frame.surface.height);
        return false;
      },
      hit: () => null, tooltip: () => null, structure: () => ({}), mirror: null, audit: () => [], visibleArt: () => [],
    }, () => {});
    const presented: { id: number; widget: number; binding: number }[] = [];
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0, lanes: 1, views: () => failView, crop: async () => [],
      present(p) { for (const e of p.entries) presented.push({ id: e.id, widget: e.widget, binding: e.binding }); },
      release() {}, fail(r) { throw new Error(r); }, later() {}, clock: () => 0, budget: Infinity,
    });
    const surface = pixelCanvas(800, 300);
    lane.setLaneSurface(surface.canvas, true);
    lane.mount(1, [], 'bars', { family: 'bars', kind: failKind, view: 'main', options: {}, revisions: {}, sources: {}, scrollTop: 0, theme: TEST_THEME } satisfies LaneBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 5 } });
    lane.setLaneSlots({ width: 800, height: 300, dpr: 1, scrollTop: 0, scrollLeft: 0, originX: 0, originY: 0, surfaceColor: TEST_THEME.surface, clipPathFallback: false, slots: [{ widget: 1, x: 0, y: 0, w: 300, h: 200, visible: true }] });
    lane.frame({ type: 'frame', serial: 1, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    const acks = presented.splice(0);
    lane.frame({ type: 'frame', serial: 2, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: acks, refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();

    failRestore = true;
    lane.setLaneSlots({ width: 900, height: 300, dpr: 1, scrollTop: 0, scrollLeft: 0, originX: 0, originY: 0, surfaceColor: TEST_THEME.surface, clipPathFallback: false, slots: [{ widget: 1, x: 10, y: 0, w: 300, h: 200, visible: true }] });
    lane.frame({ type: 'frame', serial: 3, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    assert.equal(lane.stats.restoreFailures, 1);

    failRestore = false;
    lane.frame({ type: 'frame', serial: 4, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    assert.equal(surface.pixelAt(20, 20), 'pixel:h1');
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('R4-4: snapCanvasText wraps a reused context only once', () => {
  let transforms = 0;
  const ctx = {
    getTransform() { transforms++; return { a: 1, b: 0, c: 0, d: 1, e: 0, f: 0 }; },
    fillText() {},
  } as unknown as OffscreenCanvasRenderingContext2D;
  snapCanvasText(ctx);
  snapCanvasText(ctx);
  snapCanvasText(ctx);
  ctx.fillText('hi', 1.4, 2.6);
  assert.equal(transforms, 1);
});

test('R4-5: lazy lane creation builds zero workers for an empty page and one worker for a one-widget page on 64 threads', () => {
  const navDesc = Object.getOwnPropertyDescriptor(globalThis, 'navigator');
  Object.defineProperty(globalThis, 'navigator', { value: { hardwareConcurrency: 64 }, configurable: true });
  try {
    let created = 0;
    const messagesByWorker: unknown[][] = [];
    const client = { available: true, openFollow: async () => 'sess', product: async () => ({ data: { available: true, route: 1 } }) } as never;
    const host = {
      ...lanesHost(client, () => {
        created++;
        const log: unknown[] = [];
        messagesByWorker.push(log);
        return {
          onmessage: null,
          onerror: null,
          postMessage(m: { type: string; port?: MessagePort }) {
            log.push(m.type);
            if (m.type === 'init') m.port?.close();
          },
          terminate() {},
        } as unknown as Worker;
      }),
      frameRate: () => 60,
    };
    const clock = new PlaybackController();
    clock.setBounds({ firstHour: 0, lastHour: 100 });
    clock.seek(10);
    const pipeline = new Pipeline(clock, client, host);
    try {
      pipeline.prepare(['bars'], [['bars', 'a', 'main']]);
      assert.equal(created, 0);
      assert.equal(pipeline.createdLanes, 0);
      assert.equal(pipeline.prepared(), true);

      const w1 = pipeline.acquire('w1|0', { offset: 0, pin: null, preferredLane: 0 });
      assert.equal(created, 1);
      assert.equal(pipeline.createdLanes, 1);
      assert.deepEqual(messagesByWorker[0], ['init', 'prepare']);

      clock.seek(12);
      assert.equal(created, 1);
      assert.ok(messagesByWorker[0]!.includes('frame'));

      const w2 = pipeline.acquire('w2|0', { offset: 0, pin: null, preferredLane: 1 });
      assert.equal(created, 2);
      assert.equal(pipeline.createdLanes, 2);

      pipeline.drop('w2|0');
      const lane1Before = messagesByWorker[1]!.length;
      clock.seek(14);
      assert.equal(messagesByWorker[1]!.slice(lane1Before).filter(t => t === 'frame').length, 0);
      pipeline.drop('w1|0');
      void w1; void w2;
    } finally {
      pipeline.close();
    }
  } finally {
    if (navDesc) Object.defineProperty(globalThis, 'navigator', navDesc);
    else delete (globalThis as { navigator?: unknown }).navigator;
  }
});

test('direct-surface rules: cancelled seek during sibling crop wait clears unaccepted pixels without kept picture, restores kept picture when present, and repaints on return to cancelled hour', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number; height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) { this.width = w; this.height = h; this.ctx = fakeContext2D().ctx; }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });
  try {
    for (const mode of ['shared', 'glass'] as const) {
      for (const withKept of [false, true]) {
        const kind = `test-rule-cancelled-seek-${mode}-${withKept ? 'kept' : 'empty'}`;
        registerFamilyProjection({
          family: 'bars',
          kind,
          project(v) { return { hour: v.hour }; },
        }, 'main');
        const directFamily: LaneFamily<LaneBinding, { readonly hour: number }, { readonly hour: number }> = {
          family: 'bars',
          boundary: (_m, hour) => hour + 1,
          redraw: () => Infinity,
          status: () => 'picture',
          view: [],
          layout: m => ({ hour: m.hour }),
          paint(ctx, frame) {
            ctx.fillStyle = `pixel:h${frame.hour}`;
            ctx.fillRect(0, 0, frame.surface.width, frame.surface.height);
            return false;
          },
          hit: () => null, tooltip: () => null, structure: () => ({}), mirror: null, audit: () => [], visibleArt: () => [],
        };
        const directView: LaneView = createLaneView(directFamily, () => {});
        const bitmapView: LaneView = {
          paint(input, _state, content) {
            return {
              surfaces: [{ source: { width: 10, height: 10 }, content: content() }],
              fields: {},
              motion: false,
              nextChangeAt: input.hour + 1,
            };
          },
        };

        let releaseCrop: (() => void) | null = null;
        let cropStarted = false;
        const presented: Parameters<Page<Bitmapped & { close(): void }>['offer']>[0][] = [];
        const lane = new Lane<Bitmapped & { close(): void }>({
          lane: 0,
          lanes: 1,
          views: k => (k === 'bitmap' ? bitmapView : directView),
          crop(sources) {
            cropStarted = true;
            return new Promise(resolve => {
              releaseCrop = () => resolve(sources.map(s => ({ width: s.width, height: s.height, close() {} })));
            });
          },
          present(p) { presented.push(p); },
          release() {},
          fail(r) { throw new Error(r); },
          later() {},
          clock: () => 0,
          budget: Infinity,
        });

        const sharedCanvas = pixelCanvas(800, 300);
        const glassCanvas = pixelCanvas(300, 200);
        if (mode === 'shared') {
          lane.setLaneSurface(sharedCanvas.canvas, true);
          lane.setLaneSlots({
            width: 800, height: 300, dpr: 1, scrollTop: 0, scrollLeft: 0, originX: 0, originY: 0,
            surfaceColor: TEST_THEME.surface, clipPathFallback: false,
            slots: [{ widget: 1, x: 40, y: 20, w: 300, h: 200, visible: true }],
          });
        } else {
          lane.setCanvas(1, glassCanvas.canvas, false);
        }
        const readPixel = () => (mode === 'shared' ? sharedCanvas.pixelAt(80, 60) : glassCanvas.pixelAt(80, 60));
        const expectedClear = mode === 'shared' ? TEST_THEME.surface : 'cleared';

        lane.follow(10, 'followed');
        lane.bindRoute(10, 1, 1, 1, 0);
        lane.mount(1, [], 'bars', {
          family: 'bars', kind, view: 'main', options: {}, revisions: {}, sources: {}, scrollTop: 0, theme: TEST_THEME,
        } satisfies LaneBinding);
        lane.mount(2, [], 'bitmap');
        lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: withKept ? 1 : 20, c: 25 } });
        lane.bind(2, { generation: 1, surfaces: [{ width: 10, height: 10, dpr: 1 }], offset: 0, pin: null, dated: { h: 20, c: 25 } });

        const flush = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
        if (withKept) {
          lane.frame({ type: 'frame', serial: 1, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
          await flush();
          assert.equal(readPixel(), 'pixel:h1');
          const ack1 = presented.splice(0).flatMap(p => p.entries.map(e => ({ id: e.id, widget: e.widget, binding: e.binding })));
          lane.frame({ type: 'frame', serial: 2, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: ack1, refused: [] });
          await flush();
        }

        presented.length = 0;
        cropStarted = false;
        lane.frame({ type: 'frame', serial: 3, seek: 1, target: 20, now: 20, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
        await flush();
        assert.equal(cropStarted, true);
        assert.equal(readPixel(), 'pixel:h20');
        assert.equal(presented.length, 0);

        lane.bind(1, { generation: 1, surfaces: [{ width: 300, height: 200, dpr: 1 }], offset: 0, pin: null, dated: { h: 20, c: 25 } });
        lane.follow(11, 'followed');
        lane.bindRoute(11, 1, 1, 2, 0);
        releaseCrop!();
        await flush();
        assert.equal(presented.length, 0);

        lane.frame({ type: 'frame', serial: 4, seek: 2, target: 10, now: 10, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
        await flush();
        assert.equal(readPixel(), withKept ? 'pixel:h1' : expectedClear);

        lane.frame({ type: 'frame', serial: 5, seek: 2, target: 20, now: 20, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
        await flush();
        releaseCrop!();
        await flush();
        assert.equal(readPixel(), 'pixel:h20');
        assert.ok(presented.some(p => p.entries.some(e => e.widget === 1 && e.lo === 20)));
      }
    }
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('SharedSurfaceHost aligns slots to viewport device pixels without double rounding at fractional DPR and excludes portaled bodies', () => {
  const origDoc = globalThis.document;
  const origHtmlCanvas = (globalThis as { HTMLCanvasElement?: unknown }).HTMLCanvasElement;
  const origMO = globalThis.MutationObserver;
  const origRO = globalThis.ResizeObserver;
  const origRaf = globalThis.requestAnimationFrame;
  const origCancelRaf = globalThis.cancelAnimationFrame;
  const origWindow = (globalThis as { window?: unknown }).window;
  const origGetComputedStyle = globalThis.getComputedStyle;

  const sentSlots: { lane: number; payload: LaneSlotsPayload }[] = [];
  const dpr = 1.25;

  class FakeElement {
    className = '';
    dataset: Record<string, string> = {};
    style: Record<string, string> = {};
    children: FakeElement[] = [];
    firstChild: FakeElement | null = null;
    scrollTop = 0;
    scrollLeft = 0;
    width = 0;
    height = 0;
    offsetWidth = 0;
    offsetHeight = 0;
    clientWidth = 0;
    clientHeight = 0;
    clientLeft = 0;
    clientTop = 0;
    rect = { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 };
    contained = new Set<FakeElement>();
    setAttribute() {}
    appendChild(child: FakeElement) {
      this.children.push(child);
      if (!this.firstChild) this.firstChild = child;
      return child;
    }
    insertBefore(child: FakeElement) {
      this.children.unshift(child);
      this.firstChild = child;
      return child;
    }
    addEventListener() {}
    removeEventListener() {}
    remove() {}
    closest() { return null; }
    contains(el: FakeElement) { return this.contained.has(el); }
    getBoundingClientRect() { return this.rect; }
    transferControlToOffscreen() { return fakeOffscreenCanvas().canvas; }
  }

  const docEl = new FakeElement();
  docEl.dataset.backdrop = 'solid';
  const created: FakeElement[] = [];

  Object.defineProperty(globalThis, 'HTMLCanvasElement', { value: FakeElement, configurable: true });
  Object.defineProperty(globalThis, 'document', {
    value: {
      documentElement: docEl,
      createElement() {
        const el = new FakeElement();
        created.push(el);
        return el;
      },
    },
    configurable: true,
  });
  Object.defineProperty(globalThis, 'window', { value: { devicePixelRatio: dpr }, configurable: true });
  Object.defineProperty(globalThis, 'getComputedStyle', {
    value: () => ({ getPropertyValue: () => '#0a0a0a', backgroundColor: '#0a0a0a' }),
    configurable: true,
  });
  Object.defineProperty(globalThis, 'ResizeObserver', { value: undefined, configurable: true });
  Object.defineProperty(globalThis, 'requestAnimationFrame', { value: (fn: FrameRequestCallback) => { fn(0); return 1; }, configurable: true });
  Object.defineProperty(globalThis, 'cancelAnimationFrame', { value: () => {}, configurable: true });
  Object.defineProperty(globalThis, 'MutationObserver', { value: undefined, configurable: true });

  try {
    const fakePipeline = {
      laneCount: 4,
      transferLaneSurface() {},
      sendLaneSlots(lane: number, payload: LaneSlotsPayload) { sentSlots.push({ lane, payload }); },
    };
    const scrollEl = new FakeElement();
    scrollEl.rect = { left: 0, top: 48, right: 1522, bottom: 827, width: 1522, height: 779 };
    const host = new SharedSurfaceHost(fakePipeline as never, scrollEl as unknown as HTMLElement);
    const layerEl = created[0]!;
    layerEl.rect = { left: 6, top: 56.4, right: 1522, bottom: 56.4, width: 1516, height: 0 };

    const bodyEl = new FakeElement();
    bodyEl.rect = { left: 238, top: 119.6, right: 764, bottom: 395.6, width: 526, height: 276 };
    bodyEl.offsetWidth = 526;
    bodyEl.offsetHeight = 276;
    bodyEl.clientWidth = 526;
    bodyEl.clientHeight = 276;
    const frameEl = new FakeElement();
    scrollEl.contained.add(bodyEl);

    const portalBody = new FakeElement();
    portalBody.rect = { left: 300, top: 200, right: 600, bottom: 400, width: 300, height: 200 };
    const portalFrame = new FakeElement();

    host.register(1, 0, bodyEl as unknown as HTMLElement, frameEl as unknown as HTMLElement);
    host.register(2, 1, portalBody as unknown as HTMLElement, portalFrame as unknown as HTMLElement);

    assert.equal(frameEl.dataset.sharedSurface, '1');
    assert.equal(portalFrame.dataset.sharedSurface, undefined);

    const canvasEl = created.find(el => el.className === 'dash-shared-canvas')!;
    const leftCss = parseFloat(canvasEl.style.left);
    const topCss = parseFloat(canvasEl.style.top);
    const txMatch = /translate\(([-0-9.]+)px,\s*([-0-9.]+)px\)/.exec(canvasEl.style.transform || '');
    const tx = txMatch ? parseFloat(txMatch[1]!) : 0;
    const ty = txMatch ? parseFloat(txMatch[2]!) : 0;
    const last = sentSlots.filter(s => s.lane === 0).at(-1)!.payload;
    const slot = last.slots[0]!;

    const actualDevX = (layerEl.rect.left + leftCss + tx + slot.x) * dpr;
    const actualDevY = (layerEl.rect.top + topCss + ty + slot.y) * dpr;
    assert.equal(Math.round(actualDevX * 1000) / 1000, Math.round(bodyEl.rect.left * dpr));
    assert.equal(Math.round(actualDevY * 1000) / 1000, Math.round(bodyEl.rect.top * dpr));
    assert.equal(Math.round(slot.w * dpr), Math.floor(bodyEl.clientWidth * dpr + 1e-3));
    assert.equal(Math.round(slot.h * dpr), Math.floor(bodyEl.clientHeight * dpr + 1e-3));

    (globalThis.window as { devicePixelRatio: number }).devicePixelRatio = 1;
    layerEl.rect = { left: 0, top: 0, right: 1522, bottom: 0, width: 1522, height: 0 };
    const hScrollBody = new FakeElement();
    hScrollBody.rect = { left: 20, top: 10.75, right: 220, bottom: 114.75, width: 200, height: 104 };
    hScrollBody.offsetWidth = 200;
    hScrollBody.offsetHeight = 104;
    hScrollBody.clientWidth = 200;
    hScrollBody.clientHeight = 100;
    const hScrollFrame = new FakeElement();
    scrollEl.contained.add(hScrollBody);
    host.register(3, 0, hScrollBody as unknown as HTMLElement, hScrollFrame as unknown as HTMLElement);
    const hCanvasEl = created.find(el => el.className === 'dash-shared-canvas')!;
    const hTopCss = parseFloat(hCanvasEl.style.top);
    const hTxMatch = /translate\(([-0-9.]+)px,\s*([-0-9.]+)px\)/.exec(hCanvasEl.style.transform || '');
    const hTy = hTxMatch ? parseFloat(hTxMatch[2]!) : 0;
    const hLast = sentSlots.filter(s => s.lane === 0).at(-1)!.payload;
    const hSlot = hLast.slots.find(s => s.widget === 3)!;
    assert.equal(hTopCss + hTy + hSlot.y, 11);
    assert.equal(hTopCss + hTy + hSlot.y + hSlot.h, 110);
    host.dispose();
  } finally {
    Object.defineProperty(globalThis, 'document', { value: origDoc, configurable: true });
    Object.defineProperty(globalThis, 'HTMLCanvasElement', { value: origHtmlCanvas, configurable: true });
    Object.defineProperty(globalThis, 'MutationObserver', { value: origMO, configurable: true });
    Object.defineProperty(globalThis, 'ResizeObserver', { value: origRO, configurable: true });
    Object.defineProperty(globalThis, 'requestAnimationFrame', { value: origRaf, configurable: true });
    Object.defineProperty(globalThis, 'cancelAnimationFrame', { value: origCancelRaf, configurable: true });
    Object.defineProperty(globalThis, 'window', { value: origWindow, configurable: true });
    Object.defineProperty(globalThis, 'getComputedStyle', { value: origGetComputedStyle, configurable: true });
  }
});

test('S1: unmounting a shared widget at dpr 1.75 clears its full device-pixel clip box when lane-slots arrives after with unchanged bounds', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number; height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) { this.width = w; this.height = h; this.ctx = fakeContext2D().ctx; }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });
  try {
    const dpr = 1.75;
    const cssW = 300;
    const cssH = 120;
    const pc = pixelCanvas(Math.round(cssW * dpr), Math.round(cssH * dpr));
    const kind = 'test-s1-unmount-clear-box';
    registerFamilyProjection({ family: 'bars', kind, project(v) { return { hour: v.hour }; } }, 'main');
    const family: LaneFamily<LaneBinding, { readonly hour: number }, { readonly hour: number }> = {
      family: 'bars',
      boundary: (_m, hour) => hour + 1,
      redraw: () => Infinity,
      status: () => 'picture',
      view: [],
      layout: m => m,
      paint(pctx) {
        pctx.fillStyle = TEST_THEME.text;
        pctx.fillRect(0, 0, 80, 40);
        return false;
      },
      hit: () => null,
      tooltip: () => null,
      structure: () => ({}),
      mirror: null,
      audit: () => [],
      visibleArt: () => [],
    };
    const view: LaneView = createLaneView(family, () => {});
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present() {},
      release() {},
      fail(reason) { throw new Error(reason); },
      later() {},
      clock: () => 0,
      budget: Infinity,
    });
    lane.setLaneSurface(pc.canvas, true);
    const binding = {
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      sources: {},
      scrollTop: 0,
      theme: TEST_THEME,
    } satisfies LaneBinding;
    lane.mount(1, [], 'bars', binding);
    lane.mount(2, [], 'bars', binding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 80, height: 40, dpr }], offset: 0, pin: null, dated: { h: 1, c: 5 } });
    lane.bind(2, { generation: 1, surfaces: [{ width: 80, height: 40, dpr }], offset: 0, pin: null, dated: { h: 1, c: 5 } });
    const bx1 = 61 / dpr;
    const slot1 = { widget: 1, x: bx1, y: 12, w: 80, h: 40, bx: bx1, by: 8, bw: 80, bh: 48, visible: true };
    const slot2 = { widget: 2, x: 160, y: 12, w: 80, h: 40, bx: 156, by: 8, bw: 88, bh: 48, visible: true };
    lane.setLaneSlots({
      width: cssW,
      height: cssH,
      dpr,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [slot1, slot2],
    });
    lane.frame({ type: 'frame', serial: 1, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();

    const boxTop = Math.round(slot1.by * dpr);
    const boxBottom = Math.round((slot1.by + slot1.bh) * dpr);
    const bodyY = Math.round(slot1.y * dpr) + 5;
    const siblingX = Math.round(slot2.x * dpr) + 5;
    assert.equal(pc.pixelAt(61, bodyY), TEST_THEME.text);
    assert.equal(pc.pixelAt(200, bodyY), TEST_THEME.text);
    assert.equal(pc.pixelAt(siblingX, bodyY), TEST_THEME.text);

    lane.unmount(1);
    lane.setLaneSlots({
      width: cssW,
      height: cssH,
      dpr,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [slot2],
    });
    for (let i = 0; i < 6; i++) await Promise.resolve();

    for (let py = boxTop; py < boxBottom; py++) {
      for (let px = 61; px < 201; px++) {
        assert.equal(pc.pixelAt(px, py), 'cleared', `pixel (${px},${py}) in old unmounted box must be cleared`);
      }
    }
    assert.equal(pc.pixelAt(siblingX, bodyY), TEST_THEME.text, 'surviving sibling stays painted');
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('S2: a one-device-pixel move at dpr 3 is neither stillSame nor preservedByBlit', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number; height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) { this.width = w; this.height = h; this.ctx = fakeContext2D().ctx; }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });
  try {
    const dpr = 3;
    const cssW = 200;
    const cssH = 100;
    const pc = pixelCanvas(cssW * dpr, cssH * dpr);
    const kind = 'test-s2-dpr3-one-dev-px';
    registerFamilyProjection({ family: 'bars', kind, project(v) { return { hour: v.hour }; } }, 'main');
    let paints = 0;
    const family: LaneFamily<LaneBinding, { readonly hour: number }, { readonly hour: number }> = {
      family: 'bars',
      boundary: (_m, hour) => hour + 10,
      redraw: () => Infinity,
      status: () => 'picture',
      view: [],
      layout: m => m,
      paint(pctx) {
        paints++;
        pctx.fillStyle = TEST_THEME.text;
        pctx.fillRect(0, 0, 50, 40);
        return false;
      },
      hit: () => null,
      tooltip: () => null,
      structure: () => ({}),
      mirror: null,
      audit: () => [],
      visibleArt: () => [],
    };
    const view: LaneView = createLaneView(family, () => {});
    const presented: { id: number; widget: number; binding: number }[] = [];
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present(p) {
        for (const e of p.entries) presented.push({ id: e.id, widget: e.widget, binding: e.binding });
      },
      release() {},
      fail(reason) { throw new Error(reason); },
      later() {},
      clock: () => 0,
      budget: Infinity,
    });
    lane.setLaneSurface(pc.canvas, true);
    lane.mount(1, [], 'bars', {
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      sources: {},
      scrollTop: 0,
      theme: TEST_THEME,
    } satisfies LaneBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 50, height: 40, dpr }], offset: 0, pin: null, dated: { h: 1, c: 10 } });

    lane.setLaneSlots({
      width: cssW,
      height: cssH,
      dpr,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 10, y: 10, w: 50, h: 40, bx: 10, by: 10, bw: 50, bh: 40, visible: true }],
    });
    lane.frame({ type: 'frame', serial: 1, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    assert.equal(paints, 1);
    assert.equal(pc.pixelAt(30, 35), TEST_THEME.text);

    const installed1 = presented.splice(0);
    lane.setLaneSlots({
      width: cssW,
      height: cssH,
      dpr,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 31 / dpr, y: 10, w: 50, h: 40, bx: 31 / dpr, by: 10, bw: 50, bh: 40, visible: true }],
    });
    assert.equal(pc.pixelAt(30, 35), 'cleared', 'old left device column must be cleared on 1-dev-px move at dpr 3');
    lane.frame({ type: 'frame', serial: 2, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: installed1, refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    assert.equal(paints, 2, '1-dev-px move at dpr 3 must not pass stillSame');

    const installed2 = presented.splice(0);
    lane.setLaneSlots({
      width: cssW,
      height: cssH,
      dpr,
      scrollTop: 0,
      scrollLeft: 10 / dpr,
      originX: 10 / dpr,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 22 / dpr, y: 10, w: 50, h: 40, bx: 22 / dpr, by: 10, bw: 50, bh: 40, visible: true }],
    });
    lane.frame({ type: 'frame', serial: 3, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: installed2, refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    assert.equal(paints, 3, 'scroll with 1-dev-px drift at dpr 3 must not pass preservedByBlit');
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('S2: preservedByBlit checks full clip box containment so a scrolled-in scrollbar strip repaints', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number; height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) { this.width = w; this.height = h; this.ctx = fakeContext2D().ctx; }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });
  try {
    const kind = 'test-s2-blit-clip-box';
    registerFamilyProjection({ family: 'bars', kind, project(v) { return { hour: v.hour }; } }, 'main');
    let paints = 0;
    const family: LaneFamily<LaneBinding, { readonly hour: number }, { readonly hour: number }> = {
      family: 'bars',
      boundary: (_m, hour) => hour + 10,
      redraw: () => Infinity,
      status: () => 'picture',
      view: [],
      layout: m => m,
      paint() {
        paints++;
        return false;
      },
      hit: () => null,
      tooltip: () => null,
      structure: () => ({}),
      mirror: null,
      audit: () => [],
      visibleArt: () => [],
    };
    const view: LaneView = createLaneView(family, () => {});
    const presented: { id: number; widget: number; binding: number }[] = [];
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present(p) {
        for (const e of p.entries) presented.push({ id: e.id, widget: e.widget, binding: e.binding });
      },
      release() {},
      fail(reason) { throw new Error(reason); },
      later() {},
      clock: () => 0,
      budget: Infinity,
    });
    const sharedCanvas = fakeOffscreenCanvas(200, 100);
    lane.setLaneSurface(sharedCanvas.canvas, true);
    lane.mount(1, [], 'bars', {
      family: 'bars',
      kind,
      view: 'main',
      options: {},
      sources: {},
      scrollTop: 0,
      theme: TEST_THEME,
    } satisfies LaneBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 70, height: 76, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 10 } });
    lane.setLaneSlots({
      width: 200,
      height: 100,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 10, y: 20, w: 70, h: 76, bx: 10, by: 20, bw: 70, bh: 84, visible: true }],
    });
    lane.frame({ type: 'frame', serial: 1, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    assert.equal(paints, 1);

    const installed = presented.splice(0);
    lane.setLaneSlots({
      width: 200,
      height: 100,
      dpr: 1,
      scrollTop: 10,
      scrollLeft: 0,
      originX: 0,
      originY: 10,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 10, y: 10, w: 70, h: 76, bx: 10, by: 10, bw: 70, bh: 84, visible: true }],
    });
    lane.frame({ type: 'frame', serial: 2, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed, refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();
    assert.equal(paints, 2, 'slot whose clip box scrolled into view must repaint instead of preserving by blit');
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});

test('S3: shared table paint starts background strips at effective clipped surface width and height', async () => {
  const origOffscreen = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  Object.defineProperty(globalThis, 'OffscreenCanvas', {
    value: class {
      width: number; height: number;
      private readonly ctx: OffscreenCanvasRenderingContext2D;
      constructor(w: number, h: number) { this.width = w; this.height = h; this.ctx = fakeContext2D().ctx; }
      getContext() { return this.ctx; }
    },
    configurable: true,
  });
  try {
    const kind = 'test-s3-table-scaled-strips';
    registerFamilyProjection({ family: 'table', kind, project(v) { return { hour: v.hour }; } }, 'main');
    const family: LaneFamily<LaneBinding, { readonly hour: number }, { readonly hour: number }> = {
      family: 'table',
      boundary: (_m, hour) => hour + 1,
      redraw: () => Infinity,
      status: () => 'picture',
      view: [],
      layout: m => m,
      paint() { return false; },
      hit: () => null,
      tooltip: () => null,
      structure: () => ({}),
      mirror: null,
      audit: () => [],
      visibleArt: () => [],
    };
    const view: LaneView = createLaneView(family, () => {});
    const lane = new Lane<Bitmapped & { close(): void }>({
      lane: 0,
      lanes: 1,
      views: () => view,
      crop: async () => [],
      present() {},
      release() {},
      fail(reason) { throw new Error(reason); },
      later() {},
      clock: () => 0,
      budget: Infinity,
    });
    const sharedCanvas = fakeOffscreenCanvas(400, 300);
    lane.setLaneSurface(sharedCanvas.canvas, true);
    lane.mount(1, [], 'table', {
      family: 'table',
      kind,
      view: 'main',
      options: {},
      sources: {},
      scrollTop: 0,
      theme: TEST_THEME,
    } satisfies LaneBinding);
    lane.bind(1, { generation: 1, surfaces: [{ width: 240, height: 160, dpr: 1 }], offset: 0, pin: null, dated: { h: 1, c: 5 } });
    lane.setLaneSlots({
      width: 400,
      height: 300,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: TEST_THEME.surface,
      clipPathFallback: false,
      slots: [{ widget: 1, x: 10, y: 10, w: 300, h: 200, bx: 10, by: 10, bw: 304, bh: 204, visible: true }],
    });
    sharedCanvas.ops.length = 0;
    lane.frame({ type: 'frame', serial: 1, seek: 1, target: 1, now: 1, exact: true, speed: 0, inspect: [], installed: [], refused: [] });
    for (let i = 0; i < 6; i++) await Promise.resolve();

    assert.ok(sharedCanvas.ops.includes('fillRect:240,0,64,204'), 'right strip starts at min(surface.width, slot.w)');
    assert.ok(sharedCanvas.ops.includes('fillRect:0,160,304,44'), 'bottom strip starts at min(surface.height, slot.h)');
    assert.ok(!sharedCanvas.ops.includes('fillRect:0,0,304,204'), 'table incremental area is not pre-filled');
  } finally {
    Object.defineProperty(globalThis, 'OffscreenCanvas', { value: origOffscreen, configurable: true });
  }
});
