import test from 'node:test';
import assert from 'node:assert/strict';
import { followMapPlayback, mapCommands } from './mapPlayback.ts';
import type { BoundTrack } from './model.ts';

const track = { source: { path: 'campaign.battleplan', revision: 'revision-a' }, branch: { id: '7', firstHour: 100, lastHour: 200 }, axisOffset: 100 } as BoundTrack;
const tick = () => new Promise<void>(resolve => setImmediate(resolve));
function clock() {
  let snapshot = { hour: 0, fraction: 0, hoursPerSecond: 24, playing: false, seekSequence: 0 };
  const listeners = new Set<() => void>();
  return { getSnapshot: () => snapshot, subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; }, subscribeFrame: () => () => {},
    seek: (hour: number) => { snapshot = { ...snapshot, hour, playing: false, seekSequence: snapshot.seekSequence + 1 }; for (const listener of listeners) listener(); },
    play: (hour: number, playing = true) => { snapshot = { ...snapshot, hour, playing }; for (const listener of listeners) listener(); }, update: (next: Partial<typeof snapshot>) => { snapshot = { ...snapshot, ...next }; for (const listener of listeners) listener(); }, listeners };
}

test('map playback preserves source identity, coalesces hours and stops sending after leaving the view', async () => {
  const playback = clock();
  const calls: unknown[][] = [];
  const replies: (() => void)[] = [];
  const stop = followMapPlayback(playback, track, (...args) => { calls.push(args); return new Promise(resolve => replies.push(resolve)); }, error => assert.equal(error, null));
  playback.seek(1); playback.seek(2); playback.seek(3);
  assert.deepEqual(calls, [['campaign.battleplan', 'revision-a', '7', 100, { playing: false, seekSequence: 0, fraction: 0, hoursPerSecond: 24 }]]);
  replies.shift()!(); await tick();
  assert.deepEqual(calls[1], ['campaign.battleplan', 'revision-a', '7', 103, { playing: false, seekSequence: 3, fraction: 0, hoursPerSecond: 24 }]);
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

test('a resend repeats a rejected paused seek the clock alone would not send again', async () => {
  const playback = clock();
  const calls: number[] = [];
  const statuses: unknown[] = [];
  let refuse = true;
  const failure = new Error('Cannot open recording');
  const stop = followMapPlayback(playback, track, async (_path, _revision, _branch, hour) => { calls.push(hour); if (refuse) throw failure; }, status => statuses.push(status));
  await tick();
  playback.seek(5); await tick();
  playback.update({}); await tick();
  assert.deepEqual([calls, statuses], [[100, 105], [failure, failure]]);
  refuse = false;
  stop.resend();
  assert.equal(statuses.length, 2, 'nothing is reported until the repeat is answered');
  await tick();
  assert.deepEqual([calls, statuses.at(-1)], [[100, 105, 105], null]);
  stop(); stop.resend(); await tick();
  assert.equal(calls.length, 3, 'a stopped follower sends nothing');
});

test('a failed map command stays shown until a repeat succeeds, and only its latest send reports', async () => {
  const shown = new Map<string, string>();
  const commands = mapCommands((operation, error) => { if (error == null) shown.delete(operation); else shown.set(operation, error); });
  const sent: string[] = [];
  let refuse = true;
  const command = (name: string) => async () => { sent.push(name); if (refuse) throw new Error(`Refused ${name}`); };
  commands.send('configure', command('fog')); await tick();
  assert.deepEqual(Object.fromEntries(shown), { configure: 'Refused fog' });
  commands.retry(); await tick();
  assert.deepEqual([sent, Object.fromEntries(shown)], [['fog', 'fog'], { configure: 'Refused fog' }], 'a repeat refused again keeps its failure');
  refuse = false;
  commands.retry();
  assert.deepEqual(Object.fromEntries(shown), { configure: 'Refused fog' }, 'the failure stays while its repeat is sent');
  await tick();
  assert.equal(shown.size, 0);
  commands.retry(); await tick();
  assert.equal(sent.length, 3, 'a command that succeeded is not sent again');

  refuse = true;
  let late!: () => void;
  commands.send('camera', () => new Promise<void>((_resolve, reject) => { late = () => reject(new Error('Refused walk')); }));
  commands.send('camera', command('fly')); await tick();
  late(); await tick();
  assert.deepEqual(Object.fromEntries(shown), { camera: 'Refused fly' }, 'an older reply cannot report over a newer send');
  commands.send('toggle 0', command('flip'), false);
  let due!: () => void;
  commands.send('aircraft', () => new Promise<void>((_resolve, reject) => { due = () => reject(new Error('Refused aircraft')); }));
  await tick();
  refuse = false;
  commands.forget('aircraft'); due(); await tick();
  assert.deepEqual(Object.fromEntries(shown), { camera: 'Refused fly', 'toggle 0': 'Refused flip' }, 'a forgotten send reports nothing');
  commands.retry(); await tick();
  assert.deepEqual(sent.slice(3), ['fly', 'flip', 'fly'], 'a command that cannot be repeated is cleared rather than sent again');
  assert.equal(shown.size, 0);
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


test('full map carries seek identity across coalesced playback, same-hour seeks and pauses', async () => {
  const playback = clock();
  const calls: unknown[][] = [];
  const replies: (() => void)[] = [];
  const stop = followMapPlayback(playback, track, (...args) => { calls.push(args); return new Promise(resolve => replies.push(resolve)); }, error => assert.equal(error, null));
  playback.play(20);
  playback.seek(30);
  playback.play(50);
  replies.shift()!(); await tick();
  assert.deepEqual(calls[1].slice(3), [150, { playing: true, seekSequence: 1, fraction: 0, hoursPerSecond: 24 }]);
  replies.shift()!(); await tick();
  playback.play(50, false); // Same-hour pause must reach the viewer.
  assert.deepEqual(calls[2].slice(3), [150, { playing: false, seekSequence: 1, fraction: 0, hoursPerSecond: 24 }]);
  replies.shift()!(); await tick();
  playback.seek(50); // And a same-hour seek must reset it.
  assert.deepEqual(calls[3].slice(3), [150, { playing: false, seekSequence: 2, fraction: 0, hoursPerSecond: 24 }]);
  stop(); replies.shift()!(); await tick();
});


test('fraction-only and speed-only clock updates reach both native map clocks', async () => {
  const playback = clock();
  const calls: unknown[][] = [];
  const stop = followMapPlayback(playback, track, async (...args) => { calls.push(args); }, () => {});
  await tick();
  playback.update({ playing: true, fraction: 0.25, hoursPerSecond: 0.25 }); await tick();
  playback.update({ fraction: 0.375 }); await tick();
  playback.update({ hoursPerSecond: 1440 }); await tick();
  playback.update({ playing: false, fraction: 0.75 }); await tick();
  assert.deepEqual(calls.slice(1).map(call => call.slice(3)), [
    [100, { playing: true, seekSequence: 0, fraction: 0.25, hoursPerSecond: 0.25 }],
    [100, { playing: true, seekSequence: 0, fraction: 0.375, hoursPerSecond: 0.25 }],
    [100, { playing: true, seekSequence: 0, fraction: 0.375, hoursPerSecond: 1440 }],
    [100, { playing: false, seekSequence: 0, fraction: 0.75, hoursPerSecond: 1440 }],
  ]);
  stop();
});
