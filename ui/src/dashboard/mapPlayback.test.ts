import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'vite';
import { createPageClockId, followMapPlayback, mapCommands, viewsKey } from './mapPlayback.ts';
import type { BoundTrack } from './model.ts';
import type { DashboardClient, DashboardMapState, DashboardMapView } from './client.ts';

const track = { source: { path: 'campaign.battleplan', revision: 'revision-a' }, branch: { id: '7', firstHour: 100, lastHour: 200 }, axisOffset: 100 } as BoundTrack;
const tick = () => new Promise<void>(resolve => setImmediate(resolve));
function clock(clockId = 'page-clock-1') {
  let snapshot = { hour: 0, fraction: 0, hoursPerSecond: 24, playing: false, seekSequence: 0 };
  const listeners = new Set<() => void>();
  let read = 0;
  return { getSnapshot: () => snapshot, composedAt: () => read, clockId, stamp: (at: number) => { read = at; }, subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; }, subscribeFrame: () => () => {},
    seek: (hour: number) => { snapshot = { ...snapshot, hour, playing: false, seekSequence: snapshot.seekSequence + 1 }; for (const listener of listeners) listener(); },
    play: (hour: number, playing = true) => { snapshot = { ...snapshot, hour, playing }; for (const listener of listeners) listener(); }, update: (next: Partial<typeof snapshot>) => { snapshot = { ...snapshot, ...next }; for (const listener of listeners) listener(); }, listeners };
}

test('map playback preserves source identity, coalesces hours and stops sending after leaving the view', async () => {
  const playback = clock();
  const calls: unknown[][] = [];
  const replies: (() => void)[] = [];
  const stop = followMapPlayback(playback, track, (...args) => { calls.push(args); return new Promise(resolve => replies.push(resolve)); }, error => assert.equal(error, null));
  playback.seek(1); playback.seek(2); playback.seek(3);
  assert.deepEqual(calls, [['campaign.battleplan', 'revision-a', '7', 100, { playing: false, seekSequence: 0, fraction: 0, hoursPerSecond: 24, composedAt: 0, clockId: 'page-clock-1' }]]);
  replies.shift()!(); await tick();
  assert.deepEqual(calls[1], ['campaign.battleplan', 'revision-a', '7', 103, { playing: false, seekSequence: 3, fraction: 0, hoursPerSecond: 24, composedAt: 0, clockId: 'page-clock-1' }]);
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
  assert.match(String(statuses.at(-1)), /outside the selected history/);
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
  assert.deepEqual(calls[1].slice(3), [150, { playing: true, seekSequence: 1, fraction: 0, hoursPerSecond: 24, composedAt: 0, clockId: 'page-clock-1' }]);
  replies.shift()!(); await tick();
  playback.play(50, false);
  assert.deepEqual(calls[2].slice(3), [150, { playing: false, seekSequence: 1, fraction: 0, hoursPerSecond: 24, composedAt: 0, clockId: 'page-clock-1' }]);
  replies.shift()!(); await tick();
  playback.seek(50);
  assert.deepEqual(calls[3].slice(3), [150, { playing: false, seekSequence: 2, fraction: 0, hoursPerSecond: 24, composedAt: 0, clockId: 'page-clock-1' }]);
  stop(); replies.shift()!(); await tick();
});


test('each anchor carries when the page composed it, and a new reading alone sends nothing', async () => {
  const playback = clock();
  const calls: unknown[][] = [];
  const stop = followMapPlayback(playback, track, async (...args) => { calls.push(args); }, () => {});
  await tick();
  playback.stamp(1_234.5);
  playback.seek(5); await tick();
  assert.deepEqual(calls.at(-1)!.slice(3), [105, { playing: false, seekSequence: 1, fraction: 0, hoursPerSecond: 24, composedAt: 1_234.5, clockId: 'page-clock-1' }]);
  playback.stamp(9_999);
  playback.update({}); await tick();
  assert.equal(calls.length, 2);
  stop();
});

test('a dashboard target read at a new time is not a new target, but a new hour or clock origin is', () => {
  const view = (hour: number, composedAt: number, clockId = 'page-clock-1') => ({ id: 'map', hour, playing: true, composedAt, clockId });
  assert.equal(viewsKey('page-clock-1', [view(100, 1_000)]), viewsKey('page-clock-1', [view(100, 9_999)]));
  assert.notEqual(viewsKey('page-clock-1', [view(100, 1_000, 'page-clock-1')]), viewsKey('page-clock-2', [view(100, 1_000, 'page-clock-2')]));
  assert.notEqual(viewsKey('page-clock-1', []), viewsKey('page-clock-2', []));
  assert.notEqual(viewsKey('page-clock-1', [view(100, 1_000)]), viewsKey('page-clock-1', [view(101, 1_000)]));
  assert.notEqual(viewsKey('page-clock-1', [view(100, 1_000)]), viewsKey('page-clock-1', []));
  const first = createPageClockId();
  const second = createPageClockId();
  assert.ok(first.length > 0 && first.length <= 128);
  assert.notEqual(first, second);
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
    [100, { playing: true, seekSequence: 0, fraction: 0.25, hoursPerSecond: 0.25, composedAt: 0, clockId: 'page-clock-1' }],
    [100, { playing: true, seekSequence: 0, fraction: 0.375, hoursPerSecond: 0.25, composedAt: 0, clockId: 'page-clock-1' }],
    [100, { playing: true, seekSequence: 0, fraction: 0.375, hoursPerSecond: 1440, composedAt: 0, clockId: 'page-clock-1' }],
    [100, { playing: false, seekSequence: 0, fraction: 0.75, hoursPerSecond: 1440, composedAt: 0, clockId: 'page-clock-1' }],
  ]);
  stop();
});

test('MapViewsStore stamps clockId on populated and teardown snapshots and filters map states by clockId before sequence', async () => {
  const saved = {
    window: globalThis.window,
    document: globalThis.document,
    ResizeObserver: globalThis.ResizeObserver,
    requestAnimationFrame: globalThis.requestAnimationFrame,
    cancelAnimationFrame: globalThis.cancelAnimationFrame,
  };
  const rafs = new Map<number, FrameRequestCallback>();
  let nextRaf = 1;
  const flushFrames = () => {
    const queued = [...rafs.entries()];
    rafs.clear();
    for (const [, callback] of queued) callback(0);
  };
  const rect = () => ({ left: 0, top: 0, right: 200, bottom: 200 });
  const element = { getBoundingClientRect: rect, setAttribute() {}, removeAttribute() {}, closest: () => null } as unknown as HTMLElement;
  class FakeResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  Object.assign(globalThis, {
    window: {
      devicePixelRatio: 1,
      matchMedia: () => ({ addEventListener() {}, removeEventListener() {} }),
      addEventListener() {},
      removeEventListener() {},
    },
    document: {
      hidden: false,
      documentElement: { clientWidth: 1000, clientHeight: 1000 },
      addEventListener() {},
      removeEventListener() {},
    },
    ResizeObserver: FakeResizeObserver,
    requestAnimationFrame: (callback: FrameRequestCallback) => {
      const id = nextRaf++;
      rafs.set(id, callback);
      return id;
    },
    cancelAnimationFrame: (id: number) => { rafs.delete(id); },
  });
  const vite = await createServer({
    configFile: false,
    optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
  });
  try {
    const { MapViewsStore } = await vite.ssrLoadModule('/ui/src/dashboard/geometry.ts');
    const runScenario = async (order: 'stale-before' | 'stale-after') => {
      const sent: { clockId: string; views: DashboardMapView[] }[] = [];
      let emitStates!: (states: DashboardMapState[]) => void;
      const client = {
        available: true,
        async setMapViews(clockId: string, views: DashboardMapView[]) { sent.push({ clockId, views }); },
        async onMapStates(callback: (states: DashboardMapState[]) => void) { emitStates = callback; return () => {}; },
      } as unknown as DashboardClient;
      const playback = clock('page-B');
      const store = new MapViewsStore(client, playback);
      const unregister = store.register('map-1', element, () => ({
        sourceId: 'source',
        path: 'campaign.battleplan',
        revision: 'rev-1',
        branchId: '1',
        playing: false,
        hoursPerSecond: 24,
        hour: 260,
        fraction: 0,
        sequence: 0,
        seekSequence: 0,
      }));
      const stop = store.start(element);
      flushFrames();
      await tick();
      assert.equal(sent.length, 1);
      assert.equal(sent[0].clockId, 'page-B');
      assert.equal(sent[0].views.length, 1);
      assert.equal(sent[0].views[0].clockId, 'page-B');

      const staleA: DashboardMapState = {
        id: 'map-1',
        clockId: 'page-A',
        sourceId: 'source',
        revision: 'rev-1',
        branchId: '1',
        hour: 100,
        fraction: 0,
        sequence: 80,
        requestedSequence: 80,
        loading: false,
        error: null,
      };
      const pausedB: DashboardMapState = {
        id: 'map-1',
        clockId: 'page-B',
        sourceId: 'source',
        revision: 'rev-1',
        branchId: '1',
        hour: 260,
        fraction: 0,
        sequence: 0,
        requestedSequence: 0,
        loading: false,
        error: null,
      };
      if (order === 'stale-before') {
        emitStates([staleA]);
        assert.equal(store.getState('map-1'), null, 'stale A/80 arriving before B/0 must be ignored');
        emitStates([pausedB]);
      } else {
        emitStates([pausedB]);
        emitStates([staleA]);
      }
      const shown = store.getState('map-1');
      assert.ok(shown, `${order}: page B paused state must be displayed`);
      assert.equal(shown.clockId, 'page-B');
      assert.equal(shown.hour, 260);
      assert.equal(shown.sequence, 0);
      assert.equal(shown.loading, false);

      stop();
      unregister();
      await tick();
      assert.equal(sent.length, 2, 'teardown must send an empty clear snapshot');
      assert.equal(sent[1].clockId, 'page-B');
      assert.deepEqual(sent[1].views, []);
    };
    await runScenario('stale-before');
    await runScenario('stale-after');

    const sent: DashboardMapView[][] = [];
    let emitStates!: (states: DashboardMapState[]) => void;
    const client = {
      available: true,
      async setMapViews(_clockId: string, views: DashboardMapView[]) { sent.push(views); },
      async onMapStates(callback: (states: DashboardMapState[]) => void) { emitStates = callback; return () => {}; },
    } as unknown as DashboardClient;
    const playback = clock('page-C');
    const store = new MapViewsStore(client, playback);
    const at = { hour: 260, sequence: 3, seek: 1, playing: false, outside: false };
    store.register('map-1', element, () => at.outside ? null : ({ sourceId: 'source', path: 'campaign.battleplan', revision: 'rev-1', branchId: '1', playing: at.playing,
      hoursPerSecond: 24, hour: at.hour, fraction: 0, sequence: at.sequence, seekSequence: at.seek }));
    const gap = (requestedSequence: number): DashboardMapState => ({ id: 'map-1', clockId: 'page-C', sourceId: 'source', revision: 'rev-1', branchId: null,
      hour: null, fraction: null, sequence: 2, requestedSequence, loading: false, error: 'gap' });
    const stop = store.start(element);
    flushFrames();
    await tick();
    emitStates([gap(3)]);
    flushFrames();
    await tick();
    assert.equal(sent.at(-1)![0].visible, false, 'R22: the target an error was raised for stays hidden');
    assert.equal(store.getState('map-1')?.error, 'gap', 'and keeps its error');
    store.schedule();
    flushFrames();
    await tick();
    assert.equal(store.getState('map-1')?.error, 'gap', 'R22: the same target keeps its error');
    Object.assign(at, { hour: 250, sequence: 4, seek: 2 });
    playback.seek(250);
    flushFrames();
    await tick();
    assert.equal(sent.at(-1)![0].visible, true, 'R22: a new target is asked for, whatever the previous target’s error');
    assert.equal(store.getState('map-1')?.error ?? null, null, 'R22: and carries no error raised for another target');
    Object.assign(at, { sequence: 5, playing: true });
    playback.play(250);
    flushFrames();
    await tick();
    emitStates([gap(5)]);
    flushFrames();
    await tick();
    Object.assign(at, { hour: 251, sequence: 6 });
    store.schedule();
    flushFrames();
    await tick();
    Object.assign(at, { hour: 252, sequence: 7 });
    store.schedule();
    flushFrames();
    await tick();
    assert.equal(sent.at(-1)![0].visible, true, 'R22: playing on across a gap keeps following, the error answering the run it was raised in');
    assert.equal(store.getState('map-1')?.error, 'gap');
    Object.assign(at, { outside: true, playing: false });
    playback.seek(90);
    flushFrames();
    await tick();
    assert.equal(store.getState('map-1'), null, 'R22: a target outside the recording carries no error raised inside it');
    stop();
    await tick();

    const { mapRequests } = await vite.ssrLoadModule('/ui/src/widgets/map/presentation.ts');
    let issued = 0;
    const pinClock = clock('page-D');
    const pinStore = new MapViewsStore(client, pinClock);
    const requests = mapRequests({ issue: () => ++issued });
    const pin = { follow: false, hour: 140 };
    pinStore.register('map-2', element, (now: ReturnType<typeof pinClock.getSnapshot>) => requests({ sourceId: 'source', path: 'campaign.battleplan', revision: 'rev-1',
      branchId: '1', playing: pin.follow && now.playing, hoursPerSecond: now.hoursPerSecond, hour: pin.follow ? now.hour : pin.hour, fraction: 0, seekSequence: pin.follow ? now.seekSequence : undefined }));
    const reply = (requestedSequence: number, error: string | null, hour: number | null = null): DashboardMapState => ({ id: 'map-2', clockId: 'page-D', sourceId: 'source',
      revision: 'rev-1', branchId: error ? null : '1', hour, fraction: hour == null ? null : 0, sequence: requestedSequence, requestedSequence, loading: false, error });
    const sentNow = async () => { flushFrames(); await tick(); return sent.at(-1)!.find(view => view.id === 'map-2')!; };
    const stopPin = pinStore.start(element);
    const pinnedA = await sentNow();
    pin.hour = 150;
    pinStore.schedule();
    const pinnedB = await sentNow();
    assert.ok(pinnedB.sequence > pinnedA.sequence, 'a pin moved while the clock stands still is a new request');
    emitStates([reply(pinnedA.sequence, 'gap')]);
    assert.equal((await sentNow()).visible, true, 'a late error for the old pin does not hide the new one');
    assert.equal(pinStore.getState('map-2')?.error ?? null, null, 'nor is it shown as the new pin’s error');
    emitStates([reply(pinnedB.sequence, 'gap')]);
    assert.equal((await sentNow()).visible, false, 'the new pin’s own error still hides it');
    assert.equal(pinStore.getState('map-2')?.error, 'gap');

    Object.assign(pin, { follow: true });
    pinClock.seek(140);
    const followed = await sentNow();
    emitStates([reply(followed.sequence, null, 140)]);
    Object.assign(pin, { follow: false, hour: 140 });
    pinStore.schedule();
    const pinned = await sentNow();
    assert.ok(pinned.sequence > followed.sequence, 'switching from follow to a pin at the same hour is a new, later request');
    emitStates([reply(followed.sequence, 'gap')]);
    assert.equal((await sentNow()).visible, true, 'an error answering the followed request does not hide the pin');
    assert.equal(pinStore.getState('map-2')?.error ?? null, null);
    emitStates([reply(pinned.sequence, null, 140), reply(followed.sequence, null, 130)]);
    assert.equal(pinStore.getState('map-2')?.requestedSequence, pinned.sequence, 'a reply from before the switch is not taken after the pin’s own reply');

    Object.assign(pin, { follow: true });
    pinClock.play(140);
    const run = await sentNow();
    pinClock.update({ hour: 141, hoursPerSecond: 48 });
    const later = await sentNow();
    assert.ok(later.sequence > run.sequence, 'a playing map recomposed at a later hour is a new request');
    emitStates([reply(run.sequence, 'gap')]);
    flushFrames();
    await tick();
    assert.equal(pinStore.getState('map-2')?.error ?? null, null, 'an error answering an earlier hour of the run is not shown at the later one');
    stopPin();
    await tick();
  } finally {
    await vite.close();
    Object.assign(globalThis, saved);
  }
});
