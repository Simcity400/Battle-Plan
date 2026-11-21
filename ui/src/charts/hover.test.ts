import assert from 'node:assert/strict';
import test from 'node:test';
import { keyedHour, keyStops } from './frame.ts';
import { actionHour, hoverAt, NO_HOVER, pointerMenu, pointerTarget, reduceHover } from './hover.ts';
import type { HoverState } from './hover';

type Point = { x: number };
const pointed = (point: Point) => ({ hour: point.x });
const X: [number, number] = [100, 200];

test('a navigation key takes the hover from the pointer, and the pointer takes it back', () => {
  let state: HoverState<Point> = reduceHover(NO_HOVER, { type: 'pointer', point: { x: 150 } });
  assert.deepEqual(hoverAt(state, X, null, pointed), { hour: 150, by: 'pointer' });
  assert.equal(actionHour(150, state, 120), 150);
  const end = keyedHour('End', state.keyed ?? 150, X, null, [100, 124, 148, 172, 196]);
  state = reduceHover(state, { type: 'key', hour: end });
  assert.equal(state.pointer, null);
  assert.deepEqual(hoverAt(state, X, null, pointed), { hour: 196, by: 'keyboard' });
  assert.equal(actionHour(196, state, 120), 196);
  assert.equal(keyedHour('ArrowLeft', 150, X, null, [100, 124, 148, 172, 196]), 148);
  state = reduceHover(state, { type: 'pointer', point: { x: 130 } });
  assert.deepEqual(hoverAt(state, X, null, pointed), { hour: 130, by: 'pointer' });
  assert.equal(state.keyed, null);
});

test('leaving or Escape ends only its own hover', () => {
  let state: HoverState<Point> = reduceHover(NO_HOVER, { type: 'key', hour: 172 });
  state = reduceHover(state, { type: 'pointer-leave' });
  assert.deepEqual(hoverAt(state, X, null, pointed), { hour: 172, by: 'keyboard' });
  state = reduceHover(state, { type: 'key-clear' });
  assert.equal(hoverAt(state, X, null, pointed), null);
  assert.equal(actionHour(null, state, 120), 120);
  const later = reduceHover(NO_HOVER, { type: 'key', hour: 190 });
  assert.equal(hoverAt(later, X, 180, pointed), null);
  assert.equal(actionHour(null, later, 120), 190);
  assert.equal(hoverAt(reduceHover(NO_HOVER, { type: 'pointer', point: { x: 5 } }), X, null, () => null), null);
});

test('a range between two daily reads still has a stop at the value carried into it', () => {
  const reads = [100, 124];
  assert.deepEqual(keyStops(reads, 110, 120), [110]);
  for (const key of ['Home', 'End', 'ArrowLeft', 'ArrowRight'] as const) {
    assert.equal(keyedHour(key, null, [110, 120], null, reads), 110, key);
    assert.equal(keyedHour(key, 110, [110, 120], null, reads), 110, key);
  }
  assert.deepEqual(keyStops(reads, 110, 130), [110, 124]);
  assert.equal(keyedHour('ArrowRight', 110, [110, 130], null, reads), 124);
  assert.deepEqual(keyStops(reads, 100, 130), [100, 124]);
  assert.deepEqual(keyStops(reads, 50, 99), []);
  assert.deepEqual(keyStops(reads, 110.4, 120), [110.4]);
  assert.deepEqual(keyStops(reads, 110.4, 110.9), [110.4]);
  assert.deepEqual(keyStops([0, 24], 23.2, 47), [23.2, 24]);
  assert.equal(keyedHour('Home', null, [23.2, 47], null, [0, 24]), 23.2);
  assert.equal(keyedHour('ArrowLeft', 24, [23.2, 47], null, [0, 24]), 23.2);
  assert.equal(keyedHour('End', null, [110, 120], 108, reads), null);
});

test('the browser’s menu event after the menu key opens no second menu', () => {
  let state: HoverState<Point> = reduceHover(NO_HOVER, { type: 'pointer', point: { x: 150 } });
  state = reduceHover(state, { type: 'key', hour: 196 });
  state = reduceHover(state, { type: 'menu-key' });
  assert.equal(pointerMenu(state), false);
  state = reduceHover(state, { type: 'contextmenu' });
  state = reduceHover(state, { type: 'menu-key' });
  state = reduceHover(state, { type: 'pointer-down' });
  assert.equal(pointerMenu(state), true);
  assert.equal(pointerMenu(reduceHover(NO_HOVER, { type: 'contextmenu' })), true);
});

test('a strip hover is about the bar under it; the plot keeps its own hover and snaps only in view', () => {
  const describe = (hour: number) => hour > 976 && hour <= 1000 ? 1000 : hour > 1000 && hour <= 1024 ? 1024 : hour;
  const extent = [900, 2000] as const;
  assert.deepEqual(pointerTarget('plot', 1012, [900, 2000], null, extent, { describe }), { at: 1012, describes: 1012 });
  assert.deepEqual(pointerTarget('strip', 1012, [900, 2000], null, extent, { describe }), { at: 1012, describes: 1024 });
  assert.deepEqual(pointerTarget('strip', 1012, [990, 1016], null, extent, { describe }), { at: 1012, describes: 1024 });
  assert.deepEqual(pointerTarget('strip', 1012, [900, 2000], 1020, extent, { describe }), { at: 1012, describes: 1012 });
  assert.deepEqual(pointerTarget('strip', 1012, [900, 1016], null, [900, 1016], { describe }), { at: 1012, describes: 1012 });
  assert.deepEqual(pointerTarget('strip', 1500, [900, 2000], null, extent, { describe }), { at: 1500, describes: 1500 });
  assert.deepEqual(pointerTarget('strip', 1012, [900, 2000], null, extent), { at: 1012, describes: 1012 });
  assert.deepEqual(pointerTarget('strip', 1012, [900, 2000], null, extent, { snap: () => 1010 }), { at: 1012, describes: 1012 });
  assert.deepEqual(pointerTarget('plot', 1012, [900, 2000], null, extent, { snap: () => 1010 }), { at: 1010, describes: 1010 });
  assert.deepEqual(pointerTarget('plot', 1012, [1011, 2000], null, extent, { snap: () => 1010 }), { at: 1012, describes: 1012 });
  assert.deepEqual(pointerTarget('plot', 1012, [900, 2000], 1013, extent, { snap: () => 1020 }), { at: 1012, describes: 1012 });
  assert.equal(pointerTarget('plot', null, [900, 2000], null, extent), null);
});
