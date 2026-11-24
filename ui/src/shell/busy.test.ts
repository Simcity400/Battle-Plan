import assert from 'node:assert/strict';
import test from 'node:test';
import { busy, busyCount, busyKinds, busyPreparing, inBackground } from './busy.ts';
import { FrameScheduler } from '../charts/scheduler.ts';
import { inFrame, inFrameNow } from '../dashboard/frameUpdates.ts';


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
  const before = busyCount();
  const prepared = inBackground(() => busy('requests'));
  const update = inBackground(() => busy('updates'));
  const asked = busy('requests');
  assert.equal(busyPreparing(), 1);
  assert.equal(busyKinds().requests, 1, 'only the request asked outside the lane');
  assert.equal(busyCount(), before + 2, 'an update is waited on wherever it starts');
  prepared(); prepared();
  assert.equal(busyPreparing(), 0, 'ending twice ends once');
  update(); asked();
  assert.equal(busyCount(), before);
});


test('a chart draw is in flight from its wake until it has drawn, never while hidden, and the scheduler counts once', () => {
  const frames: ((now: number) => void)[] = [];
  const scheduler = new FrameScheduler({ request: callback => frames.push(callback), clock: () => 0 });
  const canvas = { drawPending: undefined as boolean | undefined } as unknown as Element & { drawPending?: boolean };
  const first = { frame: () => false }, second = { frame: () => false };
  const remove = scheduler.add(first, canvas);
  scheduler.add(second);
  assert.equal(busyKinds().draws, 1, 'two draws asked, one count while any is');
  assert.equal(canvas.drawPending, true);
  frames.shift()!(16);
  assert.equal(busyKinds().draws, 0, 'drawn');
  assert.equal(canvas.drawPending, false);
  scheduler.wake(first);
  assert.equal(busyKinds().draws, 1);
  scheduler.setVisible(first, false);
  assert.equal(busyKinds().draws, 0, 'a hidden chart draws nothing, so waits on nothing');
  scheduler.setVisible(first, true);
  assert.equal(busyKinds().draws, 1);
  remove();
  assert.equal(busyKinds().draws, 0, 'a removed chart waits on nothing');
});

test('data updates are in flight from when they are queued until their frame renders them', () => {
  const frames: (() => void)[] = [];
  const page = globalThis as { requestAnimationFrame?: unknown; cancelAnimationFrame?: unknown };
  page.requestAnimationFrame = (run: () => void) => frames.push(run);
  page.cancelAnimationFrame = () => {};
  try {
    const ran: string[] = [];
    inFrame(() => ran.push('reply'));
    inFrameNow(() => ran.push('name'));
    assert.equal(busyKinds().updates, 1, 'one frame holds them all');
    assert.equal(frames.length, 1);
    frames.shift()!();
    assert.deepEqual(ran, ['name', 'reply']);
    assert.equal(busyKinds().updates, 0);
    assert.equal(busyKinds().commits, 0, 'the batch finished rendering');
    assert.equal(busyCount(), 0);
  } finally {
    delete page.requestAnimationFrame;
    delete page.cancelAnimationFrame;
  }
});
