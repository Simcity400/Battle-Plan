import assert from 'node:assert/strict';
import test from 'node:test';
import { busy, busyCount, busyKinds, busyPreparing, inBackground, onForegroundRequest } from './busy.ts';
import { inFrame } from '../dashboard/frameUpdates.ts';

test('each thing in flight counts once until it ends, by its kind', () => {
  const before = busyCount();
  const asked = busy('requests'), queued = busy('updates');
  assert.equal(busyCount(), before + 2);
  assert.equal(busyKinds().requests, 1);
  asked();
  asked();
  assert.equal(busyCount(), before + 1, 'ending twice ends once');
  queued();
  assert.equal(busyCount(), before);
  assert.deepEqual(Object.values(busyKinds()), [0, 0, 0, 0, 0, 0]);
});

test('a request the prepare lane starts is its own, never in the count the page settles by', () => {
  const foreground: number[] = [];
  const stop = onForegroundRequest(() => foreground.push(busyKinds().requests));
  const before = busyCount();
  const prepared = inBackground(() => busy('requests'));
  const update = inBackground(() => busy('updates'));
  const asked = busy('requests');
  stop();
  assert.deepEqual(foreground, [1], 'foreground listeners run synchronously, only for the request that takes priority');
  assert.equal(busyPreparing(), 1);
  assert.equal(busyKinds().requests, 1, 'only the request asked outside the lane');
  assert.equal(busyCount(), before + 2, 'an update is waited on wherever it starts');
  prepared(); prepared();
  assert.equal(busyPreparing(), 0, 'ending twice ends once');
  update(); asked();
  assert.equal(busyCount(), before);
});

test('data updates are in flight from when they are queued until their frame renders them', () => {
  const frames: (() => void)[] = [];
  const page = globalThis as { requestAnimationFrame?: unknown; cancelAnimationFrame?: unknown };
  page.requestAnimationFrame = (run: () => void) => frames.push(run);
  page.cancelAnimationFrame = () => {};
  try {
    const ran: string[] = [];
    inFrame(() => ran.push('reply'));
    inFrame(() => ran.push('name'));
    assert.equal(busyKinds().updates, 1, 'one frame holds them all');
    assert.equal(frames.length, 1);
    frames.shift()!();
    assert.deepEqual(ran, ['reply', 'name']);
    assert.equal(busyKinds().updates, 0);
    assert.equal(busyKinds().commits, 0, 'the batch finished rendering');
    assert.equal(busyCount(), 0);
  } finally {
    delete page.requestAnimationFrame;
    delete page.cancelAnimationFrame;
  }
});
