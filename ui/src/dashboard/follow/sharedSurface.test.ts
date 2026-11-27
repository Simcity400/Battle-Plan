import assert from 'node:assert/strict';
import test from 'node:test';
import { Lane, type Bitmapped, type LaneView } from '../../draw/lane.ts';
import type { LaneSlotsPayload } from '../../draw/laneMessages.ts';
import type { FamilyTheme } from '../../families/contract.ts';
import { createLaneView, type LaneBinding, type LaneFamily } from '../../families/laneRuntime.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { PAGES } from '../pages.ts';
import { computeSharedLaneGeometry, computeSpatialLaneMap, isImageBackdropActive, SharedSurfaceHost } from './sharedSurface.ts';
import { laneCount } from './useFollowed.ts';

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

test('laneCount derives from hardwareConcurrency without an artificial 4-lane cap and spatial partition bounds by widget count', () => {
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
    assert.equal(laneCount(), 6);
    setHw(16);
    assert.equal(laneCount(), 14);
  } finally {
    if (navDesc) Object.defineProperty(globalThis, 'navigator', navDesc);
    else delete (globalThis as { navigator?: unknown }).navigator;
  }

  const three = [
    { id: 'a', x: 0, y: 0, w: 6, h: 8 },
    { id: 'b', x: 6, y: 0, w: 6, h: 8 },
    { id: 'c', x: 0, y: 8, w: 12, h: 8 },
  ];
  const part = computeSpatialLaneMap(three, 14);
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

test('direct shared and worker targets paint request.now instead of lookahead request.target, repaint across in-flight slot moves, and repaint on shared-to-worker fallback', async () => {
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
          target: ctx === sharedCanvas.ctx ? 'shared' : 'worker',
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
    assert.deepEqual(painted, [{ hour: 1, target: 'shared' }]);

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
    assert.deepEqual(painted.at(-1), { hour: 1, target: 'shared' });

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
    await sendFrame(3, 1, 3, true);
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
