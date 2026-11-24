import assert from 'node:assert/strict';
import test from 'node:test';
import type { ReactElement } from 'react';
import { createServer } from 'vite';
import type { PlotFrame } from './frame';
import type { FrameTask } from './scheduler';
import type { TimeChartProps } from './TimeChart';
import type { EventBadge } from './badges';
import type { TooltipContent } from './tooltip';

type Effect = { deps?: readonly unknown[]; run: () => void | (() => void); cleanup?: () => void };

function styleOf() {
  const values = new Map<string, string>();
  return { getPropertyValue: (name: string) => values.get(name) ?? '', setProperty: (name: string, value: string) => { values.set(name, value); } };
}

test('actual TimeChart observes pointer/keyboard hours, visible badges and the owning consumer', async () => {
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  const listeners = new Map<string, (event: unknown) => void>();
  let cursor = 0, visible = true, task: FrameTask | undefined, frame: PlotFrame | undefined, paints = 0;
  const fixture = {
    reducedMotion: true,
    shownTooltip: null as TooltipContent | null,
    heldTooltip: null as { owner: string } | null,
    tooltipRevision: 0,
    useId: () => 'time-chart-observer',
    useRef(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, { current: initial });
      return slots.get(index);
    },
    useState(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, initial);
      return [slots.get(index), (next: unknown) => slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next)];
    },
    useEffect(run: Effect['run'], deps?: readonly unknown[]) {
      const index = cursor++, before = effects.get(index);
      if (!deps || !before?.deps || before.deps.length !== deps.length || before.deps.some((value, i) => !Object.is(value, deps[i])))
        pending.set(index, { run, deps });
    },
    commit() {
      for (const [index, effect] of pending) {
        effects.get(index)?.cleanup?.();
        const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined;
        effects.set(index, effect);
      }
      pending.clear();
    },
    unmount() { for (const effect of effects.values()) effect.cleanup?.(); effects.clear(); slots.clear(); pending.clear(); },
    scheduler: { add(next: FrameTask) { task = next; return () => { task = undefined; }; }, wake() {},
      setVisible(_task: FrameTask, value: boolean) { visible = value; } },
  };
  const context = { setTransform() {}, clearRect() {}, save() {}, restore() {}, beginPath() {}, moveTo() {}, lineTo() {},
    closePath() {}, fill() {}, stroke() {}, arc() {}, fillText() {}, setLineDash() {}, drawImage() {} };
  let layoutReads = 0;
  const element = { get clientWidth() { layoutReads++; return 500; }, clientHeight: 240, width: 500, height: 240, style: styleOf(), focus() {}, setPointerCapture() {},
    getContext: () => context, getBoundingClientRect: () => ({ left: 0, top: 0, width: 500, height: 240 }),
    addEventListener: (type: string, callback: (event: unknown) => void) => listeners.set(type, callback),
    removeEventListener: (type: string) => listeners.delete(type) };
  const host = globalThis as typeof globalThis & { __timeChartObserver?: typeof fixture };
  host.__timeChartObserver = fixture;
  const globals = new Map(['window', 'ResizeObserver', 'document'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const replacements: Record<string, string> = {
    '/charts/scheduler.ts': 'export const scheduler = globalThis.__timeChartObserver.scheduler;',
    '/charts/liveMotion.ts': 'export const useMotionNow = () => given => given;',
    '/charts/tokens.ts': `export const CHART_LAYOUT = { right: 14 };
      export const readTokens = () => ({ fontSize: { small: 11 }, motion: 'snappy', reducedMotion: globalThis.__timeChartObserver.reducedMotion });
      export const subscribeTokens = () => () => {};`,
    '/charts/draw.ts': `export const clipToPlot = () => {}; export const dayLabel = String;
      export const drawAxes = () => {}; export const drawCrosshair = () => {}; export const drawPlayhead = () => {};
      export const drawSelection = () => {}; export const VALUE_LABEL_GAP = 6; export const valueLabelWidth = () => 0;`,
    '/charts/tooltip.ts': `export const hideTooltip = () => { globalThis.__timeChartObserver.shownTooltip = null; };
      export const showTooltip = (_owner, content) => { globalThis.__timeChartObserver.shownTooltip = content; };
      export const tooltipStore = { get: () => globalThis.__timeChartObserver.heldTooltip, revision: () => globalThis.__timeChartObserver.tooltipRevision };
      export const tooltipText = content => content.spoken ?? content.title;`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'time-chart-observer-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const key = Object.keys(replacements).find(key => path.endsWith(key));
        if (key) return replacements[key];
        if (path.endsWith('/charts/TimeChart.tsx')) return source.replace("import { useEffect, useId, useRef, useState } from 'react';",
          'const { useEffect, useId, useRef, useState } = globalThis.__timeChartObserver;');
      },
    }] });
  try {
    const { default: Chart } = await vite.ssrLoadModule('/ui/src/charts/TimeChart.tsx') as typeof import('./TimeChart.tsx');
    Object.defineProperty(globalThis, 'window', { configurable: true, value: { devicePixelRatio: 1 } });
    Object.defineProperty(globalThis, 'ResizeObserver', { configurable: true, value: class { observe() {} disconnect() {} } });
    const seen: [string, number | null][] = [], described: number[] = [];
    const observe = (name: string) => (hour: number | null) => { seen.push([name, hour]); };
    const props: TimeChartProps = { label: 'Synthetic shares', extent: [0, 100], branchStart: 0,
      playhead: 90, playback: 'cursor', axes: { xFrom: null, xTo: null, yMin: 0, yMax: 1, log: false },
      layers: [{ draw: (_ctx, drawn) => { frame = drawn; paints++; } }], active: true, revision: 1,
      keyHours: [0, 10, 80, 100], snap: () => 37.9, pickOwner: {}, onHover: observe('A'),
      tooltip: hour => { described.push(hour); return { title: String(hour), rows: [] }; } };
    type Canvas = { ref: { current: unknown }; onKeyDown: (event: unknown) => void };
    const render = (commit = true) => {
      cursor = 0;
      const tree = Chart(props) as ReactElement<{ children: ReactElement<Canvas>[] }>;
      const canvas = tree.props.children[0].props;
      canvas.ref.current = element;
      if (commit) fixture.commit();
      return canvas;
    };
    let frameTime = 0;
    const draw = (now = frameTime) => { frameTime = now; if (visible) task!.frame({ now, dt: 16 }); };
    const pointer = () => listeners.get('pointermove')!({ clientX: frame!.toX(50), clientY: frame!.plotTop + 10 });
    const key = (canvas: Canvas, name: string) => canvas.onKeyDown({ key: name, preventDefault() {} });
    let canvas = render(); draw(); pointer(); draw();
    assert.deepEqual(seen.at(-1), ['A', null]); assert.equal(described.at(-1), 37);
    draw(100);
    assert.deepEqual(seen.at(-1), ['A', 37]); assert.equal(described.at(-1), 37);
    assert.equal(paints, 1, 'hovering repaints only the overlay: the series paint once');
    const reads = layoutReads; draw(); draw();
    assert.equal(layoutReads, reads, 'frames reuse the canvas size measured until it resizes');
    const refreshedAt = seen.length;
    props.onHover = observe('B'); canvas = render(); draw();
    assert.equal(seen.length, refreshedAt, 'a new callback for the same owner is not told the unchanged hour again');
    key(canvas, 'End'); draw();
    assert.deepEqual(seen.at(-1), ['B', 80]); assert.equal(described.at(-1), 80);
    assert.ok(seen.slice(refreshedAt).every(([name]) => name === 'B'), 'callback refresh keeps the same owner');
    const numericTooltip = props.tooltip;
    const simultaneous = Array.from({ length: 5000 }, (_, index) => ({ hour: 80, glyph: '\u00d7', label: `opaque actor ${index} capitulated` }));
    const early = { hour: 10, glyph: '\u00d7', label: 'earlier country capitulated' };
    const late = { hour: 100, glyph: '\u00d7', label: 'future country capitulated' };
    const badgeReads: { hour: number; badges: readonly EventBadge[] }[] = [];
    props.badges = [late, ...simultaneous, early];
    props.tooltip = (hour, _frame, badges) => {
      badgeReads.push({ hour, badges });
      return { title: String(hour), rows: badges.map(badge => ({ label: badge.label })),
        spoken: [String(hour), ...badges.map(badge => badge.label)].join('; ') };
    };
    canvas = render(); draw();
    const reached = badgeReads.at(-1)!;
    assert.equal(reached.hour, 80);
    assert.deepEqual(reached.badges, simultaneous, 'the real keyboard handler supplies every same-hour event in source order');
    assert.equal(fixture.shownTooltip?.rows.length, 5000);
    cursor = 0;
    const spokenTree = Chart(props) as ReactElement<{ children: ReactElement<{ children: string }>[] }>;
    assert.match(spokenTree.props.children[2].props.children, /opaque actor 4999 capitulated/);
    fixture.commit();
    fixture.heldTooltip = { owner: 'other chart' }; fixture.shownTooltip = null;
    props.revision = 3; canvas = render(); draw();
    assert.equal(fixture.shownTooltip, null, 'a kept keyboard hour redraws without taking another chart\'s tooltip');
    key(canvas, 'End'); draw();
    assert.equal((fixture.shownTooltip as TooltipContent | null)?.rows.length, 5000, 'a fresh key takes the tooltip back');
    fixture.heldTooltip = null;
    props.revision = 2; canvas = render(); draw();
    assert.equal(badgeReads.at(-1)?.badges, reached.badges, 'redrawing the same source and hour reuses the indexed cohort');
    props.badges = [{ hour: 80, glyph: '\u00d7', label: 'replacement country capitulated' }, early, late];
    canvas = render(); draw();
    assert.deepEqual(badgeReads.at(-1)?.badges.map(badge => badge.label), ['replacement country capitulated'], 'a new source cannot retain the old cohort');
    props.axes = { ...props.axes, xFrom: 81, xTo: 100 }; canvas = render(); draw();
    assert.equal(fixture.shownTooltip, null, 'a keyboard hour outside the zoom has no event tooltip');
    key(canvas, 'Home'); draw();
    assert.deepEqual(badgeReads.at(-1), { hour: 81, badges: [] }, 'carried keyboard stops never acquire a clipped earlier event');
    props.axes = { ...props.axes, xFrom: null, xTo: null }; props.badges = [late, ...simultaneous, early];
    canvas = render(); draw(); key(canvas, 'End'); draw();
    props.playhead = 20; canvas = render(); draw();
    assert.equal(fixture.shownTooltip, null, 'events after the playhead stay hidden');
    props.afterPlayhead = true; canvas = render(); draw();
    assert.deepEqual(badgeReads.at(-1)?.badges, simultaneous, 'Show after the playhead restores only the focused hour');
    key(canvas, 'Home'); draw(); assert.deepEqual(badgeReads.at(-1), { hour: 0, badges: [] });
    key(canvas, 'ArrowRight'); draw(); assert.deepEqual(badgeReads.at(-1), { hour: 10, badges: [early] });
    key(canvas, 'End'); draw();
    props.badges = undefined; props.tooltip = numericTooltip; props.playhead = 90; props.afterPlayhead = undefined;
    canvas = render(); draw(); key(canvas, 'End'); draw();
    props.playhead = 20; canvas = render(); draw(); assert.deepEqual(seen.at(-1), ['B', null]);
    props.afterPlayhead = true; canvas = render(); draw(); assert.deepEqual(seen.at(-1), ['B', 80]);
    key(canvas, 'Escape'); draw(); assert.deepEqual(seen.at(-1), ['B', null]);
    pointer(); draw(); listeners.get('pointerleave')!({}); draw(); assert.deepEqual(seen.at(-1), ['B', null]);
    pointer(); draw(); draw(200); assert.deepEqual(seen.at(-1), ['B', 37]);
    props.pickOwner = {}; props.onHover = observe('C'); canvas = render(false);
    const handedOffAt = seen.length; draw();
    assert.deepEqual(seen.slice(handedOffAt), [['B', null], ['C', 37]], 'a frame before cleanup clears old facts before publishing new ones');
    fixture.commit(); assert.deepEqual(seen.at(-1), ['C', 37], 'old passive cleanup leaves the new consumer alone');
    props.active = false; props.onHover = observe('D'); canvas = render(false);
    const deactivatedAt = seen.length; draw(); assert.equal(seen.length, deactivatedAt, 'latest inactive props cannot publish before passive cleanup');
    fixture.commit();
    assert.ok(seen.some(([name, hour]) => name === 'C' && hour === null));
    assert.deepEqual(seen.at(-1), ['D', null]);
    const count = seen.length; draw(); assert.equal(seen.length, count, 'inactive charts do not publish moments');
    props.active = true; canvas = render(); draw(); assert.deepEqual(seen.at(-1), ['D', 37]);
    props.onHover = undefined; canvas = render(false);
    const beforeRemoval = seen.length; draw(); assert.equal(seen.length, beforeRemoval);
    fixture.commit(); assert.deepEqual(seen.at(-1), ['D', null], 'a frame before passive cleanup keeps the last notified consumer');
    props.onHover = observe('E'); canvas = render(); draw(); assert.deepEqual(seen.at(-1), ['E', 37]);
    props.onHover = undefined; canvas = render(); assert.deepEqual(seen.at(-1), ['E', null], 'normal removal clears its consumer');
    props.onHover = observe('F'); canvas = render(); draw(); assert.deepEqual(seen.at(-1), ['F', 37]);
    const picked: number[] = [];
    props.onPick = hour => { picked.push(hour); }; canvas = render(); draw();
    const press = (detail: number) => {
      const at = { clientX: frame!.toX(50), clientY: frame!.plotTop + 10, button: 0, pointerId: 1, detail, preventDefault() {} };
      listeners.get('pointerdown')!(at); listeners.get('pointerup')!(at);
    };
    press(1); assert.deepEqual(picked, [37]);
    press(2); assert.deepEqual(picked, [37]);
    const views: unknown[] = [];
    props.onViewChange = x => { views.push(x); }; canvas = render(); draw(); draw(); canvas = render(); draw();
    assert.deepEqual([...views], [[0, 100]]);
    props.onViewChange = x => { views.push(x); }; canvas = render(); draw(); draw();
    assert.deepEqual(views, [[0, 100], [0, 100]]);
    const navigation: string[] = [];
    props.onNavigateHour = name => { navigation.push(name); };
    canvas = render(); draw(); key(canvas, 'ArrowRight');
    pointer(); draw();
    props.keyboardHour = 80; props.keyboardRevision = 1; canvas = render(); draw();
    assert.deepEqual(seen.at(-1), ['F', 37], 'a delayed key reply cannot reclaim hover after pointer movement');
    key(canvas, 'ArrowRight');
    props.keyboardRevision = 2; canvas = render(); draw();
    assert.deepEqual(seen.at(-1), ['F', 80], 'a fresh key request still accepts the same returned hour');
    assert.deepEqual(navigation, ['ArrowRight', 'ArrowRight']);
    fixture.shownTooltip = null;
    key(canvas, 'End');
    fixture.heldTooltip = { owner: 'other chart' }; fixture.tooltipRevision++;
    props.keyboardRevision = 3; canvas = render(); draw();
    assert.equal(fixture.shownTooltip, null, 'a reply cannot take a tooltip another chart showed after its key');
    fixture.heldTooltip = null;
    key(canvas, 'End');
    props.keyboardRevision = 4; canvas = render();
    fixture.heldTooltip = { owner: 'other chart' }; fixture.tooltipRevision++;
    draw();
    assert.equal(fixture.shownTooltip, null, 'nor one shown between its reply and its frame');
    fixture.heldTooltip = null;
    key(canvas, 'End');
    fixture.heldTooltip = { owner: 'other chart' }; fixture.tooltipRevision++;
    draw();
    assert.equal(fixture.shownTooltip, null, 'nor, before its reply, one shown after its key');
    fixture.heldTooltip = null;
    fixture.unmount(); assert.deepEqual(seen.at(-1), ['F', null]);
    fixture.reducedMotion = false;
    props.axes = { ...props.axes, yMin: null, yMax: null };
    let balanceExtent: [number, number] | null = null;
    props.layers = [{ draw: (_ctx, drawn) => { frame = drawn; }, extent: () => balanceExtent }];
    render(); draw(); assert.deepEqual(frame!.y, [0, 1]);
    balanceExtent = [8, 20]; render();
    let animating = true, frames = 0;
    while (animating && frames++ < 300) animating = task!.frame({ now: frameTime += 16, dt: 16 }) === true;
    assert.equal(animating, false, 'the animation reaches a final scheduled frame');
    const preparedY = frame!.y;
    fixture.unmount(); render(); draw();
    assert.deepEqual(preparedY, frame!.y, 'settled scale has no residual sub-pixel displacement');
  } finally {
    fixture.unmount(); await vite.close(); delete host.__timeChartObserver;
    for (const [key, descriptor] of globals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key);
    }
  }
});

test('TimeChart keeps painted geometry for picks, preserves zoom across renderer switches, and falls back when worker fails', async () => {
  const slots = new Map<number, unknown>(), effects = new Map<number, Effect>(), pending = new Map<number, Effect>();
  const listeners = new Map<string, (event: unknown) => void>();
  let cursor = 0, task: FrameTask | undefined, workerDraws = 0, workerDetaches = 0, syncDraws = 0, asyncPending = false;
  let nextSlotFrameId = 1;
  let triggerFallback: ((reason: string) => void) | null = null;
  let onPainted: ((frameId: number) => void) | null = null;
  let lastPostedFrameId = 0;
  let lastPostedEndLabels: readonly { text: string; color: string; y: number }[] | undefined;
  let lastSyncFrame: PlotFrame | null = null;
  let overlayClears = 0;
  let canvasWidth = 400;
  const fixture = {
    useId: () => 'time-chart-worker-lifecycle',
    useRef(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, { current: initial });
      return slots.get(index);
    },
    useState(initial: unknown) {
      const index = cursor++;
      if (!slots.has(index)) slots.set(index, initial);
      return [slots.get(index), (next: unknown) => slots.set(index, typeof next === 'function' ? next(slots.get(index)) : next)];
    },
    useEffect(run: Effect['run'], deps?: readonly unknown[]) {
      const index = cursor++, before = effects.get(index);
      if (!deps || !before?.deps || before.deps.length !== deps.length || before.deps.some((value, i) => !Object.is(value, deps[i])))
        pending.set(index, { run, deps });
    },
    commit() {
      for (const [index, effect] of pending) {
        effects.get(index)?.cleanup?.();
        const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined;
        effects.set(index, effect);
      }
      pending.clear();
    },
    replayEffects() {
      for (const effect of effects.values()) {
        effect.cleanup?.();
        const cleanup = effect.run();
        effect.cleanup = typeof cleanup === 'function' ? cleanup : undefined;
      }
    },
    unmount() { for (const effect of effects.values()) effect.cleanup?.(); effects.clear(); slots.clear(); pending.clear(); },
    scheduler: {
      add(next: FrameTask) { task = next; return () => { task = undefined; asyncPending = false; }; },
      wake() {},
      setVisible() {},
      setAsyncPending(_task: FrameTask, value: boolean) { asyncPending = value; },
    },
    supportsTimeChartWorker: () => true,
    attachTimeChartWorker(_element: unknown, callbacks: ((reason: string) => void) | { onFallback: (reason: string) => void; onPainted?: (frameId: number) => void }) {
      if (typeof callbacks === 'function') {
        triggerFallback = callbacks;
        onPainted = null;
      } else {
        triggerFallback = callbacks.onFallback;
        onPainted = callbacks.onPainted ?? null;
      }
      return {
        id: 1,
        nextFrameId: () => nextSlotFrameId++,
        draw(scene: { frameId?: number; endLabels?: readonly { text: string; color: string; y: number }[] }) {
          workerDraws++;
          lastPostedFrameId = scene.frameId ?? workerDraws;
          lastPostedEndLabels = scene.endLabels;
          return true;
        },
        detach() { workerDetaches++; },
      };
    },
  };
  const context = { setTransform() {}, clearRect() {}, save() {}, restore() {}, beginPath() {}, moveTo() {}, lineTo() {},
    closePath() {}, fill() {}, stroke() {}, arc() {}, fillText() {}, setLineDash() {}, drawImage() {}, measureText: () => ({ width: 10 }) };
  const overlayCtx = { ...context, clearRect() { overlayClears++; } };
  const makeElement = () => ({
    get clientWidth() { return canvasWidth; },
    clientHeight: 200,
    get width() { return canvasWidth; },
    set width(_v: number) {},
    height: 200,
    style: styleOf(),
    focus() {},
    setPointerCapture() {},
    getContext: () => context,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: canvasWidth, height: 200 }),
    addEventListener: (type: string, callback: (event: unknown) => void) => listeners.set(type, callback),
    removeEventListener: (type: string) => listeners.delete(type),
  });
  const overlayEl = {
    get clientWidth() { return canvasWidth; },
    clientHeight: 200,
    width: 400,
    height: 200,
    style: styleOf(),
    getContext: () => overlayCtx,
  };
  const host = globalThis as typeof globalThis & { __timeChartFallback?: typeof fixture };
  host.__timeChartFallback = fixture;
  const globals = new Map(['window', 'ResizeObserver'].map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const replacements: Record<string, string> = {
    '/charts/scheduler.ts': 'export const scheduler = globalThis.__timeChartFallback.scheduler;',
    '/charts/liveMotion.ts': 'export const useMotionNow = () => given => given;',
    '/charts/tokens.ts': `export const CHART_LAYOUT = { right: 14 };
      export const readTokens = () => ({ fontSize: { small: 11 }, motion: 'snappy', reducedMotion: true });
      export const subscribeTokens = () => () => {};`,
    '/charts/draw.ts': `export const clipToPlot = () => {}; export const dayLabel = String;
      export const drawAxes = () => {}; export const drawCrosshair = () => {}; export const drawPlayhead = () => {};
      export const drawSelection = () => {}; export const VALUE_LABEL_GAP = 6; export const valueLabelWidth = () => 0;
      export const placeEndLabels = () => {};`,
    '/charts/tooltip.ts': `export const hideTooltip = () => {}; export const showTooltip = () => {};
      export const tooltipStore = { get: () => null, revision: () => 0 }; export const tooltipText = () => '';`,
    '/charts/timeChartWorkerClient.ts': `export const supportsTimeChartWorker = () => globalThis.__timeChartFallback.supportsTimeChartWorker();
      export const attachTimeChartWorker = (el, cb) => globalThis.__timeChartFallback.attachTimeChartWorker(el, cb);`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'time-chart-fallback-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        const key = Object.keys(replacements).find(key => path.endsWith(key));
        if (key) return replacements[key];
        if (path.endsWith('/charts/TimeChart.tsx')) return source.replace("import { useEffect, useId, useRef, useState } from 'react';",
          'const { useEffect, useId, useRef, useState } = globalThis.__timeChartFallback;');
      },
    }] });
  try {
    const { default: Chart } = await vite.ssrLoadModule('/ui/src/charts/TimeChart.tsx') as typeof import('./TimeChart.tsx');
    Object.defineProperty(globalThis, 'window', { configurable: true, value: { devicePixelRatio: 1 } });
    Object.defineProperty(globalThis, 'ResizeObserver', { configurable: true, value: class { observe() {} disconnect() {} } });
    const picked: number[] = [];
    const props: TimeChartProps = {
      label: 'Worker lifecycle test',
      extent: [0, 100],
      branchStart: 0,
      playhead: 100,
      playback: 'cursor',
      axes: { xFrom: 0, xTo: 100, yMin: 0, yMax: 100, log: false },
      layers: [{
        draw(_ctx, drawn) { syncDraws++; lastSyncFrame = drawn; },
        describe: () => ({ kind: 'line-summary', summary: undefined, color: '#ff0000' }),
      }],
      endLabels: (frame, until) => [{ id: 'a', text: `until=${until}@${Math.round(frame.x[1])}`, color: '#ff0000', y: frame.toY(50) }],
      formatValue: 'powerAxis',
      onPick: hour => { picked.push(Math.round(hour)); },
      active: true,
      revision: 1,
    };
    const render = () => {
      cursor = 0;
      const tree = Chart(props) as ReactElement<{ children: ReactElement<{ ref?: { current: unknown } }>[] }>;
      const canvasNode = tree.props.children[0];
      canvasNode.props.ref!.current = makeElement();
      const overlayNode = tree.props.children[3];
      if (overlayNode?.props?.ref) overlayNode.props.ref.current = overlayEl;
      fixture.commit();
      return canvasNode.key;
    };
    const clickAt = (clientX: number, clientY = 100) => {
      const at = { clientX, clientY, button: 0, pointerId: 1, detail: 1, preventDefault() {} };
      listeners.get('pointerdown')!(at);
      listeners.get('pointerup')!(at);
    };
    const clickMidpoint = () => {
      const midX = 44 + (400 - 44 - 14) / 2;
      clickAt(midX, 100);
    };

    const firstKey = render();
    assert.equal(firstKey, 'worker');
    task!.frame({ now: 0, dt: 16 });
    assert.equal(workerDraws, 1);
    assert.equal(asyncPending, true);
    assert.equal(lastPostedEndLabels?.[0]?.text, 'until=100@100');
    onPainted!(lastPostedFrameId);
    assert.equal(asyncPending, false);
    clickMidpoint();
    assert.deepEqual(picked, [50]);

    props.axes = { xFrom: 50, xTo: 100, yMin: 0, yMax: 100, log: false };
    render();
    task!.frame({ now: 16, dt: 16 });
    assert.equal(workerDraws, 2);
    assert.equal(asyncPending, true);
    clickMidpoint();
    assert.deepEqual(picked, [50, 50]);
    onPainted!(lastPostedFrameId);
    assert.equal(asyncPending, false);
    clickMidpoint();
    assert.deepEqual(picked, [50, 50, 75]);

    const pickedA: number[] = [];
    const pickedB: number[] = [];
    props.axes = { xFrom: 0, xTo: 100, yMin: 0, yMax: 100, log: false };
    props.pickOwner = { id: 'A' };
    props.onPick = hour => { pickedA.push(Math.round(hour)); };
    props.revision = 2;
    render();
    task!.frame({ now: 20, dt: 16 });
    const frameA = lastPostedFrameId;
    props.pickOwner = { id: 'B' };
    props.onPick = hour => { pickedB.push(Math.round(hour)); };
    props.revision = 3;
    render();
    onPainted!(frameA);
    clickMidpoint();
    assert.deepEqual(pickedA, []);
    assert.deepEqual(pickedB, []);

    const hoveredHours: (number | null)[] = [];
    const menuHours: number[] = [];
    let liveValue = 80;
    props.pickOwner = { id: 'live' };
    props.onPick = hour => { picked.push(Math.round(hour)); };
    props.onHover = hour => { hoveredHours.push(hour); };
    props.onContextMenu = hour => { menuHours.push(hour); };
    props.playback = 'draw-in';
    props.extent = [0, 100];
    props.playhead = 20;
    props.liveHead = { read: () => liveValue, subscribe: () => () => {} };
    props.axes = { xFrom: 0, xTo: 100, yMin: 0, yMax: 100, log: false };
    props.revision = 4;
    render();
    task!.frame({ now: 30, dt: 16 });
    onPainted!(lastPostedFrameId);
    const x50 = 44 + (400 - 44 - 14) * 0.5;
    clickAt(x50, 100);
    assert.equal(picked.at(-1), 50);

    liveValue = 20;
    props.revision = 5;
    render();
    task!.frame({ now: 40, dt: 16 });
    onPainted!(lastPostedFrameId);
    liveValue = 80;
    props.revision = 6;
    render();
    task!.frame({ now: 50, dt: 16 });
    listeners.get('pointermove')!({ clientX: x50, clientY: 100 });
    task!.frame({ now: 250, dt: 16 });
    task!.frame({ now: 450, dt: 16 });
    await Promise.resolve();
    assert.ok(hoveredHours.at(-1) == null || hoveredHours.at(-1)! <= 20);
    listeners.get('contextmenu')!({ clientX: x50, clientY: 100, preventDefault() {} });
    assert.equal(menuHours.at(-1), 20);
    onPainted!(lastPostedFrameId);
    listeners.get('pointermove')!({ clientX: x50, clientY: 100 });
    listeners.get('contextmenu')!({ clientX: x50, clientY: 100, preventDefault() {} });
    assert.equal(menuHours.at(-1), 50);

    const inFlightBeforeReplay = lastPostedFrameId + 1;
    props.revision = 7;
    render();
    task!.frame({ now: 260, dt: 16 });
    assert.equal(lastPostedFrameId, inFlightBeforeReplay);
    fixture.replayEffects();
    props.revision = 8;
    render();
    task!.frame({ now: 270, dt: 16 });
    const replayedFrameId = lastPostedFrameId;
    assert.ok(replayedFrameId > inFlightBeforeReplay);
    assert.equal(asyncPending, true);
    onPainted!(inFlightBeforeReplay);
    assert.equal(asyncPending, true);
    onPainted!(replayedFrameId);
    assert.equal(asyncPending, false);

    canvasWidth = 800;
    props.revision = 9;
    render();
    task!.frame({ now: 280, dt: 16 });
    const stretchedMidX = (44 + (400 - 44 - 14) * 0.5) * 2;
    const beforeResizeClick = picked.length;
    clickAt(stretchedMidX, 100);
    assert.equal(picked.length, beforeResizeClick + 1);
    assert.equal(picked.at(-1), 50);
    onPainted!(lastPostedFrameId);
    canvasWidth = 400;

    props.liveHead = undefined;
    props.playback = 'cursor';
    props.axes = { xFrom: null, xTo: null, yMin: 0, yMax: 100, log: false };
    props.extent = [0, 1000];
    props.playhead = 1000;
    props.revision = 10;
    render();
    task!.frame({ now: 300, dt: 16 });
    onPainted!(lastPostedFrameId);
    const midX = 44 + (400 - 44 - 14) / 2;
    listeners.get('wheel')!({ clientX: midX, clientY: 100, deltaY: -240, deltaMode: 0, preventDefault() {} });
    task!.frame({ now: 316, dt: 16 });
    onPainted!(lastPostedFrameId);

    listeners.get('pointerdown')!({ clientX: midX - 20, clientY: 80, button: 0, pointerId: 2, detail: 1, preventDefault() {} });
    listeners.get('pointermove')!({ clientX: midX + 20, clientY: 120 });
    task!.frame({ now: 332, dt: 16 });
    const clearsBeforeFallback = overlayClears;
    triggerFallback!('worker crashed');
    const secondKey = render();
    assert.equal(secondKey, 'sync');
    assert.ok(overlayClears > clearsBeforeFallback);
    assert.equal(workerDetaches, 2);
    task!.frame({ now: 348, dt: 16 });
    assert.equal(syncDraws, 1);
    const syncFrame = lastSyncFrame as PlotFrame | null;
    assert.ok(syncFrame !== null && syncFrame.x[1] - syncFrame.x[0] < 999);
  } finally {
    fixture.unmount(); await vite.close(); delete host.__timeChartFallback;
    for (const [key, descriptor] of globals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key);
    }
  }
});

test('TimeChart hands Stability widget endLabels using widget cutoff during liveHead playback', async () => {
  const { stabilityEndLabels } = await import('../widgets/stability/stabilityData.ts');
  const { END_LABEL_HEIGHT } = await import('./draw.ts');
  const { plotFrame } = await import('./frame.ts');
  const { summaryGeometry } = await import('./chartSummary.ts');
  const { lineEndHour } = await import('./series.ts');
  const { resolveDeclarativeEndLabels } = await import('./renderTimeChart.ts') as Record<string, unknown>;
  assert.equal(resolveDeclarativeEndLabels, undefined);

  const summary = {
    from: 0, to: 100, width: 342,
    buckets: [
      { firstHour: 0, lastHour: 20, firstValue: 0.6, lastValue: 0.6, lastValueHour: 20, minHour: 0, minValue: 0.6, maxHour: 0, maxValue: 0.6, firstBreak: false, hasBreak: false },
      { firstHour: 21, lastHour: 80, firstValue: 0.8, lastValue: 0.8, lastValueHour: 80, minHour: 21, minValue: 0.8, maxHour: 21, maxValue: 0.8, firstBreak: false, hasBreak: false },
    ],
  };
  const nameAt = (hour: number) => (_tag: string) => ({ name: hour < 50 ? 'Old Name' : 'New Name' });
  const widgetUntil = 20;
  const resolver = (frame: PlotFrame) => stabilityEndLabels(
    frame,
    widgetUntil,
    100,
    [{ tag: 'ITA', hex: '#44aa44' }, { tag: 'GER', hex: '#555555' }],
    [{ key: 'stability', label: 'Stability' }],
    (_measure, tag) => tag === 'ITA' ? summary : undefined,
    nameAt,
    3,
    END_LABEL_HEIGHT,
    4,
  );
  const frameAtLive80 = plotFrame(400, 200, { left: 44, right: 14, top: 12, bottom: 24 }, [0, 80], [0, 1], false, 0, false);
  const resolved = resolver(frameAtLive80);
  assert.ok('ends' in resolved && resolved.ends.length === 1);
  assert.equal(resolved.ends[0]!.text, 'Old Name: Stability 60.0%');

  const runOnlyFrame = plotFrame(400, 200, { left: 44, right: 14, top: 12, bottom: 24 }, [0, 100], [0, 100], false, 0, false);
  const runOnly = {
    from: 0, to: 100, width: Math.ceil(runOnlyFrame.plotWidth),
    buckets: [],
    runs: [{ firstHour: 20, lastHour: 70, value: 50, breakBefore: false }],
  };
  const end = summaryGeometry(runOnlyFrame, runOnly, false, 100).end;
  const last = lineEndHour(runOnly.runs, runOnlyFrame.x[0], 100);
  assert.ok(end && last === 70);

  const offPlotFrame = plotFrame(400, 200, { left: 44, right: 14, top: 12, bottom: 24 }, [40, 100], [0, 100], false, 0, false);
  const offPlot = {
    from: 0, to: 100, width: Math.ceil(offPlotFrame.plotWidth),
    buckets: [{ firstHour: 0, lastHour: 30, firstValue: 50, lastValue: 50, lastValueHour: 30, minHour: 0, minValue: 50, maxHour: 0, maxValue: 50, firstBreak: false, hasBreak: false }],
    runs: [{ firstHour: 0, lastHour: 30, value: 50, breakBefore: false }],
  };
  assert.equal(summaryGeometry(offPlotFrame, offPlot, false, 100).end, null);
});
