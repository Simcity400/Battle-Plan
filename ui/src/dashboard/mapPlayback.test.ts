import test from 'node:test';
import assert from 'node:assert/strict';
import { followMapPlayback } from './mapPlayback.ts';
import type { BoundTrack } from './model.ts';

const track = { source: { path: 'campaign.battleplan', revision: 'revision-a' }, branch: { id: '7', firstHour: 100, lastHour: 200 }, axisOffset: 100 } as BoundTrack;
const tick = () => new Promise<void>(resolve => setImmediate(resolve));
function clock() {
  let hour = 0;
  const listeners = new Set<() => void>();
  return { getHour: () => hour, subscribeHour: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    seek: (next: number) => { hour = next; for (const listener of listeners) listener(); }, listeners };
}

test('map playback preserves source identity, coalesces hours and stops sending after leaving the view', async () => {
  const playback = clock();
  const calls: unknown[][] = [];
  const replies: (() => void)[] = [];
  const stop = followMapPlayback(playback, track, (...args) => { calls.push(args); return new Promise(resolve => replies.push(resolve)); }, error => assert.equal(error, null));
  playback.seek(1); playback.seek(2); playback.seek(3);
  assert.deepEqual(calls, [['campaign.battleplan', 'revision-a', '7', 100]]);
  replies.shift()!(); await tick();
  assert.deepEqual(calls[1], ['campaign.battleplan', 'revision-a', '7', 103]);
  playback.seek(4); stop(); replies.shift()!(); await tick();
  assert.equal(calls.length, 2);
  assert.equal(playback.listeners.size, 0);
});

test('map playback reports failures and out-of-range dates without clamping, then recovers on a valid seek', async () => {
  const playback = clock();
  const calls: number[] = [];
  const statuses: unknown[] = [];
  const failure = new Error('Recording changed');
  const stop = followMapPlayback(playback, track, async (_path, _revision, _branch, hour) => { calls.push(hour); if (hour === 100) throw failure; }, status => statuses.push(status));
  await tick(); assert.equal(statuses.at(-1), failure);
  playback.seek(101); await tick();
  assert.match(String(statuses.at(-1)), /outside the selected campaign branch/);
  assert.deepEqual(calls, [100]);
  playback.seek(2); await tick();
  assert.equal(statuses.at(-1), null);
  assert.deepEqual(calls, [100, 102]);
  stop();
});

test('late native failure cannot surface in a different view', async () => {
  const playback = clock();
  let reject!: (error: Error) => void;
  const statuses: unknown[] = [];
  const stop = followMapPlayback(playback, track, () => new Promise((_resolve, fail) => { reject = fail; }), status => statuses.push(status));
  stop(); reject(new Error('Old view failed')); await tick();
  assert.deepEqual(statuses, []);
});

test('an obsolete native failure does not discard the newest requested hour', async () => {
  const playback = clock();
  let reject!: (error: Error) => void;
  const calls: number[] = [];
  const statuses: unknown[] = [];
  const stop = followMapPlayback(playback, track, (_path, _revision, _branch, hour) => {
    calls.push(hour);
    return hour === 100 ? new Promise((_resolve, fail) => { reject = fail; }) : Promise.resolve();
  }, status => statuses.push(status));
  playback.seek(3); reject(new Error('Obsolete hour failed')); await tick();
  assert.deepEqual(calls, [100, 103]);
  assert.deepEqual(statuses, [null]);
  stop();
});
