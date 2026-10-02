import assert from 'node:assert/strict';
import test from 'node:test';
import { barEndLabel, divergingBar } from './divergingBar.ts';

test('bars grow from a shared zero on one scale, and are cut at the track’s edge', () => {
  // A symmetric scale: zero in the middle, a full shortage reaching the left edge.
  assert.deepEqual(divergingBar(-1, 1, 1), { zero: 0.5, start: 0, width: 0.5 });
  assert.deepEqual(divergingBar(0.5, 1, 1), { zero: 0.5, start: 0.5, width: 0.25 });
  // Only positives: zero at the left edge.
  assert.deepEqual(divergingBar(2, 0, 4), { zero: 0, start: 0, width: 0.5 });
  // Beyond the extent, the bar stops at the edge.
  assert.deepEqual(divergingBar(-3, 1, 1), { zero: 0.5, start: 0, width: 0.5 });
  // No scale or no value: nothing drawn.
  assert.deepEqual(divergingBar(1, 0, 0), { zero: 0, start: 0, width: 0 });
  assert.deepEqual(divergingBar(Number.NaN, 1, 1), { zero: 0.5, start: 0.5, width: 0 });
});

test('a bar’s label sits past its outer end on its side', () => {
  assert.deepEqual(barEndLabel({ start: 0.2, width: 0.3 }, -5), { at: 0.2, side: 'left' });
  assert.deepEqual(barEndLabel({ start: 0.5, width: 0.3 }, 5), { at: 0.8, side: 'right' });
});
