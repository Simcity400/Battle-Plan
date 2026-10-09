import assert from 'node:assert/strict';
import test from 'node:test';
import { plotFrame } from './frame.ts';
import { pickLines } from './linePick.ts';

const frame = plotFrame(300, 200, { left: 0, right: 0, top: 0, bottom: 0 }, [0, 100], [0, 100], false);
const runs = (value: number) => [{ firstHour: 0, lastHour: 100, value }];

test('no drawn geometry cannot supply raw finite candidates', () => {
  const lines = [{ key: 'finite', runs: runs(50) }, { key: 'off plot', runs: runs(200) }];
  assert.deepEqual(pickLines(lines, 50), []);
  assert.deepEqual(pickLines(lines, 50, { until: null } as Parameters<typeof pickLines>[2]), []);
});

test('pick the nearest drawn actor and offer overlapping actors without taking the first', () => {
  const lines = [{ key: 'first', runs: runs(10) }, { key: 'clicked', runs: runs(80) }, { key: 'same', runs: runs(80) }];
  const pointer = { frame, until: null, point: { x: frame.toX(50), y: frame.toY(80) } };
  assert.deepEqual(pickLines(lines.slice(0, 2), 50, pointer), ['clicked']);
  assert.deepEqual(pickLines(lines, 50, pointer), ['clicked', 'same']);
  assert.deepEqual(pickLines(lines, 50, { frame, until: null }), ['first', 'clicked', 'same']);
});

test('unknown, gap, clipped and not yet drawn lines cannot be picked', () => {
  const lines = [{ key: 'visible', runs: runs(50) }, { key: 'gap', runs: [{ firstHour: 0, lastHour: 10, value: 50 }] },
    { key: 'unknown', runs: runs(NaN) }, { key: 'outside', runs: runs(200) }];
  assert.deepEqual(pickLines(lines, 50, { frame, until: 70 }), ['visible']);
  assert.deepEqual(pickLines(lines, 80, { frame, until: 70 }), []);
});

test('picks use the smoothed path that is drawn, preserving the displayed hour', () => {
  const curved = [{ firstHour: 0, lastHour: 49, value: 0 }, { firstHour: 50, lastHour: 100, value: 100 }];
  const lines = [{ key: 'held operand', runs: runs(0) }, { key: 'drawn curve', runs: curved }];
  assert.deepEqual(pickLines(lines, 49, { frame, until: null, point: { x: frame.toX(49), y: frame.toY(40) } }), ['drawn curve']);
});

test('held vertical edges and multiple measures of one actor remain one choice', () => {
  const lines = [{ key: 'same country', held: true, runs: [{ firstHour: 0, lastHour: 49, value: 20 }, { firstHour: 50, lastHour: 100, value: 80 }] },
    { key: 'same country', held: true, runs: runs(80) }, { key: 'other', held: true, runs: runs(30) }];
  assert.deepEqual(pickLines(lines, 50, { frame, until: null, point: { x: frame.toX(50), y: frame.toY(60) } }), ['same country']);
});
