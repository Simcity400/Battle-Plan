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
  recorder.reader(1, 0, async () => {
    await staleGate;
    return { values: new Map([['A:v', 'old-expected']]) };
  });

  await recorder.beginAudit();
  const sample = (widget: string, at: number, value: string) => ({ t: 'picture', at, widget, surface: 'table', placement: 1, owner: 0, renderer: 'page', head: at, hour: at, offset: 0, since: at, through: at, eligibleHour: at, entry: at, parts: [], face: '', drawMs: 0, mode: 'picture', dim: false, reset: 0, binding: 0, inspect: null, tooltip: false, rows: [{ row: 'A', key: 'A:v', value }] }) as never;
  recordPicture(sample('old-widget', 1, 'old-expected'));
  recordPicture(sample('old-widget', 2, 'old-expected'));
  const inFlightOld = recorder.auditPicturesBatch(1);

  recorder.reader(1, 0, async () => ({ values: new Map([['A:v', 'new-expected']]) }));
  await recorder.beginAudit();
  recordPicture(sample('new-widget', 10, 'new-expected'));

  releaseStale();
  const staleOutcome = await inFlightOld;
  assert.deepEqual(staleOutcome, { done: true, results: [] }, 'stale continuation returns empty and does not touch new audit');

  const next = await recorder.auditPicturesBatch(10);
  assert.equal(next.done, true);
  assert.deepEqual(next.results.map(r => [r.widget, r.error]), [['new-widget', null]]);
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
