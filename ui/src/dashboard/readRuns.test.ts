import test from 'node:test';
import assert from 'node:assert/strict';
import { readRunAt } from './reads.ts';

test('compressed reads retain their supplying hour and never fill gaps', () => {
  const reads = [
    { firstHour: 10, lastHour: 33, readHour: 10, hourly: false },
    { firstHour: 35, lastHour: 100, readHour: 35, hourly: true },
  ];
  assert.equal(readRunAt(reads, 9), null);
  assert.equal(readRunAt(reads, 33), 10);
  assert.equal(readRunAt(reads, 34), null);
  assert.equal(readRunAt(reads, 35), 35);
  assert.equal(readRunAt(reads, 99), 99);
  assert.equal(readRunAt(reads, 101), null);
});

test('carried-only stability reads opt into implicit hourly dates', () => {
  const carried = [
    { firstHour: 11, lastHour: 14, readHour: 10 },
    { firstHour: 16, lastHour: 17, readHour: 15 },
    { firstHour: 18, lastHour: 18, readHour: 16 },
  ];
  assert.equal(readRunAt(carried, 9, true), 9);
  assert.equal(readRunAt(carried, 14, true), 10);
  assert.equal(readRunAt(carried, 15, true), 15);
  assert.equal(readRunAt(carried, 18, true), 16);
  assert.equal(readRunAt(carried, 19, true), 19);
  assert.equal(readRunAt([], 15), null);
  assert.equal(readRunAt([], 15, true), 15);
});
