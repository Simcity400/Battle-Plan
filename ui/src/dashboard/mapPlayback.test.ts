import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
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

test('BattleViewer map and battle-window views give every distinct request a strictly increasing sequence and never step backwards across clock removal', async () => {
  const saved = {
    window: globalThis.window,
    document: globalThis.document,
    ResizeObserver: globalThis.ResizeObserver,
    requestAnimationFrame: globalThis.requestAnimationFrame,
    cancelAnimationFrame: globalThis.cancelAnimationFrame,
    getComputedStyle: globalThis.getComputedStyle,
  };
  const rafs = new Map<number, FrameRequestCallback>();
  let nextRaf = 1;
  const flushFrames = () => {
    const queued = [...rafs.entries()];
    rafs.clear();
    for (const [, callback] of queued) callback(0);
  };
  const rect = () => ({ left: 0, top: 0, right: 200, bottom: 200 });
  const element = { getBoundingClientRect: rect, setAttribute() {}, removeAttribute() {}, closest: () => null, style: { setProperty() {} } } as unknown as HTMLElement;
  class FakeResizeObserver { observe() {} unobserve() {} disconnect() {} }
  Object.assign(globalThis, {
    window: { devicePixelRatio: 1, matchMedia: () => ({ addEventListener() {}, removeEventListener() {} }), addEventListener() {}, removeEventListener() {} },
    document: { hidden: false, documentElement: { clientWidth: 1000, clientHeight: 1000 }, addEventListener() {}, removeEventListener() {} },
    ResizeObserver: FakeResizeObserver,
    requestAnimationFrame: (callback: FrameRequestCallback) => { const id = nextRaf++; rafs.set(id, callback); return id; },
    cancelAnimationFrame: (id: number) => { rafs.delete(id); },
    getComputedStyle: () => ({ backgroundColor: 'rgb(16, 20, 24)', color: 'rgb(240, 244, 248)' }),
  });

  type EffectEntry = { deps: readonly unknown[] | undefined; cleanup: (() => void) | void };
  const makeHooks = () => {
    const slots: unknown[] = [];
    let cursor = 0;
    const same = (a: readonly unknown[] | undefined, b: readonly unknown[] | undefined) =>
      a !== undefined && b !== undefined && a.length === b.length && a.every((item, i) => Object.is(item, b[i]));
    return {
      begin() { cursor = 0; },
      unmount() {
        for (const slot of slots) {
          if (slot && typeof slot === 'object' && 'cleanup' in slot && typeof (slot as EffectEntry).cleanup === 'function') {
            (slot as EffectEntry).cleanup!();
          }
        }
        slots.length = 0;
      },
      useState<T>(init: T | (() => T)): [T, (next: T | ((prev: T) => T)) => void] {
        const at = cursor++;
        if (!(at in slots)) slots[at] = typeof init === 'function' ? (init as () => T)() : init;
        return [slots[at] as T, next => { slots[at] = typeof next === 'function' ? (next as (prev: T) => T)(slots[at] as T) : next; }];
      },
      useRef<T>(init: T): { current: T } {
        const at = cursor++;
        if (!(at in slots)) slots[at] = { current: init === null ? element : init };
        return slots[at] as { current: T };
      },
      useMemo<T>(factory: () => T, deps: readonly unknown[]): T {
        const at = cursor++;
        const prev = slots[at] as { value: T; deps: readonly unknown[] } | undefined;
        if (prev && same(prev.deps, deps)) return prev.value;
        const value = factory();
        slots[at] = { value, deps };
        return value;
      },
      useCallback<T>(fn: T, deps: readonly unknown[]): T {
        const at = cursor++;
        const prev = slots[at] as { value: T; deps: readonly unknown[] } | undefined;
        if (prev && same(prev.deps, deps)) return prev.value;
        slots[at] = { value: fn, deps };
        return fn;
      },
      useEffect(fn: () => (() => void) | void, deps?: readonly unknown[]) {
        const at = cursor++;
        const prev = slots[at] as EffectEntry | undefined;
        if (prev && same(prev.deps, deps)) return;
        prev?.cleanup?.();
        slots[at] = { deps, cleanup: fn() };
      },
      useLayoutEffect(fn: () => (() => void) | void, deps?: readonly unknown[]) {
        const at = cursor++;
        const prev = slots[at] as EffectEntry | undefined;
        if (prev && same(prev.deps, deps)) return;
        prev?.cleanup?.();
        slots[at] = { deps, cleanup: fn() };
      },
      useSyncExternalStore<T>(_sub: () => () => void, get: () => T): T { return get(); },
    };
  };

  let now = 0;
  const clockFrames: (() => void)[] = [];
  const fakeHost = {
    now: () => now,
    next: (run: () => void) => { clockFrames.push(run); return () => { const i = clockFrames.indexOf(run); if (i >= 0) clockFrames.splice(i, 1); }; },
  };
  const advanceClock = (ms: number) => { now += ms; const due = clockFrames.splice(0); due.forEach(run => run()); };

  const captured: {
    setView: ((next: 'battle' | 'map' | 'details') => void) | undefined;
    clock: import('../components/battleClock.ts').BattleClock | null;
  } = { setView: undefined, clock: null };
  const testHost = globalThis as typeof globalThis & {
    __bvHooks?: ReturnType<typeof makeHooks>;
    __bvSetView?: (fn: (next: 'battle' | 'map' | 'details') => void) => void;
    __bvClockHost?: typeof fakeHost;
    __bvCapturedClock?: (clock: import('../components/battleClock.ts').BattleClock) => void;
  };
  testHost.__bvSetView = fn => { captured.setView = fn; };
  testHost.__bvClockHost = fakeHost;
  testHost.__bvCapturedClock = created => { captured.clock = created; };

  const vite = await createServer({
    configFile: false,
    optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{
      name: 'battle-viewer-sequence-test',
      enforce: 'pre',
      transform(code, id) {
        const norm = id.replaceAll('\\', '/');
        if (norm.endsWith('/components/BattleViewer.tsx')) {
          return code
            .replace("from 'react'", "from 'bv-test-react'")
            .replace("from '../families/host/DetailWindowHost.tsx'", "from 'bv-test-detailHost'")
            .replace("from '../charts/tokens.ts'", "from 'bv-test-tokens'")
            .replace("from './FailureLine'", "from 'bv-test-failure'")
            .replace("from './Icons'", "from 'bv-test-icons'")
            .replace("from './Segmented'", "from 'bv-test-segmented'")
            .replace("from './battleClock.ts'", "from 'bv-test-clock'")
            .replace("import './battleViewer.css';", '');
        }
      },
      resolveId(id) {
        if (id.startsWith('bv-test-')) return `\0${id}`;
      },
      load(id) {
        if (id === '\0bv-test-react') {
          return `export const useCallback = (fn, deps) => globalThis.__bvHooks.useCallback(fn, deps);
            export const useEffect = (fn, deps) => globalThis.__bvHooks.useEffect(fn, deps);
            export const useLayoutEffect = (fn, deps) => globalThis.__bvHooks.useLayoutEffect(fn, deps);
            export const useMemo = (fn, deps) => globalThis.__bvHooks.useMemo(fn, deps);
            export const useRef = init => globalThis.__bvHooks.useRef(init);
            export const useState = init => globalThis.__bvHooks.useState(init);
            export const useSyncExternalStore = (sub, get) => globalThis.__bvHooks.useSyncExternalStore(sub, get);`;
        }
        if (id === '\0bv-test-detailHost') {
          return `import { createElement, Fragment } from 'react';
            export const DetailWindowHost = props => createElement(Fragment, null, props.headerActions, props.children);
            export const useDetailWindowClose = () => undefined;`;
        }
        if (id === '\0bv-test-tokens') return `export const subscribeTokens = () => () => {};`;
        if (id === '\0bv-test-failure') return `export const FailureLine = () => null;`;
        if (id === '\0bv-test-icons') return `export const Icon = () => null;`;
        if (id === '\0bv-test-segmented') {
          return `export const Segmented = props => { globalThis.__bvSetView(props.onChange); return null; };`;
        }
        if (id === '\0bv-test-clock') {
          return `import { BattleClock as RealBattleClock, issueSequence, sharedPause } from '/ui/src/components/battleClock.ts';
            export { issueSequence, sharedPause };
            export class BattleClock extends RealBattleClock {
              constructor(span, start, host) {
                super(span, start, host ?? globalThis.__bvClockHost);
                globalThis.__bvCapturedClock(this);
              }
            }`;
        }
      },
    }],
  });

  try {
    const { MapViewsStore } = await vite.ssrLoadModule('/ui/src/dashboard/geometry.ts');
    const { BattleViewer, BattleGameWindow } = await vite.ssrLoadModule('/ui/src/components/BattleViewer.tsx') as typeof import('../components/BattleViewer.tsx');
    const { BattleClock } = await vite.ssrLoadModule('/ui/src/components/battleClock.ts') as typeof import('../components/battleClock.ts');

    const sent: DashboardMapView[][] = [];
    let emitStates!: (states: DashboardMapState[]) => void;
    const client = {
      available: true,
      nativeMaps: true,
      async setMapViews(_clockId: string, views: DashboardMapView[]) { sent.push(views); },
      async onMapStates(callback: (states: DashboardMapState[]) => void) { emitStates = callback; return () => {}; },
    } as unknown as DashboardClient;
    let pageIssued = 0;
    const pageClock = Object.assign(clock('page-battle'), { issue: () => ++pageIssued });
    const store = new MapViewsStore(client, pageClock);
    const stop = store.start(element);
    const runtime = { maps: store, clock: pageClock, client, nativeHidden: false } as unknown as import('./definitions.ts').WidgetRuntime;
    const battleTrack = {
      id: 'main',
      source: { id: 'source', path: 'campaign.battleplan', revision: 'rev-1' },
      branch: { id: '1', firstHour: 50, lastHour: 300 },
      selectedTag: 'AAA',
      selectedSlot: 1,
    } as unknown as BoundTrack;
    const sentView = async (prefix: string) => {
      flushFrames();
      await tick();
      return sent.at(-1)!.find(view => view.id.startsWith(prefix))!;
    };
    const reply = (id: string, requestedSequence: number, error: string | null, hour: number | null = null): DashboardMapState => ({
      id, clockId: 'page-battle', sourceId: 'source', revision: 'rev-1', branchId: error ? null : '1',
      hour, fraction: hour == null ? null : 0, sequence: requestedSequence, requestedSequence, loading: false, error,
    });

    const mapHooks = makeHooks();
    testHost.__bvHooks = mapHooks;
    const renderViewer = (span: { first: number; last: number } | null, province: number) => {
      mapHooks.begin();
      renderToStaticMarkup(createElement(BattleViewer, {
        subject: 'land:42', title: 'Battle', day: null, closeLabel: 'Close',
        anchor: { current: null }, runtime, track: battleTrack, province, span,
        unplayable: null, gaps: [], fallbackHour: 100,
        sides: () => null, battle: () => null, onClose: () => {},
      }));
    };
    renderViewer(null, 10);
    assert.ok(captured.setView, 'BattleViewer mounts view switcher');
    captured.setView('map');
    renderViewer(null, 10);
    const m1 = await sentView('battle-map-');
    assert.equal(m1.hour, 100);
    assert.equal(m1.focusProvince, 10);

    renderViewer({ first: 100, last: 200 }, 10);
    assert.ok(captured.clock, 'BattleViewer creates BattleClock when span becomes available');
    captured.clock.play();
    const mPlay1 = await sentView('battle-map-');
    assert.ok(mPlay1.sequence > m1.sequence, 'starting BattleMap playback issues a greater sequence');
    advanceClock(1000);
    store.schedule();
    const mPlay2 = await sentView('battle-map-');
    assert.ok(mPlay2.hour > mPlay1.hour, 'BattleClock advances while playing');
    assert.ok(mPlay2.sequence > mPlay1.sequence, 'BattleMap recomposed at a later playing hour gets a greater sequence');
    emitStates([reply(mPlay2.id, mPlay2.sequence, null, mPlay2.hour)]);

    renderViewer(null, 10);
    const mNull = await sentView('battle-map-');
    assert.equal(mNull.id, mPlay2.id, 'same BattleMap view stays mounted across clock -> null');
    assert.ok(mNull.sequence > mPlay2.sequence, 'clock -> null transition must never step sequence backwards');
    emitStates([reply(mNull.id, mNull.sequence, null, mNull.hour)]);
    assert.equal(store.getState(mNull.id)?.hour, 100, 'reply after clock -> null is accepted rather than dropped as older');

    emitStates([reply(mNull.id, mNull.sequence, 'province failed')]);
    assert.equal((await sentView('battle-map-')).visible, false);
    renderViewer(null, 20);
    const mProv2 = await sentView('battle-map-');
    assert.equal(mProv2.focusProvince, 20);
    assert.ok(mProv2.sequence > mNull.sequence, 'changing BattleMap province at the same hour issues a greater sequence');
    assert.equal(mProv2.visible, true, 'old province error does not hide the new province request');
    assert.equal(store.getState(mProv2.id)?.error ?? null, null);
    mapHooks.unmount();

    const winHooks = makeHooks();
    testHost.__bvHooks = winHooks;
    const gameClock = new BattleClock({ first: 100, last: 200 }, 100, fakeHost);
    const live: readonly (readonly [number, number])[] = [[100, 200]];
    const renderGameWindow = (lead: { slot: number; tag: string }) => {
      winHooks.begin();
      renderToStaticMarkup(createElement(BattleGameWindow, {
        runtime, track: battleTrack, clock: gameClock, combat: '42',
        lead, live, until: null, ended: null,
      }));
    };
    renderGameWindow({ slot: 1, tag: 'AAA' });
    const w1 = await sentView('battle-window-');

    gameClock.play();
    const w2 = await sentView('battle-window-');
    assert.ok(w2.sequence > w1.sequence);
    advanceClock(1000);
    const w3 = await sentView('battle-window-');
    assert.ok(w3.hour > w2.hour);
    assert.ok(w3.sequence > w2.sequence, 'BattleGameWindow advancing hour while playing issues a greater sequence');
    emitStates([reply(w3.id, w2.sequence, 'stale play error')]);
    flushFrames();
    await tick();
    assert.equal(store.getState(w3.id)?.error ?? null, null, 'late error from earlier playing hour does not show at later hour');

    gameClock.pause();
    const w4 = await sentView('battle-window-');
    assert.ok(w4.sequence > w3.sequence);
    emitStates([reply(w4.id, w4.sequence, 'lead failed')]);
    assert.equal((await sentView('battle-window-')).visible, false);
    renderGameWindow({ slot: 2, tag: 'BBB' });
    const w5 = await sentView('battle-window-');
    assert.equal(w5.id, w4.id, 'lead change updates the same registered battle window');
    assert.equal(w5.battle?.country, 'BBB', 'outgoing request names the new lead');
    assert.ok(w5.sequence > w4.sequence, 'changing BattleGameWindow lead at the same paused hour issues a greater sequence');
    assert.equal(w5.visible, true, 'error for previous lead does not hide the new lead request');
    assert.equal(store.getState(w5.id)?.error ?? null, null);

    gameClock.seek(120);
    const w6 = await sentView('battle-window-');
    assert.ok(w6.sequence > w5.sequence);
    assert.ok(w6.sequence > gameClock.movedSequence, 'post-seek BattleGameWindow sequence stays above BattleClock.movedSequence');
    gameClock.acknowledge(108, w1.sequence);
    assert.equal(gameClock.seenThrough, null, 'pre-seek reply is still rejected by BattleClock.acknowledge');
    gameClock.acknowledge(120, w6.sequence);
    assert.equal(gameClock.seenThrough, 120, 'post-seek reply is accepted by BattleClock.acknowledge');
    winHooks.unmount();
    gameClock.dispose();

    stop();
    await tick();
  } finally {
    delete testHost.__bvHooks;
    delete testHost.__bvSetView;
    delete testHost.__bvClockHost;
    delete testHost.__bvCapturedClock;
    await vite.close();
    Object.assign(globalThis, saved);
  }
});
