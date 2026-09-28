import assert from 'node:assert/strict';
import test from 'node:test';
import { keyedHour, keyStops } from './frame.ts';
import { actionHour, hoverAt, NO_HOVER, pointerMenu, reduceHover } from './hover.ts';
import type { HoverState } from './hover';

type Point = { x: number };
/** The pointer's hour: one hour per pixel, from 0. */
const pointed = (point: Point) => ({ hour: point.x });
const X: [number, number] = [100, 200];

test('a navigation key takes the hover from the pointer, and the pointer takes it back', () => {
  // The review's case: the pointer over hour 150, then End with the last stop at 196.
  let state: HoverState<Point> = reduceHover(NO_HOVER, { type: 'pointer', point: { x: 150 } });
  assert.deepEqual(hoverAt(state, X, null, pointed), { hour: 150, by: 'pointer' });
  assert.equal(actionHour(150, state, 120), 150);
  const end = keyedHour('End', state.keyed ?? 150, X, null, [100, 124, 148, 172, 196]);
  state = reduceHover(state, { type: 'key', hour: end });
  // Drawn, told, read out and acted on: all hour 196.
  assert.equal(state.pointer, null);
  assert.deepEqual(hoverAt(state, X, null, pointed), { hour: 196, by: 'keyboard' });
  assert.equal(actionHour(196, state, 120), 196);
  // The first arrow steps on from the pointer's hour.
  assert.equal(keyedHour('ArrowLeft', 150, X, null, [100, 124, 148, 172, 196]), 148);
  // Moving the pointer takes over again; the keyboard's hour is dropped.
  state = reduceHover(state, { type: 'pointer', point: { x: 130 } });
  assert.deepEqual(hoverAt(state, X, null, pointed), { hour: 130, by: 'pointer' });
  assert.equal(state.keyed, null);
});

test('leaving or Escape ends only its own hover', () => {
  let state: HoverState<Point> = reduceHover(NO_HOVER, { type: 'key', hour: 172 });
  // The pointer leaving does not end the keyboard's hover.
  state = reduceHover(state, { type: 'pointer-leave' });
  assert.deepEqual(hoverAt(state, X, null, pointed), { hour: 172, by: 'keyboard' });
  // Escape does.
  state = reduceHover(state, { type: 'key-clear' });
  assert.equal(hoverAt(state, X, null, pointed), null);
  assert.equal(actionHour(null, state, 120), 120);
  // A keyboard hour out of view or after the playhead shows nothing, but the keys still act on it.
  const later = reduceHover(NO_HOVER, { type: 'key', hour: 190 });
  assert.equal(hoverAt(later, X, 180, pointed), null);
  assert.equal(actionHour(null, later, 120), 190);
  // A pointer over nothing (off the plot) shows nothing.
  assert.equal(hoverAt(reduceHover(NO_HOVER, { type: 'pointer', point: { x: 5 } }), X, null, () => null), null);
});

test('a range between two daily reads still has a stop at the value carried into it', () => {
  // The review's case: reads at 100 and 124, the pools carried through 123, a range of 110 to 120.
  const reads = [100, 124];
  assert.deepEqual(keyStops(reads, 110, 120), [110]);
  for (const key of ['Home', 'End', 'ArrowLeft', 'ArrowRight'] as const) {
    assert.equal(keyedHour(key, null, [110, 120], null, reads), 110, key);
    assert.equal(keyedHour(key, 110, [110, 120], null, reads), 110, key);
  }
  // Within a range reaching the next read, both stops.
  assert.deepEqual(keyStops(reads, 110, 130), [110, 124]);
  assert.equal(keyedHour('ArrowRight', 110, [110, 130], null, reads), 124);
  // A view starting at a read, or before the first, adds nothing.
  assert.deepEqual(keyStops(reads, 100, 130), [100, 124]);
  assert.deepEqual(keyStops(reads, 50, 99), []);
  // A zoomed view starts at a fractional hour: the stop is the view's start, which hover
  // shows as its hour (110), as the pointer at the edge does, even within one hour.
  assert.deepEqual(keyStops(reads, 110.4, 120), [110.4]);
  assert.deepEqual(keyStops(reads, 110.4, 110.9), [110.4]);
  // The review's case: values change at 24 and the view starts at 23.2; hour 23's value
  // (carried from the stop at 0) stays reachable before the stop at 24.
  assert.deepEqual(keyStops([0, 24], 23.2, 47), [23.2, 24]);
  assert.equal(keyedHour('Home', null, [23.2, 47], null, [0, 24]), 23.2);
  assert.equal(keyedHour('ArrowLeft', 24, [23.2, 47], null, [0, 24]), 23.2);
  // Nothing after the playhead.
  assert.equal(keyedHour('End', null, [110, 120], 108, reads), null);
});

test('the browser’s menu event after the menu key opens no second menu', () => {
  // The review's case, as the browser plays it: End, then the menu key opens the menu at
  // 196, then the browser's own contextmenu event arrives where the pointer rests (150).
  let state: HoverState<Point> = reduceHover(NO_HOVER, { type: 'pointer', point: { x: 150 } });
  state = reduceHover(state, { type: 'key', hour: 196 });
  state = reduceHover(state, { type: 'menu-key' });
  assert.equal(pointerMenu(state), false);
  state = reduceHover(state, { type: 'contextmenu' });
  // A right-click after that is the pointer's again: its button goes down first.
  state = reduceHover(state, { type: 'menu-key' });
  state = reduceHover(state, { type: 'pointer-down' });
  assert.equal(pointerMenu(state), true);
  assert.equal(pointerMenu(reduceHover(NO_HOVER, { type: 'contextmenu' })), true);
});
