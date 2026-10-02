import assert from 'node:assert/strict';
import test from 'node:test';
import { AXIS_HEIGHT, ROW_HEIGHT, laneAt, laneOffset, lanesInView } from './layout.ts';

test('double-height lanes use the same rows for drawing, hover and Jump to after scrolling', () => {
  const row = ROW_HEIGHT * 2;
  const height = AXIS_HEIGHT + row * 3;
  const offset = row * 17 + row / 2;
  assert.deepEqual(lanesInView(0, height, 5000, row), [0, 2]);
  assert.deepEqual(lanesInView(offset, height, 5000, row), [17, 20]);
  assert.equal(laneAt(AXIS_HEIGHT + offset, 5000, row), 17);
  assert.equal(laneAt(AXIS_HEIGHT + row / 2 + offset, 5000, row), 18);
  assert.equal(laneAt(AXIS_HEIGHT + row * 3 - 0.01 + offset, 5000, row), 20);
  assert.equal(laneAt(AXIS_HEIGHT - 0.01, 5000, row), null);
  assert.equal(laneAt(AXIS_HEIGHT + row * 5000, 5000, row), null);
});

test('double-height spacer clamps scrolling at the last lane and keeps it in view', () => {
  const row = ROW_HEIGHT * 2;
  const count = 5000;
  const full = AXIS_HEIGHT + count * row + 8;
  const height = AXIS_HEIGHT + row * 3;
  const offset = laneOffset(full, 0, height, full);
  assert.equal(offset, row * (count - 3) + 8);
  assert.deepEqual(lanesInView(offset, height, count, row), [4997, 4999]);
  assert.equal(laneAt(AXIS_HEIGHT + row * 2 + offset, count, row), 4999);
  assert.deepEqual(lanesInView(0, AXIS_HEIGHT, count, row), [0, -1]);
});
