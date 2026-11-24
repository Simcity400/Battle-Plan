import assert from 'node:assert/strict';
import test from 'node:test';
import { PerfRecorder, auditInBatches, pictureError } from './perf.ts';

test('audit work stays bounded while retaining every result in input order', async () => {
  let active = 0;
  let peak = 0;
  const visited: number[] = [];
  let started = 0, release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const result = await auditInBatches(Array.from({ length: 19 }, (_, i) => i), 3, async value => {
    active++;
    peak = Math.max(peak, active);
    if (++started === 3) release();
    visited.push(value);
    await gate;
    active--;
    return value * 2;
  });

  assert.equal(peak, 3, `audit only started ${peak} checks at once`);
  assert.deepEqual(result, Array.from({ length: 19 }, (_, i) => i * 2));
  assert.deepEqual([...visited].sort((a, b) => a - b), Array.from({ length: 19 }, (_, i) => i));
});

test('review 5 finding 18: a picture matches its exact read in displayed order, by row identity, and never as two empty lists', async () => {
  const { panelAuditRows } = await import('../dashboard/follow/panelAudit.ts');
  const shown = [{ row: 'B', key: 'B:v', value: '2' }, { row: 'A', key: 'A:v', value: '1' }];
  const exact = new Map([['A:v', '1'], ['B:v', '2']]);
  assert.equal(pictureError('table', shown, { values: exact }), null, 'unordered reads compare by key');
  assert.match(pictureError('table', shown, { values: exact, ordered: true }) ?? '', /order/, 'a table compares in the order it shows');
  assert.match(pictureError('heat-matrix', [...shown, { row: 'A', key: 'A:v', value: '1' }], { values: exact, ordered: false }) ?? '', /twice/, 'R3-S12: an unordered comparison refuses a painted key shown twice, even with its right value');
  const live = panelAuditRows('wargoals', { goals: [], justifications: [] }, new Map([['J1\u0000points', 5]]));
  assert.equal(pictureError('page', live, { values: new Map(live.map(row => [row.key, row.value])) }), null, 'live cells keep their identity');
  const owned = panelAuditRows('wargoals', { goals: [{ handle: 1, actor: 'GER', recipient: 'POL', name: 'Conquer', available: true, expires: null }], justifications: [] }, null);
  assert.equal(owned.length, 1, 'owned goals alone still give rows to compare');
});

test('review 6 D: a picture missing part of its exact read fails, as one goal of two does', () => {
  const exact = new Map([['["goal",1]', '["goal",1]'], ['["goal",2]', '["goal",2]']]);
  assert.match(pictureError('page', [{ row: '["goal",1]', key: '["goal",1]', value: '["goal",1]' }], { values: exact }) ?? '', /not shown/);
});

test('capturePicturesBatch freezes its snapshot on the first batch so post-window frames cannot extend the drain', async () => {
  const { auditing, capturePictures, capturePicturesBatch, recordPicture } = await import('../dashboard/pulled.ts');
  await capturePictures(true);
  assert.equal(auditing(), true);
  const sample = (at: number) => ({ t: 'picture', at, widget: 'w', surface: 'table', placement: 1, owner: 0, renderer: 'page', head: at, hour: at, offset: 0, since: at, through: at, eligibleHour: at, entry: at, parts: [], face: '', drawMs: 0, mode: 'picture', dim: false, reset: 0, binding: 0, inspect: null, tooltip: false }) as never;
  recordPicture(sample(1));
  recordPicture(sample(2));
  recordPicture(sample(3));
  const first = await capturePicturesBatch(2);
  assert.equal(auditing(), false);
  assert.equal(first.done, false);
  assert.deepEqual(first.records.map(r => r.at), [1, 2]);
  recordPicture(sample(4));
  const second = await capturePicturesBatch(2);
  assert.equal(second.done, true);
  assert.deepEqual(second.records.map(r => r.at), [3]);
});

test('beginAudit clears any unfinished pictureAudit and cached exactReads before the next window', async () => {
  const { capturePictures, capturePicturesBatch, recordPicture } = await import('../dashboard/pulled.ts');
  const recorder = Object.create(PerfRecorder.prototype) as PerfRecorder;
  Object.assign(recorder as unknown as Record<string, unknown>, {
    readers: new Map(),
    exactReads: new Map(),
    auditGen: 0,
    lines: [],
    buffered: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    lost: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    sending: 0,
    stopped: false,
    push: () => {},
    flush: async () => {},
  });

  let reads = 0;
  await recorder.exactOnce('stale-key', async () => ++reads);
  assert.equal(reads, 1);

  await recorder.beginAudit();
  const sample = (widget: string, at: number) => ({ t: 'picture', at, widget, surface: 'table', placement: 1, owner: 0, renderer: 'page', head: at, hour: at, offset: 0, since: at, through: at, eligibleHour: at, entry: at, parts: [], face: '', drawMs: 0, mode: 'picture', dim: false, reset: 0, binding: 0, inspect: null, tooltip: false, rows: [{ row: 'A', key: 'A:v', value: '1' }] }) as never;
  recordPicture(sample('old-widget', 1));
  recordPicture(sample('old-widget', 2));
  const partial = await recorder.auditPicturesBatch(1);
  assert.equal(partial.done, false);
  assert.equal(partial.results[0]!.widget, 'old-widget');

  await recorder.beginAudit();
  await recorder.exactOnce('stale-key', async () => ++reads);
  assert.equal(reads, 2, 'beginAudit cleared exactReads cache');
  recordPicture(sample('new-widget', 10));
  const next = await recorder.auditPicturesBatch(10);
  assert.equal(next.done, true);
  assert.deepEqual(next.results.map(r => r.widget), ['new-widget'], 'stale pictureAudit from the abandoned window did not leak');
  await capturePictures(false);
  await capturePicturesBatch(100);
});

test('R4-S6: each audited picture hands its own installed fields to the exact reader', async () => {
  const { capturePictures, capturePicturesBatch, recordPicture } = await import('../dashboard/pulled.ts');
  const recorder = Object.create(PerfRecorder.prototype) as PerfRecorder;
  Object.assign(recorder as unknown as Record<string, unknown>, {
    readers: new Map(), exactReads: new Map(), auditGen: 0, lines: [],
    buffered: { count: 0, bytes: 0, from: Infinity, to: -Infinity }, lost: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    sending: 0, stopped: false, push: () => {}, flush: async () => {},
  });
  const seen: unknown[] = [];
  recorder.reader(1, 0, async (_hour, _auditId, fields) => { seen.push(fields?.range); return { values: new Map([['A:v', '1']]) }; });
  await recorder.beginAudit();
  const sample = (at: number, fields: Readonly<Record<string, unknown>>) => ({ t: 'picture', at, widget: 'w', surface: 'table', placement: 1, owner: 0, renderer: 'page', head: at, hour: at, offset: 0, since: at, through: at,
    eligibleHour: at, entry: at, parts: [], face: '', drawMs: 0, mode: 'picture', dim: false, reset: 0, binding: 0, inspect: null, tooltip: false, auditId: 3, fields, rows: [{ row: 'A', key: 'A:v', value: '1' }] }) as never;
  recordPicture(sample(1, { range: { first: 0, count: 4 } }));
  recordPicture(sample(2, { range: { first: 0, count: 2 } }));
  const results = await recorder.auditPictures();
  assert.deepEqual(results.map(result => result.error), [null, null]);
  assert.deepEqual(seen, [{ first: 0, count: 4 }, { first: 0, count: 2 }], 'the earlier sample keeps its own range when a later one with the same audit id installs');
  await capturePictures(false);
  await capturePicturesBatch(100);
});

test('beginAudit invalidates in-flight audit continuations via generation token so stale drains cannot touch the next window', async () => {
  const { capturePictures, capturePicturesBatch, recordPicture } = await import('../dashboard/pulled.ts');
  const recorder = Object.create(PerfRecorder.prototype) as PerfRecorder;
  Object.assign(recorder as unknown as Record<string, unknown>, {
    readers: new Map(),
    exactReads: new Map(),
    auditGen: 0,
    lines: [],
    buffered: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    lost: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    sending: 0,
    stopped: false,
    push: () => {},
    flush: async () => {},
  });

  let releaseStale!: () => void;
  const staleGate = new Promise<void>(resolve => { releaseStale = resolve; });
  let staleReadCalls = 0;
  recorder.reader(1, 0, async () => {
    staleReadCalls++;
    await staleGate;
    return { values: new Map([['A:v', 'old-expected']]) };
  });

  await recorder.beginAudit();
  let secondStructuralReads = 0;
  let releaseFirstStructural!: () => void;
  const firstStructuralGate = new Promise<void>(resolve => { releaseFirstStructural = resolve; });
  recorder.structure(
    'old-widget',
    'table',
    { hour: 1, rows: () => ['a'], exact: async () => { await firstStructuralGate; return ['a']; } },
    { hour: 2, rows: () => ['b'], exact: async () => { secondStructuralReads++; return ['b']; } },
  );
  (recorder as unknown as { commitAudit: unknown[] }).commitAudit.push({
    at: 1, serial: 1, widgets: ['old-widget'], causes: {},
    changes: (recorder as unknown as { rowChanges: unknown[] }).rowChanges,
  });

  const sample = (widget: string, at: number, value: string) => ({ t: 'picture', at, widget, surface: 'table', placement: 1, owner: 0, renderer: 'page', head: at, hour: at, offset: 0, since: at, through: at, eligibleHour: at, entry: at, parts: [], face: '', drawMs: 0, mode: 'picture', dim: false, reset: 0, binding: 0, inspect: null, tooltip: false, rows: [{ row: 'A', key: 'A:v', value }] }) as never;
  for (let i = 1; i <= 12; i++) recordPicture(sample('old-widget', i, 'old-expected'));
  const inFlightOld = recorder.auditPicturesBatch(12);

  let newReadCalls = 0;
  recorder.reader(1, 0, async () => {
    newReadCalls++;
    return { values: new Map([['A:v', 'new-expected']]) };
  });
  await recorder.beginAudit();
  recordPicture(sample('new-widget', 100, 'new-expected'));

  releaseFirstStructural();
  releaseStale();
  const staleOutcome = await inFlightOld;
  assert.deepEqual(staleOutcome, { done: true, results: [] }, 'stale continuation returns empty and does not touch new audit');
  assert.equal(secondStructuralReads, 0, 'stale commit audit does not start second structural read after beginAudit');
  assert.equal(staleReadCalls, 0, 'stale picture audit does not start queued picture reads after beginAudit');

  const next = await recorder.auditPicturesBatch(10);
  assert.equal(next.done, true);
  assert.equal(newReadCalls, 1);
  assert.deepEqual(next.results.map(r => [r.widget, r.error]), [['new-widget', null]]);
  await capturePictures(false);
  await capturePicturesBatch(100);
});

test('perf audit keys include pipeline generation and unregistering live readers keeps readers captured for the current audit', async () => {
  const { PerfRecorder } = await import('./perf.ts');
  const { capturePictures, capturePicturesBatch, recordPicture } = await import('../dashboard/pulled.ts');
  const recorder = Object.create(PerfRecorder.prototype) as InstanceType<typeof PerfRecorder>;
  Object.assign(recorder, {
    readers: new Map(),
    rowChanges: [],
    exactReads: new Map(),
    auditGen: 0,
    lines: [],
    buffered: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    lost: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    sending: 0,
    stopped: false,
    push: () => {},
    flush: async () => {},
  });

  const stopGen1 = recorder.reader(1, 0, async () => ({ values: new Map([['A:v', 'gen1-val']]) }), 1);
  await recorder.beginAudit();
  const sample = (widget: string, generation: number, value: string) => ({
    t: 'picture', at: generation, widget, surface: 'table', placement: 1, owner: 0, generation,
    renderer: 'page', head: 10, hour: 10, offset: 0, since: 10, through: 10, eligibleHour: 10, entry: generation,
    parts: [], face: '', drawMs: 0, mode: 'picture', dim: false, reset: 0, binding: 0, inspect: null, tooltip: false,
    rows: [{ row: 'A', key: 'A:v', value }],
  }) as never;
  recordPicture(sample('w-gen1', 1, 'gen1-val'));

  stopGen1();
  recorder.dropGeneration(1);
  recorder.reader(1, 0, async () => ({ values: new Map([['A:v', 'gen2-val']]) }), 2);
  recordPicture(sample('w-gen2', 2, 'gen2-val'));

  const batch = await recorder.auditPicturesBatch(10);
  assert.equal(batch.done, true);
  assert.deepEqual(batch.results.map(r => [r.widget, r.error]), [['w-gen1', null], ['w-gen2', null]],
    'each generation picture is audited against its own generation reader without colliding on reused placement/owner ids');

  await recorder.beginAudit();
  recordPicture(sample('w-gen1-stale', 1, 'gen1-val'));
  const afterDrop = await recorder.auditPicturesBatch(10);
  assert.equal(afterDrop.results[0]?.error, 'Displayed rows or exact read missing',
    'disposed generation readers are removed from live registrations before later audits');
  await capturePictures(false);
  await capturePicturesBatch(100);
});

test('stock vite config stays permanently gated while vite.ungated.config.ts enables ungated mode', async () => {
  const stockMod = await import('../../vite.config.ts');
  const ungatedMod = await import('../../vite.ungated.config.ts');
  const stockFactory = stockMod.default as (env: { command: string; mode: string }) => { define?: Record<string, string> };
  const ungatedFactory = ungatedMod.default as (env: { command: string; mode: string }) => { define?: Record<string, string> };
  const prev = process.env.BATTLEPLAN_MEASURE_UNGATED;
  try {
    process.env.BATTLEPLAN_MEASURE_UNGATED = '1';
    for (const mode of ['production', 'development', 'measure-ungated']) {
      assert.equal(stockFactory({ command: 'build', mode }).define?.__BATTLEPLAN_MEASURE_UNGATED__, 'false');
    }
    assert.equal(ungatedFactory({ command: 'build', mode: 'production' }).define?.__BATTLEPLAN_MEASURE_UNGATED__, 'true');
  } finally {
    if (prev === undefined) delete process.env.BATTLEPLAN_MEASURE_UNGATED;
    else process.env.BATTLEPLAN_MEASURE_UNGATED = prev;
  }
});


function installPerfEnv() {
  const saved = new Map(['requestAnimationFrame', 'cancelAnimationFrame', 'PerformanceObserver', 'devicePixelRatio', 'innerWidth', 'innerHeight', 'setInterval', 'clearInterval', 'setTimeout', 'clearTimeout', 'MessageChannel', 'performance', 'window']
    .map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  let now = 0;
  const rafs = new Map<number, FrameRequestCallback>();
  let nextRaf = 0, rafCount = 0;
  let intervalCalls = 0;
  const timeouts = new Map<number, () => void>();
  let nextTimeout = 0;
  const pendingMessages: (() => void)[] = [];
  const observers = new Map<string, Set<{ cb: PerformanceObserverCallback; disconnected: boolean }>>();
  class FakeChannel {
    port1 = { onmessage: null as ((event: { data: number }) => void) | null };
    port2 = { postMessage: (data: number) => { pendingMessages.push(() => this.port1.onmessage?.({ data })); } };
  }
  class FakeObserver {
    static supportedEntryTypes: string[] = ['longtask', 'long-animation-frame'];
    private readonly entry: { cb: PerformanceObserverCallback; disconnected: boolean };
    private types: string[] = [];
    constructor(cb: PerformanceObserverCallback) {
      this.entry = { cb, disconnected: false };
    }
    observe(options: { type: string }) {
      this.types.push(options.type);
      let set = observers.get(options.type);
      if (!set) observers.set(options.type, set = new Set());
      set.add(this.entry);
    }
    disconnect() {
      this.entry.disconnected = true;
      for (const type of this.types) observers.get(type)?.delete(this.entry);
    }
  }
  const emitObserver = (type: string, entries: unknown[]) => {
    const list = { getEntries: () => entries } as unknown as PerformanceObserverEntryList;
    for (const obs of [...(observers.get(type) ?? [])]) {
      if (!obs.disconnected) obs.cb(list, {} as PerformanceObserver);
    }
  };
  const flushMessages = () => {
    while (pendingMessages.length) pendingMessages.shift()!();
  };
  const runFrame = (at: number) => {
    const batch = [...rafs.entries()];
    if (!batch.length) return false;
    for (const [id] of batch) rafs.delete(id);
    now = at;
    for (const [, cb] of batch) cb(at);
    flushMessages();
    return true;
  };
  const runNextRaf = (at: number) => {
    const first = rafs.entries().next().value as [number, FrameRequestCallback] | undefined;
    if (!first) return false;
    rafs.delete(first[0]);
    now = at;
    first[1](at);
    flushMessages();
    return true;
  };
  const runAllTimeouts = () => {
    for (const [id, fn] of [...timeouts.entries()]) {
      timeouts.delete(id);
      fn();
    }
  };
  const setNow = (value: number) => { now = value; };
  Object.assign(globalThis, {
    requestAnimationFrame: (cb: FrameRequestCallback) => { rafCount++; const id = ++nextRaf; rafs.set(id, cb); return id; },
    cancelAnimationFrame: (id: number) => { rafs.delete(id); },
    PerformanceObserver: FakeObserver,
    devicePixelRatio: 1,
    innerWidth: 1280,
    innerHeight: 720,
    setInterval: () => { intervalCalls++; return 1 as unknown as ReturnType<typeof setInterval>; },
    clearInterval: () => {},
    setTimeout: (fn: () => void) => { const id = ++nextTimeout; timeouts.set(id, fn); return id as unknown as ReturnType<typeof setTimeout>; },
    clearTimeout: (id: number) => { timeouts.delete(id); },
    MessageChannel: FakeChannel,
    performance: { timeOrigin: 0, now: () => now },
    window: {},
  });
  return {
    rafs,
    get rafCount() { return rafCount; },
    get intervalCalls() { return intervalCalls; },
    setNow,
    runFrame,
    runNextRaf,
    flushMessages,
    runAllTimeouts,
    emitObserver,
    restore() {
      for (const [key, desc] of saved) {
        if (desc) Object.defineProperty(globalThis, key, desc);
        else Reflect.deleteProperty(globalThis, key);
      }
    },
  };
}

test('PerfRecorder schedules no animation frames or flush timers while paused and idle, and wakes on playback or activity', async () => {
  const env = installPerfEnv();
  let recorder: PerfRecorder | null = null;
  try {
    recorder = new PerfRecorder();
    const lines = (recorder as unknown as { lines: string[] }).lines;
    assert.equal(env.intervalCalls, 0, 'does not run a permanent polling interval while idle');
    assert.equal(env.rafs.size, 0, 'starts idle with no animation frame scheduled while paused');

    env.setNow(10);
    recorder.clock(100, false, 24, 1, 1);
    assert.equal(env.rafs.size, 0, 'paused clock tick without work keeps the recorder asleep');

    env.setNow(100);
    recorder.count('w1', 'render');
    assert.equal(env.rafs.size, 1, 'wakes on widget work while paused');
    env.runFrame(116);
    assert.equal(env.rafs.size, 0, 'sleeps immediately once the work frame is recorded');
    const recorded = lines.map(line => JSON.parse(line) as { t: string; playing?: boolean; work?: Record<string, unknown> });
    assert.ok(recorded.some(r => r.t === 'frame' && r.work?.w1), 'recorded the paused work frame');

    env.setNow(200);
    recorder.clock(101, true, 24, 2, 1);
    assert.equal(env.rafs.size, 1, 'starts continuous frames while playing');
    env.runFrame(216);
    assert.equal(env.rafs.size, 1, 'keeps scheduling frames while playing');
    env.runFrame(232);
    assert.equal(env.rafs.size, 1, 'still scheduling frames while playing');

    env.setNow(240);
    recorder.clock(102, false, 24, 3, 1);
    env.runFrame(248);
    assert.equal(env.rafs.size, 0, 'stops scheduling frames once playback pauses and the final frame is recorded');
  } finally {
    if (recorder) {
      const stop = recorder.finish();
      env.runFrame(999);
      env.runAllTimeouts();
      await stop.catch(() => {});
    }
    env.restore();
  }
});

test('F6: Play after idle records the stall interval before the first animation callback without charging idle time', async () => {
  const env = installPerfEnv();
  let recorder: PerfRecorder | null = null;
  try {
    env.setNow(100);
    recorder = new PerfRecorder();
    const lines = (recorder as unknown as { lines: string[] }).lines;
    assert.equal(env.rafs.size, 0);

    env.setNow(30_100);
    recorder.clock(100, true, 24, 1, 1);
    assert.equal(env.rafs.size, 1);
    env.runFrame(30_185);
    const frames = lines.map(line => JSON.parse(line) as { t: string; dt?: number; playing?: boolean }).filter(r => r.t === 'frame');
    assert.equal(frames.length, 1);
    assert.equal(frames[0]!.playing, true);
    assert.equal(frames[0]!.dt, 85, 'records the 85ms Play stall without charging the 30s idle gap');
  } finally {
    if (recorder) {
      const stop = recorder.finish();
      env.runFrame(31_000);
      env.runAllTimeouts();
      await stop.catch(() => {});
    }
    env.restore();
  }
});

test('F7: finish() waits through one frame so a microtask commit and a pending observer entry are captured before detaching', async () => {
  const env = installPerfEnv();
  const hook = (globalThis as { __REACT_DEVTOOLS_GLOBAL_HOOK__?: { onCommitFiberRoot: (r: number, root: { current: unknown }) => void } }).__REACT_DEVTOOLS_GLOBAL_HOOK__!;
  const root = { current: { child: null, sibling: null, return: null, alternate: null, flags: 0, subtreeFlags: 0, type: null, elementType: null, memoizedProps: null, memoizedState: null, dependencies: null } };
  let recorder: PerfRecorder | null = null;
  try {
    env.setNow(100);
    recorder = new PerfRecorder();
    const flushed: string[] = [];
    (recorder as unknown as { flush: () => Promise<void> }).flush = async () => {
      const lines = (recorder as unknown as { lines: string[] }).lines;
      flushed.push(...lines.splice(0));
    };

    assert.equal(env.rafs.size, 0, 'idle before finish()');
    env.setNow(500);
    queueMicrotask(() => {
      hook.onCommitFiberRoot(1, root);
      env.emitObserver('long-animation-frame', [{ startTime: 490, duration: 60, blockingDuration: 10, renderStart: 540, scripts: [{ duration: 40, invoker: 'fn', sourceURL: 'app.js' }] }]);
    });
    const finishing = recorder.finish();
    await Promise.resolve();
    env.runFrame(516);
    env.runAllTimeouts();
    await finishing;
    recorder = null;

    const records = flushed.map(line => JSON.parse(line) as { t: string; commits?: number; commitSequence?: number; dur?: number });
    assert.ok(records.some(r => r.t === 'frame' && r.commits === 1 && r.commitSequence === 1), 'microtask commit queued when finish() starts is captured before detach');
    assert.ok(records.some(r => r.t === 'loaf' && r.dur === 60), 'observer entry awaiting delivery is captured before disconnect');
  } finally {
    if (recorder) {
      const stop = recorder.finish();
      env.runFrame(999);
      env.runAllTimeouts();
      await stop.catch(() => {});
    }
    env.restore();
  }
});

test('G2: stop() during playback attaches a pending paint to the next real animation frame without inventing an extra interval', async () => {
  const env = installPerfEnv();
  let recorder: PerfRecorder | null = null;
  try {
    env.setNow(100);
    recorder = new PerfRecorder();
    const flushed: string[] = [];
    (recorder as unknown as { flush: () => Promise<void> }).flush = async () => {
      const lines = (recorder as unknown as { lines: string[] }).lines;
      flushed.push(...lines.splice(0));
    };
    await recorder.start();

    env.setNow(100);
    recorder.clock(10, true, 24, 1, 1);
    env.runFrame(116);
    recorder.paint('w-play', 3.5, 116);

    env.setNow(120);
    const stopping = recorder.stop().catch(() => {});
    env.runFrame(132);
    env.runAllTimeouts();
    await stopping;

    const records = flushed.map(line => JSON.parse(line) as { t: string; at?: number; dt?: number; playing?: boolean; paint?: Record<string, number>; paint_ms?: number });
    const startIdx = records.findIndex(r => r.t === 'start');
    const stopIdx = records.findIndex(r => r.t === 'stop');
    const frames = records.slice(startIdx + 1, stopIdx).filter(r => r.t === 'frame');
    assert.deepEqual(
      frames.map(f => ({ at: f.at, dt: f.dt, playing: f.playing, paint: f.paint, paint_ms: f.paint_ms })),
      [
        { at: 116, dt: 16, playing: true, paint: undefined, paint_ms: undefined },
        { at: 132, dt: 16, playing: true, paint: { 'w-play': 3.5 }, paint_ms: 3.5 },
      ],
      'emits exactly one 16ms interval per real animation frame and attaches the 116ms paint to the 132ms frame',
    );
  } finally {
    if (recorder) {
      const stop = recorder.finish();
      env.runFrame(999);
      env.runAllTimeouts();
      await stop.catch(() => {});
    }
    env.restore();
  }
});

test('F8: commits before PerfRecorder attaches are not counted in the first emitted frame', async () => {
  const env = installPerfEnv();
  const hook = (globalThis as { __REACT_DEVTOOLS_GLOBAL_HOOK__?: { onCommitFiberRoot: (r: number, root: { current: unknown }) => void } }).__REACT_DEVTOOLS_GLOBAL_HOOK__;
  assert.ok(hook, 'commitCount installs the fiber commit hook');
  const root = { current: { child: null, sibling: null, return: null, alternate: null, flags: 0, subtreeFlags: 0, type: null, elementType: null, memoizedProps: null, memoizedState: null, dependencies: null } };
  let recorder: PerfRecorder | null = null;
  try {
    hook.onCommitFiberRoot(1, root);
    hook.onCommitFiberRoot(1, root);
    hook.onCommitFiberRoot(1, root);
    env.setNow(100);
    recorder = new PerfRecorder();
    const lines = (recorder as unknown as { lines: string[] }).lines;
    env.setNow(120);
    hook.onCommitFiberRoot(1, root);
    env.runFrame(136);
    const frames = lines.map(line => JSON.parse(line) as { t: string; commits?: number; commitSequence?: number }).filter(r => r.t === 'frame');
    assert.equal(frames.length, 1);
    assert.equal(frames[0]!.commits, 1);
    assert.equal(frames[0]!.commitSequence, 1);
  } finally {
    if (recorder) {
      const stop = recorder.finish();
      env.runFrame(999);
      env.runAllTimeouts();
      await stop.catch(() => {});
    }
    env.restore();
  }
});

test('G4: playing window matches main frame-by-frame in timestamps, durations, attribution, long frames and audited pictures', async () => {
  const { capturePictures, recordPicture } = await import('../dashboard/pulled.ts');
  const env = installPerfEnv();
  const hook = (globalThis as { __REACT_DEVTOOLS_GLOBAL_HOOK__?: { onCommitFiberRoot: (r: number, root: { current: unknown }) => void } }).__REACT_DEVTOOLS_GLOBAL_HOOK__!;
  const root = { current: { child: null, sibling: null, return: null, alternate: null, flags: 0, subtreeFlags: 0, type: null, elementType: null, memoizedProps: null, memoizedState: null, dependencies: null } };
  let recorder: PerfRecorder | null = null;
  try {
    env.setNow(1000);
    recorder = new PerfRecorder();
    const flushed: string[] = [];
    (recorder as unknown as { flush: () => Promise<void> }).flush = async () => {
      const lines = (recorder as unknown as { lines: string[] }).lines;
      flushed.push(...lines.splice(0));
    };
    recorder.reader(1, 0, async () => ({ values: new Map([['A:v', '1']]) }), 0);
    await recorder.beginAudit();
    await recorder.start();

    type FrameShape = { at: number; dt: number; playing: boolean; commits: number; commitSequence: number; work?: Record<string, Record<string, number>>; paint?: Record<string, number>; paint_ms?: number };
    const refFrames: FrameShape[] = [];
    let refLast: number | null = 1000;
    let refPlaying = false;
    let refCommits = 0;
    let refCommitSeq = 0;
    let refWork: Record<string, Record<string, number>> = {};
    const refPaints = new Map<number, Map<string, number>>();
    const refTick = (at: number) => {
      if (refLast !== null) {
        const by = new Map<string, number>();
        for (const [pAt, widgets] of refPaints) {
          if (pAt > refLast) continue;
          refPaints.delete(pAt);
          for (const [w, ms] of widgets) by.set(w, (by.get(w) ?? 0) + ms);
        }
        let paint: Record<string, number> | undefined;
        let paint_ms: number | undefined;
        if (by.size) {
          let total = 0;
          paint = {};
          for (const [w, ms] of by) { total += ms; paint[w] = Math.round(ms * 100) / 100; }
          paint_ms = Math.round(total * 100) / 100;
        }
        refFrames.push({
          at,
          dt: at - refLast,
          playing: refPlaying,
          commits: refCommits,
          commitSequence: refCommitSeq,
          ...(Object.keys(refWork).length ? { work: refWork } : {}),
          ...(paint ? { paint, paint_ms } : {}),
        });
        refCommits = 0;
        refWork = {};
      }
      refLast = at;
    };

    const sample = (at: number) => ({ t: 'picture', at, widget: 'w1', surface: 'table', placement: 1, owner: 0, renderer: 'page', head: at, hour: at, offset: 0, since: at, through: at, eligibleHour: at, entry: at, parts: [], face: '', drawMs: 0, mode: 'picture', dim: false, reset: 0, binding: 0, inspect: null, tooltip: false, rows: [{ row: 'A', key: 'A:v', value: '1' }] }) as never;
    const steps: { t: number; commits: number; widget?: string; paintMs?: number; loafDur?: number; pictureAt?: number }[] = [
      { t: 1016, commits: 1, widget: 'w1', paintMs: 2.5, pictureAt: 1016 },
      { t: 1038, commits: 0, widget: 'w2', paintMs: 4.0, loafDur: 55, pictureAt: 1038 },
      { t: 1052, commits: 2, widget: 'w1', pictureAt: 1052 },
      { t: 1087, commits: 0, paintMs: 1.25, loafDur: 72, pictureAt: 1087 },
      { t: 1103, commits: 1, widget: 'w2', paintMs: 3.0, pictureAt: 1103 },
    ];

    env.setNow(1000);
    recorder.clock(10, true, 24, 1, 1);
    refPlaying = true;
    for (const step of steps) {
      env.setNow(step.t - 2);
      for (let c = 0; c < step.commits; c++) {
        hook.onCommitFiberRoot(1, root);
        refCommits++;
        refCommitSeq++;
      }
      if (step.widget) {
        recorder.count(step.widget, 'render');
        refWork[step.widget] = { ...(refWork[step.widget] ?? {}), render: (refWork[step.widget]?.render ?? 0) + 1 };
      }
      if (step.loafDur) {
        env.emitObserver('long-animation-frame', [{ startTime: step.t - step.loafDur, duration: step.loafDur, blockingDuration: step.loafDur - 50, renderStart: step.t - 5, scripts: [{ duration: 30, invoker: 'tick', sourceURL: 'clock.ts' }] }]);
      }
      if (step.pictureAt) recordPicture(sample(step.pictureAt));
      env.runFrame(step.t);
      refTick(step.t);
      if (step.paintMs) {
        recorder.paint('w1', step.paintMs, step.t);
        let atMap = refPaints.get(step.t);
        if (!atMap) refPaints.set(step.t, atMap = new Map());
        atMap.set('w1', (atMap.get('w1') ?? 0) + step.paintMs);
      }
    }
    env.setNow(1110);
    recorder.clock(15, false, 24, 2, 1);
    refPlaying = false;
    const stopping = recorder.stop().catch(() => {});
    env.runFrame(1119);
    refTick(1119);
    env.runAllTimeouts();
    await stopping;

    const audit = await recorder.auditPicturesBatch(100);
    assert.equal(audit.done, true);
    assert.equal(audit.results.length, 5);
    assert.ok(audit.results.every(r => r.error === null));
    await recorder.flush();
    await capturePictures(false);

    const records = flushed.map(line => JSON.parse(line) as Record<string, unknown>);
    const startIdx = records.findIndex(r => r.t === 'start');
    const stopIdx = records.findIndex(r => r.t === 'stop');
    const windowRecords = records.slice(startIdx + 1, stopIdx);
    const actualFrames: FrameShape[] = windowRecords
      .filter(r => r.t === 'frame')
      .map(f => ({
        at: f.at as number,
        dt: f.dt as number,
        playing: f.playing as boolean,
        commits: f.commits as number,
        commitSequence: f.commitSequence as number,
        ...(f.work ? { work: f.work as Record<string, Record<string, number>> } : {}),
        ...(f.paint ? { paint: f.paint as Record<string, number>, paint_ms: f.paint_ms as number } : {}),
      }));
    assert.deepEqual(actualFrames, refFrames, 'every frame matches main in timestamp, dt, playing, commits, commitSequence, work, and per-frame paint attribution');

    const actualLoafs = windowRecords.filter(r => r.t === 'loaf').map(r => ({ at: r.at, dur: r.dur }));
    assert.deepEqual(actualLoafs, [{ at: 1038 - 55, dur: 55 }, { at: 1087 - 72, dur: 72 }], 'long animation frames are emitted in recorder output');

    const emittedPictures = records.filter(r => r.t === 'picture').map(r => r.at);
    assert.deepEqual(emittedPictures, [1016, 1038, 1052, 1087, 1103], 'audited pictures are emitted through recorder output');
  } finally {
    if (recorder) {
      const stop = recorder.finish();
      env.runFrame(2000);
      env.runAllTimeouts();
      await stop.catch(() => {});
    }
    env.restore();
  }
});
test('unaudited painted pictures fail audit explicitly and reader(null) unregisters future audits without breaking an active audit', async () => {
  const { capturePictures, capturePicturesBatch, recordPicture } = await import('../dashboard/pulled.ts');
  await capturePictures(false);
  await capturePicturesBatch(100);
  const recorder = Object.create(PerfRecorder.prototype) as PerfRecorder;
  Object.assign(recorder as unknown as Record<string, unknown>, {
    readers: new Map(),
    commitSequence: 0,
    rowChanges: [],
    exactReads: new Map(),
    auditGen: 0,
    lines: [],
    buffered: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    lost: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    sending: 0,
    stopped: false,
    push: () => {},
    flush: async () => {},
  });

  recorder.reader(7, -1, async () => ({ values: new Map([['A:v', 'ok']]), ordered: true }));
  await recorder.beginAudit();
  recorder.reader(7, -1, null);
  recordPicture({
    t: 'picture', at: 10, shownAt: 10, seek: 0, widget: 'w-unaudited', surface: 'table', placement: 7, owner: -1,
    renderer: 'page', head: 10, hour: 10, offset: 0, eligibleHour: 10, drawMs: 0, mode: 'picture', dim: false,
    reset: 0, binding: 0, inspect: null, tooltip: false, auditMissing: true,
  });
  recordPicture({
    t: 'picture', at: 11, shownAt: 11, seek: 0, widget: 'w-ok', surface: 'table', placement: 7, owner: -1,
    renderer: 'page', head: 11, hour: 11, offset: 0, eligibleHour: 11, drawMs: 0, mode: 'picture', dim: false,
    reset: 0, binding: 0, inspect: null, tooltip: false, rows: [{ row: 'A', key: 'A:v', value: 'ok' }],
  });
  const first = await recorder.auditPicturesBatch(10);
  assert.equal(first.done, true);
  assert.equal(first.results.length, 2);
  assert.equal(first.results[0]!.error, 'Painted picture emitted without audit rows');
  assert.equal(first.results[1]!.error, null, 'active audit still completes with snapshotted reader after unmount unregister');

  await recorder.beginAudit();
  recordPicture({
    t: 'picture', at: 20, shownAt: 20, seek: 0, widget: 'w-unregistered', surface: 'table', placement: 7, owner: -1,
    renderer: 'page', head: 20, hour: 20, offset: 0, eligibleHour: 20, drawMs: 0, mode: 'picture', dim: false,
    reset: 0, binding: 0, inspect: null, tooltip: false, rows: [{ row: 'A', key: 'A:v', value: 'ok' }],
  });
  const second = await recorder.auditPicturesBatch(10);
  assert.equal(second.results[0]!.error, 'Displayed rows or exact read missing', 'subsequent audit sees unregistered reader removed');
  await capturePictures(false);
  await capturePicturesBatch(100);
});

test('P4: deferUntilAuditDone retains widget audit snapshots across unmount until active audit drains', async () => {
  const { capturePictures, capturePicturesBatch, recordPicture } = await import('../dashboard/pulled.ts');
  const { AuditSnapshotStore } = await import('../families/audit.ts');
  await capturePictures(false);
  await capturePicturesBatch(100);
  const recorder = Object.create(PerfRecorder.prototype) as PerfRecorder;
  Object.assign(recorder as unknown as Record<string, unknown>, {
    readers: new Map(),
    commitSequence: 0,
    rowChanges: [],
    exactReads: new Map(),
    auditGen: 0,
    lines: [],
    buffered: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    lost: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    sending: 0,
    stopped: false,
    push: () => {},
    flush: async () => {},
  });

  const store = new AuditSnapshotStore<{ expected: string }>(16);
  const snapId = store.create({ expected: 'ok-before-remount' });
  const stopReader = recorder.reader(9, -1, async (_hour, auditId) => {
    const snap = store.require(auditId);
    return { values: new Map([['R:v', snap.expected]]), ordered: true };
  }, 1);

  await recorder.beginAudit();
  recordPicture({
    t: 'picture', at: 10, shownAt: 10, seek: 0, widget: 'w-remount', surface: 'table', placement: 9, owner: -1, generation: 1,
    renderer: 'page', head: 10, hour: 10, offset: 0, eligibleHour: 10, drawMs: 0, mode: 'picture', dim: false,
    reset: 0, binding: 0, inspect: null, tooltip: false, auditId: snapId,
    rows: [{ row: 'R', key: 'R:v', value: 'ok-before-remount' }],
  });

  stopReader();
  recorder.dropGeneration(1);
  recorder.deferUntilAuditDone(() => store.clear());
  assert.equal(store.get(snapId)?.expected, 'ok-before-remount', 'snapshot stays alive while audit is still active');

  const drained = await recorder.auditPicturesBatch(10);
  assert.equal(drained.done, true);
  assert.equal(drained.results.length, 1);
  assert.equal(drained.results[0]!.error, null, 'deferred verification succeeds against retained snapshot after unmount');
  assert.equal(store.get(snapId), undefined, 'deferred cleanup releases snapshots once audit drains');
  await capturePictures(false);
  await capturePicturesBatch(100);
});

test('F2: capturePictures waits for surfaces to acknowledge audit-enabled bindings so clock ticks in the gap emit zero unaudited pictures', async () => {
  const { auditing, auditingDesired, capturePictures, capturePicturesBatch, onAuditingChange, recordPicture } = await import('../dashboard/pulled.ts');
  await capturePictures(false);
  await capturePicturesBatch(100);
  let ackSurface!: () => void;
  const off = onAuditingChange(active => {
    if (!active) return;
    return new Promise<void>(resolve => { ackSurface = resolve; });
  });
  try {
    const starting = capturePictures(true);
    assert.equal(auditingDesired(), true, 'surfaces are notified to install audit-enabled bindings');
    assert.equal(auditing(), false, 'sampling has not begun while surface acknowledgement is pending');
    recordPicture({
      t: 'picture', at: 1, shownAt: 1, seek: 0, widget: 'gap-tick', surface: 'table', placement: 1, owner: -1,
      renderer: 'page', head: 10, hour: 10, offset: 0, eligibleHour: 10, drawMs: 0, mode: 'picture', dim: false,
      reset: 0, binding: 1, inspect: null, tooltip: false, auditMissing: true,
    });
    ackSurface();
    await starting;
    assert.equal(auditing(), true, 'sampling starts once every visible surface acknowledges');
    recordPicture({
      t: 'picture', at: 2, shownAt: 2, seek: 0, widget: 'after-ack', surface: 'table', placement: 1, owner: -1,
      renderer: 'page', head: 11, hour: 11, offset: 0, eligibleHour: 11, drawMs: 0, mode: 'picture', dim: false,
      reset: 0, binding: 2, inspect: null, tooltip: false, rows: [{ row: 'A', key: 'A:v', value: 'ok' }],
    });
    const captured = await capturePictures(false);
    assert.deepEqual(captured.map(r => (r.t === 'picture' ? r.widget : r.t)), ['after-ack'], 'gap tick before ack was never recorded as an unaudited failure');
  } finally {
    off();
    await capturePictures(false);
    await capturePicturesBatch(100);
  }
});

test('F3 & P4: captured picture snapshots stay pinned across >512 store creates and across unmount during auditPicturesBatch awaits until verification drains', async () => {
  const { capturePictures, capturePicturesBatch, recordPicture } = await import('../dashboard/pulled.ts');
  const { AuditSnapshotStore } = await import('../families/audit.ts');
  await capturePictures(false);
  await capturePicturesBatch(100);
  const recorder = Object.create(PerfRecorder.prototype) as PerfRecorder;
  Object.assign(recorder as unknown as Record<string, unknown>, {
    readers: new Map(),
    commitSequence: 0,
    rowChanges: [],
    exactReads: new Map(),
    auditGen: 0,
    lines: [],
    buffered: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    lost: { count: 0, bytes: 0, from: Infinity, to: -Infinity },
    sending: 0,
    stopped: false,
    push: () => {},
    flush: async () => {},
  });

  const store = new AuditSnapshotStore<{ expected: string }>(8);
  const pinnedId = store.create({ expected: 'pinned-row' });
  const stopReader = recorder.reader(5, -1, async (_hour, auditId) => {
    const snap = store.require(auditId);
    return { values: new Map([['R:v', snap.expected]]), ordered: true };
  }, 1, auditId => store.pin(auditId));

  await recorder.beginAudit();
  let releaseCommitAwait!: () => void;
  const commitGate = new Promise<void>(resolve => { releaseCommitAwait = resolve; });
  recorder.structure(
    'w-commit',
    'table',
    { hour: 1, rows: () => ['a'], exact: async () => { await commitGate; return ['a']; } },
    { hour: 2, rows: () => ['b'], exact: async () => ['b'] },
  );
  (recorder as unknown as { commitAudit: unknown[] }).commitAudit.push({
    at: 1, serial: 1, widgets: ['w-commit'], causes: {},
    changes: (recorder as unknown as { rowChanges: unknown[] }).rowChanges,
  });

  recorder.pinPicture(1, 5, -1, pinnedId);
  recordPicture({
    t: 'picture', at: 10, shownAt: 10, seek: 0, widget: 'w-pinned', surface: 'table', placement: 5, owner: -1, generation: 1,
    renderer: 'page', head: 10, hour: 10, offset: 0, eligibleHour: 10, drawMs: 0, mode: 'picture', dim: false,
    reset: 0, binding: 0, inspect: null, tooltip: false, auditId: pinnedId,
    rows: [{ row: 'R', key: 'R:v', value: 'pinned-row' }],
  });

  for (let i = 0; i < 600; i++) store.create({ expected: `scroll-${i}` });
  assert.equal(store.get(pinnedId)?.expected, 'pinned-row', 'F3: pinned snapshot survives >512 scroll/resize creates');
  assert.ok(store.size <= 9, `F3: unpinned snapshots still evict to maxEntries + pinned (got ${store.size})`);

  const batchPromise = recorder.auditPicturesBatch(10);
  await Promise.resolve();
  stopReader();
  store.clear();
  assert.equal(store.get(pinnedId)?.expected, 'pinned-row', 'P4: unmount during auditPicturesBatch awaits keeps pinned snapshot alive');
  releaseCommitAwait();

  const drained = await batchPromise;
  assert.equal(drained.done, true);
  assert.equal(drained.results.length, 1);
  assert.equal(drained.results[0]!.error, null, 'F3 & P4: audit verifies pinned snapshot with zero error');
  assert.equal(store.get(pinnedId), undefined, 'F3 & P4: pinned stale snapshot is freed once verification drains');
  await capturePictures(false);
  await capturePicturesBatch(100);
});
