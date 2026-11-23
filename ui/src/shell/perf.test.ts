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

test('renderer preload status reports incomplete before preload runs', async () => {
  const { preloadStatus } = await import('../widgets/rendererLoaders.ts');
  const status = preloadStatus();
  assert.equal(status.started, false);
  assert.equal(status.done, false);
});

test('PerfRecorder schedules no animation frames or flush timers while paused and idle, and wakes on playback or activity', async () => {
  const saved = new Map(['requestAnimationFrame', 'cancelAnimationFrame', 'PerformanceObserver', 'devicePixelRatio', 'innerWidth', 'innerHeight', 'setInterval', 'clearInterval', 'setTimeout', 'clearTimeout', 'MessageChannel']
    .map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const rafs = new Map<number, FrameRequestCallback>();
  let nextRaf = 0, rafCount = 0;
  let intervalCalls = 0;
  const timeouts = new Map<number, () => void>();
  let nextTimeout = 0;
  const ports: { onmessage: ((event: { data: number }) => void) | null }[] = [];
  class FakeChannel {
    port1 = { onmessage: null as ((event: { data: number }) => void) | null };
    port2 = { postMessage: (data: number) => { queueMicrotask(() => this.port1.onmessage?.({ data })); } };
    constructor() { ports.push(this.port1); }
  }
  class FakeObserver {
    static supportedEntryTypes: string[] = [];
    observe() {}
    disconnect() {}
  }
  const runNextRaf = (now: number) => {
    const first = rafs.entries().next().value as [number, FrameRequestCallback] | undefined;
    if (!first) return false;
    rafs.delete(first[0]);
    first[1](now);
    return true;
  };
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
  });
  let recorder: PerfRecorder | null = null;
  try {
    recorder = new PerfRecorder();
    const lines = (recorder as unknown as { lines: string[] }).lines;
    assert.equal(intervalCalls, 0, 'does not run a permanent polling interval while idle');
    assert.equal(rafs.size, 0, 'starts idle with no animation frame scheduled while paused');
    const initialCount = rafCount;

    recorder.clock(100, false, 24, 1, 1);
    assert.equal(rafs.size, 1, 'wakes on a paused clock update');
    runNextRaf(10);
    await Promise.resolve();
    runNextRaf(26);
    await Promise.resolve();
    assert.equal(rafs.size, 0, 'returns to sleep after recording the paused update');
    const afterPaused = rafCount;
    assert.ok(afterPaused > initialCount);

    recorder.count('w1', 'render');
    assert.equal(rafs.size, 1, 'wakes on widget work while paused');
    runNextRaf(100);
    await Promise.resolve();
    runNextRaf(116);
    await Promise.resolve();
    assert.equal(rafs.size, 0, 'sleeps again once the work frame is recorded');
    const recorded = lines.map(line => JSON.parse(line) as { t: string; playing?: boolean; work?: Record<string, unknown> });
    assert.ok(recorded.some(r => r.t === 'frame' && r.work?.w1), 'recorded the paused work frame');

    recorder.clock(101, true, 24, 2, 1);
    assert.equal(rafs.size, 1, 'starts continuous frames while playing');
    runNextRaf(200);
    await Promise.resolve();
    assert.equal(rafs.size, 1, 'keeps scheduling frames while playing');
    runNextRaf(216);
    await Promise.resolve();
    assert.equal(rafs.size, 1, 'still scheduling frames while playing');

    recorder.clock(102, false, 24, 3, 1);
    runNextRaf(232);
    await Promise.resolve();
    assert.equal(rafs.size, 0, 'stops scheduling frames once playback pauses and the final frame is recorded');
  } finally {
    if (recorder) {
      const stop = recorder.finish();
      for (let i = 0; i < 4 && runNextRaf(999); i++) {}
      for (const fn of [...timeouts.values()]) fn();
      await stop.catch(() => {});
    }
    for (const [key, desc] of saved) {
      if (desc) Object.defineProperty(globalThis, key, desc);
      else Reflect.deleteProperty(globalThis, key);
    }
  }
});
