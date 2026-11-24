import assert from 'node:assert/strict';
import test from 'node:test';
import { AXIS_HEIGHT, laneAt, laneBandLayouts, laneBands, laneRows, laneStops, lanesInView, spanAt } from './layout.ts';
import { backingSize, toLocal } from '../surface.ts';
import { coversViewport, laneWindow, placeLaneWindow, sizeLaneWindow, timelineHeight, timelineViewStyle, timelineViewport, timelineY, visibleTimeline } from './viewport.ts';

test('compact expanded rows and tall overlap lanes share the virtual canvas coordinates', () => {
  const lanes = [
    { id: 'war', label: '', spans: [], rowHeight: 30 },
    { id: 'side', label: '', spans: [], rowHeight: 18 },
    { id: 'country', label: '', spans: [], rowHeight: 22, subrows: 500 },
    { id: 'ordinary', label: '', spans: [] },
  ];
  const rows = laneRows(lanes);
  assert.equal(timelineHeight(lanes.length, 40, rows), AXIS_HEIGHT + 30 + 18 + 11_000 + 40 + 8);
  for (const top of [0, 5_000, 10_900]) {
    const painted = laneWindow(lanes.length, top, 360, 40, rows);
    assert.equal(coversViewport(painted, lanes.length, { top, height: 360 }, 40, rows), true);
    assert.equal(laneAt(timelineY(10, painted), lanes.length, rows), laneAt(painted.top + 10, lanes.length, rows));
    const expected = lanesInView(Math.max(0, painted.top - AXIS_HEIGHT), painted.height + AXIS_HEIGHT - Math.max(0, AXIS_HEIGHT - painted.top), lanes.length, rows);
    assert.deepEqual([painted.first, painted.end - 1], expected);
  }
  assert.equal(laneWindow(lanes.length, 5_000, 360, 40, rows).first, 2);
});

test('both axis modes size the canvas backing from the laid-out window, never its 300x150 default', () => {
  for (const sticky of [false, true]) for (const row of [40, 54]) for (const dpr of [1, 1.25, 2]) {
    const full = timelineHeight(1000, row), parentWidth = 920, height = 600, zoom = 1.5;
    const view = timelineViewStyle(full, height, sticky);
    assert.equal(view.width, '100%', 'the absolute view must fill its parent without an in-flow child');
    assert.equal(view.height, sticky ? height : full);
    const style = { top: '', height: '' } as CSSStyleDeclaration;
    const canvas = {
      style, width: 300, height: 150,
      get clientWidth() { return view.width === '100%' ? parentWidth : sticky ? parentWidth : 0; },
      get clientHeight() { return parseFloat(style.height) || 150; },
      getBoundingClientRect() { return { width: this.clientWidth * zoom, height: this.clientHeight * zoom } as DOMRect; },
    };
    for (const top of [0, 20000, full - height]) {
      const painted = laneWindow(1000, top, height, row);
      const size = sizeLaneWindow(canvas, painted, sticky ? top : 0, dpr);
      assert.ok(size, 'layout gives drawing a nonzero surface');
      assert.equal(size.layoutHeight, painted.height, 'measure after applying the window height');
      assert.equal(canvas.width, Math.round(parentWidth * zoom * dpr));
      assert.equal(canvas.height, Math.round(painted.height * zoom * dpr));
      assert.notDeepEqual([canvas.width, canvas.height], [300, 150]);
      assert.equal(size.scaleY, zoom * dpr);
    }
    const writes: PropertyKey[] = [];
    const same = new Proxy(canvas, { set(target, key, value) {
      writes.push(key); return Reflect.set(target, key, value);
    } });
    sizeLaneWindow(same, laneWindow(1000, full - height, height, row), sticky ? full - height : 0, dpr);
    assert.deepEqual(writes, [], 'an unchanged layout does not reset the backing store');
  }
});

test('1,000 lifetimes retain sharp 40px lanes at every scroll position', () => {
  assert.equal(timelineHeight(1000), 40032);
  for (const top of [0, 17, 20000, 39432]) {
    const window = laneWindow(1000, top, 600);
    assert.ok(window.height <= 1800, `${top}: a viewport, not a 40,032px bitmap`);
    assert.ok(window.end - window.first <= 46, `${top}: only visible and overscan lanes`);
    const size = backingSize({ layoutWidth: 800, layoutHeight: window.height, rectWidth: 1200, rectHeight: window.height * 1.5, devicePixelRatio: 2 });
    assert.equal(size.scaleY, 3, 'Size zoom and DPI keep 120 raster pixels per lane');
    const lane = laneAt(top + 300, 1000)!;
    assert.ok(lane >= window.first && lane < window.end);
    const local = toLocal(600, (top + 300 - window.top) * 1.5, { left: 0, top: 0, width: 1200, height: window.height * 1.5 }, 800, window.height);
    assert.equal(laneAt(timelineY(local.y, window), 1000), lane, 'hover and right-click reach the drawn lane under zoom');
  }
});

test('scrolling lays out only the lanes in view and keeps keyboard stops for every lane', () => {
  const lanes = Array.from({ length: 1000 }, (_, index) => ({ id: `${index}`, label: `${index}`, spans: [
    { id: `span-${index}`, label: `${index}`, firstHour: 100, lastHour: 147, stops: [124], band: [{ firstHour: 100, lastHour: 147, value: 5 }] },
  ] }));
  const window = laneWindow(lanes.length, 20000, 600);
  const shown = lanes.slice(window.first, window.end);
  let computed = 0;
  const layouts = laneBandLayouts(shown, 'view', new Map(), lane => {
    computed++;
    return laneBands(lane.spans, null, h => h, v => v, 0, 200);
  });
  assert.equal(computed, shown.length);
  assert.ok(computed < 47);
  assert.equal(spanAt(lanes[500].spans, 124)?.id, 'span-500');
  assert.deepEqual(laneStops(lanes[999].spans, 100, 200, null).map(stop => stop.hour), [100, 124]);
  const end = laneWindow(1000, 39432, 600);
  assert.equal(end.end, 1000);
  const next = laneBandLayouts(lanes.slice(end.first, end.end), 'view', layouts, lane => laneBands(lane.spans, null, h => h, v => v, 0, 200));
  assert.ok(next.size <= 46);
  assert.ok(!next.has('500'), 'off-screen band layouts are released');
});

test('small timelines keep their original height and the date axis is not pinned', () => {
  assert.deepEqual(laneWindow(2, 0, 600), { top: 0, height: 112, first: 0, end: 2 });
  assert.equal(laneAt(timelineY(0, laneWindow(1000, 20000, 600)), 1000), 484);
  assert.equal(laneAt(timelineY(10, laneWindow(2, 0, 600)), 2), null);
  const offscreen = laneWindow(1000, 50000, 0);
  assert.equal(offscreen.top + offscreen.height, timelineHeight(1000), 'an off-screen canvas adds no scroll overflow');
});

test('the canvas window follows nested clipping containers, their borders and Size zoom', () => {
  const beforeWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const beforeStyle = Object.getOwnPropertyDescriptor(globalThis, 'getComputedStyle');
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { innerHeight: 900 } });
  Object.defineProperty(globalThis, 'getComputedStyle', { configurable: true, value: (element: { overflowY: string }) => element });
  const outer = { overflowY: 'hidden', offsetHeight: 504, clientHeight: 500, clientTop: 2,
    getBoundingClientRect: () => ({ top: 90, height: 756 }), parentElement: null };
  const inner = { overflowY: 'auto', offsetHeight: 404, clientHeight: 400, clientTop: 2,
    getBoundingClientRect: () => ({ top: 120, height: 606 }), parentElement: outer };
  const spacer = { clientHeight: 40032, getBoundingClientRect: () => ({ top: -29880, bottom: 30168, height: 60048 }), parentElement: inner };
  try {
    assert.deepEqual(visibleTimeline(spacer as unknown as HTMLElement), { top: 20002, height: 400 });
    const window = laneWindow(1000, 20002, 400);
    assert.ok(window.end - window.first <= 31);
    assert.equal(laneAt(timelineY(300, window), 1000), laneAt(window.top + 300, 1000));
  } finally {
    if (beforeWindow) Object.defineProperty(globalThis, 'window', beforeWindow);
    else Reflect.deleteProperty(globalThis, 'window');
    if (beforeStyle) Object.defineProperty(globalThis, 'getComputedStyle', beforeStyle);
    else Reflect.deleteProperty(globalThis, 'getComputedStyle');
  }
});

test('page scrolling stays painted and arbitrary jumps require an immediate repaint', () => {
  const count = 1000, height = 600;
  let painted = laneWindow(count, 20000, height);
  assert.ok(coversViewport(painted, count, { top: 20600, height }), 'a full page down is already painted');
  assert.ok(coversViewport(painted, count, { top: 19400, height }), 'a full page up is already painted');
  for (const top of [36000, 0, 39432, 10000]) {
    const visible = { top, height };
    assert.ok(!coversViewport(painted, count, visible), 'a scrollbar jump repaints in its scroll event');
    painted = laneWindow(count, top, height);
    assert.ok(coversViewport(painted, count, visible));
    assert.ok(painted.height <= 3 * height, 'fast scrolling keeps the bitmap bounded');
  }
});


test('sticky and scrolling axes share overscan and full-coordinate hits under Size zoom', () => {
  const beforeWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const beforeStyle = Object.getOwnPropertyDescriptor(globalThis, 'getComputedStyle');
  Object.defineProperty(globalThis, 'window', { configurable: true, value: { innerHeight: 1200 } });
  Object.defineProperty(globalThis, 'getComputedStyle', { configurable: true, value: (element: { overflowY: string }) => element });
  const count = 5000, row = 80, scrolled = 20000, height = 600, scale = 1.5;
  const parent = { overflowY: 'auto', offsetHeight: 604, clientHeight: height, clientTop: 2,
    getBoundingClientRect: () => ({ top: 90, height: 906 }), parentElement: null };
  const top = 93 - scrolled * scale;
  const full = timelineHeight(count, row);
  const content = { clientHeight: full, getBoundingClientRect: () => ({ top, bottom: top + full * scale, height: full * scale }), parentElement: parent };
  const view = { clientHeight: height, getBoundingClientRect: () => ({ top: 93, bottom: 993, height: height * scale }), parentElement: parent };
  try {
    for (const sticky of [true, false]) {
      const state = timelineViewport(content as unknown as HTMLElement, view as unknown as HTMLElement, count, row, sticky);
      assert.deepEqual(state.visible, { top: scrolled, height });
      assert.equal(state.offset, sticky ? scrolled : 0);
      const painted = laneWindow(count, state.visible.top, state.visible.height, row);
      assert.ok(painted.height <= 3 * height, 'taller lanes still cost a viewport with overscan');
      assert.ok(coversViewport(painted, count, state.visible, row));
      const style = { top: '', height: '' };
      placeLaneWindow(style, painted, state.offset);
      assert.equal(style.top, `${painted.top - state.offset}px`);
      const localY = scrolled + 300 - painted.top;
      assert.equal(laneAt(timelineY(localY, painted), count, row), 253, 'hover and Jump to resolve the full lane');
      assert.deepEqual(lanesInView(scrolled, height, count, row), [250, 257], 'reported lanes exclude overscan');
    }
  } finally {
    if (beforeWindow) Object.defineProperty(globalThis, 'window', beforeWindow);
    else Reflect.deleteProperty(globalThis, 'window');
    if (beforeStyle) Object.defineProperty(globalThis, 'getComputedStyle', beforeStyle);
    else Reflect.deleteProperty(globalThis, 'getComputedStyle');
  }
});

test('taller lanes retain overscan at the last row and across scrollbar jumps', () => {
  const count = 5000, row = 80, height = 600;
  const full = timelineHeight(count, row);
  let painted = laneWindow(count, 20000, height, row);
  for (const top of [360000, 0, full - height]) {
    assert.ok(!coversViewport(painted, count, { top, height }, row));
    painted = laneWindow(count, top, height, row);
    assert.ok(coversViewport(painted, count, { top, height }, row));
    assert.ok(painted.height <= 3 * height);
  }
  assert.equal(painted.end, count);
  assert.equal(painted.top + painted.height, full);
});
