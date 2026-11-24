import assert from 'node:assert/strict';
import test from 'node:test';
import { FrameEngine } from './frame.ts';
import type { EngineFrame } from './frame.ts';

function manual() {
  let asked: ((now: number) => void) | null = null;
  const engine = new FrameEngine({ request: run => { asked = run; } });
  return {
    engine,
    get pending() { return asked !== null; },
    run(now: number) { const run = asked; asked = null; assert.ok(run, 'a frame was asked for'); run(now); },
  };
}

test('every frame runs its phases in order, whatever order the work was asked in', () => {
  const frames = manual();
  const seen: string[] = [];
  frames.engine.next('commit', () => seen.push('commit'));
  frames.engine.next('paint', () => seen.push('paint'));
  frames.engine.next('after', () => seen.push('after'));
  frames.engine.next('data', () => seen.push('data'));
  frames.engine.next('measure', () => seen.push('measure'));
  frames.engine.next('clock', () => seen.push('clock'));
  frames.run(16);
  assert.deepEqual(seen, ['clock', 'data', 'measure', 'paint', 'commit', 'after']);
  assert.equal(frames.pending, false, 'nothing left: no frame asked for');
});

test('work a phase adds for a later phase runs in the same frame; for its own or an earlier phase, in the next', () => {
  const frames = manual();
  const seen: string[] = [];
  frames.engine.next('data', frame => {
    seen.push(`data ${frame.index}`);
    frames.engine.next('paint', painted => seen.push(`paint ${painted.index}`));
    frames.engine.next('data', again => seen.push(`data again ${again.index}`));
    frames.engine.next('clock', clock => seen.push(`clock ${clock.index}`));
  });
  frames.run(16);
  assert.deepEqual(seen, ['data 1', 'paint 1']);
  assert.ok(frames.pending, 'the next frame is asked for once the frame ends');
  frames.run(32);
  assert.deepEqual(seen, ['data 1', 'paint 1', 'clock 2', 'data again 2']);
});

test('a batch lands everything pushed between frames once, in order, before the frame paints', () => {
  const frames = manual();
  const landed: (readonly number[])[] = [];
  const order: string[] = [];
  const push = frames.engine.batch<number>(items => { landed.push(items); order.push('land'); });
  frames.engine.next('paint', () => order.push('paint'));
  push(1); push(2); push(3);
  frames.run(16);
  assert.deepEqual(landed, [[1, 2, 3]], 'three messages, one landing');
  assert.deepEqual(order, ['land', 'paint']);
  push(4);
  frames.run(32);
  assert.deepEqual(landed, [[1, 2, 3], [4]]);
  assert.equal(frames.pending, false);
});

test('a task runs every frame while it asks to, sleeps when it does not, and runs again once woken', () => {
  const frames = manual();
  const ran: number[] = [];
  let more = 2;
  const task = frames.engine.every('paint', frame => { ran.push(frame.index); return --more > 0; });
  frames.run(16);
  frames.run(32);
  assert.deepEqual(ran, [1, 2]);
  assert.equal(frames.pending, false, 'asleep: no frame asked for');
  more = 1;
  task.wake();
  task.wake();
  frames.run(48);
  assert.deepEqual(ran, [1, 2, 3], 'woken twice, run once');
  task.stop();
  task.wake();
  assert.equal(frames.pending, false, 'a stopped task asks for no frame');
});

test('a failing task does not stop the frame, and each frame reports its time step, capped', () => {
  const frames = manual();
  const steps: EngineFrame[] = [];
  const errors = console.error;
  console.error = () => {};
  try {
    frames.engine.next('paint', () => { throw new Error('broken painter'); });
    frames.engine.every('paint', frame => { steps.push(frame); return steps.length < 3; });
    frames.run(100);
    frames.run(116);
    frames.run(1116);
  } finally { console.error = errors; }
  assert.deepEqual(steps.map(frame => frame.dt), [16, 16, 64], 'the first frame steps one frame; a long gap at most 64 ms');
  assert.ok(steps.every(frame => frame.phase === 'paint'));
});


