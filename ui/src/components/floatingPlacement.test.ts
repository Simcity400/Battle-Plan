import assert from 'node:assert/strict';
import test from 'node:test';
import { BAR, centerPanel, clampPanel } from './floatingPlacement.ts';

test('a panel opened or grown near the bottom stays whole on screen, while a drag may leave only its bar showing', () => {
  const view = { width: 1000, height: 800 };
  assert.deepEqual(clampPanel(440, 500, 300, 700, view, Infinity), { x: 300, y: 288 });
  assert.deepEqual(clampPanel(440, 500, 300, 700, view, BAR), { x: 300, y: 700 });
  assert.deepEqual(clampPanel(440, 900, 300, 400, view, Infinity), { x: 300, y: 12 }, 'a panel taller than the view keeps its bar at the top');
  assert.deepEqual(clampPanel(440, 200, 900, 5, view, Infinity), { x: 548, y: 12 });
});

test('centerPanel centers inside the viewport and clamps to margins when oversized', () => {
  assert.deepEqual(centerPanel(600, 400, { width: 1200, height: 800 }), { x: 300, y: 200 });
  assert.deepEqual(centerPanel(1400, 900, { width: 1200, height: 800 }), { x: 12, y: 12 });
});
