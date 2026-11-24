import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChartBucket } from '../charts/chartSummary';
import type { MarkCluster } from '../charts/markSummary';
import type { Run } from '../charts/series';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import { Pipeline, type LaneWorker, type PipelineHost } from '../dashboard/follow/pipeline.ts';
import { PlaybackController } from '../dashboard/playback.ts';
import type { TiledLine } from '../dashboard/tiles/view';
import type { MissingSpan } from '../generated/dashboard';
import {
  BoundedSeriesCursor,
  boundTiledSeries,
  createBoundedReadView,
  cutStraddlingBucket,
  displayedHour,
  FutureDataError,
  type BoundedWorkCounter,
} from './boundedView.ts';
import {
  assertValidCoverage,
  compileFollowPlan,
  CoverageContractError,
  FamilySourceBroker,
  planQueryKey,
  planSignature,
} from './broker.ts';
import type {
  ArtHandle,
  ArtProvider,
  DisplayedHour,
  EntityKind,
  IdentityProvider,
  SourcePlan,
} from './plan.ts';
import type { DatedLogRow, TableRowSource } from './table/rowSource.ts';

function fakeSource(id = 'src-1', revision = 'rev-1'): DashboardSource {
  return {
    id,
    revision,
    name: 'Test',
    firstHour: 0,
    lastHour: 87_600,
    branches: [{ id: '1', firstHour: 0, lastHour: 87_600 }],
  } as unknown as DashboardSource;
}

function makeBucket(firstHour: number, lastHour: number, firstValue: number, lastValue: number, minValue: number, maxValue: number): ChartBucket {
  return {
    firstHour,
    lastHour,
    firstValue,
    lastValue,
    lastValueHour: lastHour,
    minHour: firstHour,
    minValue,
    maxHour: lastHour,
    maxValue,
    firstBreak: false,
    hasBreak: false,
  };
}

function makeTenYearLine(): { readonly line: TiledLine; readonly runs: readonly Run[] } {
  const binHours = 24;
  const totalHours = 87_600;
  const bucketCount = totalHours / binHours;
  const buckets: ChartBucket[] = [];
  const runs: Run[] = [];
  for (let i = 0; i < bucketCount; i++) {
    const start = i * binHours;
    const end = start + binHours - 1;
    const v = 100 + (i % 50);
    buckets.push(makeBucket(start, end, v, v + 1, v, v + 10));
    runs.push({ firstHour: start, lastHour: start + 11, value: v });
    runs.push({ firstHour: start + 12, lastHour: end, value: v + 1 });
  }
  const missing: MissingSpan[] = [];
  for (let i = 0; i < 2000; i++) {
    const first = i * 40 + 5;
    missing.push({ firstHour: first, lastHour: first + 2, reason: 'gap' });
  }
  const marks: MarkCluster[] = [];
  for (let i = 0; i < 4000; i++) {
    const first = i * 20 + 2;
    marks.push({
      firstHour: first,
      lastHour: first + 4,
      firstOrdinal: String(i),
      count: '2',
      bins: [[first, 1], [first + 4, 1]],
    });
  }
  const line: TiledLine = {
    tag: 'GER',
    head: 'GER',
    decimals: 0,
    summary: {
      from: 0,
      to: totalHours - 1,
      width: 1000,
      buckets,
      tiles: { binHours, valueAt: () => null },
    },
    missing,
    parts: [],
    marks: { from: 0, to: totalHours - 1, width: 1000, clusters: marks },
  };
  return { line, runs };
}

test('BoundedSeriesCursor advances over a 10-year history in O(what changed) without copying arrays or growing with history, and quiet ticks inside validInterval do zero work', () => {
  const { line, runs } = makeTenYearLine();
  const cursor = new BoundedSeriesCursor();

  const initialCounter: BoundedWorkCounter = { visits: 0, allocations: 0 };
  const initial = cursor.advance(line, 4_800, { prefixRuns: runs, counter: initialCounter });
  assert.equal(initial.buckets.source, line.summary.buckets);
  assert.equal(initial.missing.source, line.missing);
  assert.equal(initial.marks.source, line.marks!.clusters);

  let earlyMaxVisits = 0;
  let earlyMaxAllocs = 0;
  for (let h = 4_801; h <= 5_800; h++) {
    const c: BoundedWorkCounter = { visits: 0, allocations: 0 };
    cursor.advance(line, h, { prefixRuns: runs, counter: c });
    if (c.visits > earlyMaxVisits) earlyMaxVisits = c.visits;
    if (c.allocations > earlyMaxAllocs) earlyMaxAllocs = c.allocations;
  }

  let lateMaxVisits = 0;
  let lateMaxAllocs = 0;
  for (let h = 84_000; h <= 85_000; h++) {
    const c: BoundedWorkCounter = { visits: 0, allocations: 0 };
    cursor.advance(line, h, { prefixRuns: runs, counter: c });
    if (c.visits > lateMaxVisits) lateMaxVisits = c.visits;
    if (c.allocations > lateMaxAllocs) lateMaxAllocs = c.allocations;
  }

  assert.ok(earlyMaxVisits <= 64, `early step visits bounded by O(log N): ${earlyMaxVisits}`);
  assert.ok(lateMaxVisits <= 64, `late step visits do not grow with history: ${lateMaxVisits}`);
  assert.ok(lateMaxVisits <= earlyMaxVisits + 8, `late visits (${lateMaxVisits}) stay flat vs early (${earlyMaxVisits})`);
  assert.ok(earlyMaxAllocs <= 8 && lateMaxAllocs <= 8, `per-step allocations stay O(1): early=${earlyMaxAllocs}, late=${lateMaxAllocs}`);

  const quietLine: TiledLine = {
    tag: 'GER',
    head: 'GER',
    decimals: 0,
    summary: {
      from: 0,
      to: 400,
      width: 400,
      buckets: [makeBucket(0, 23, 10, 10, 10, 10)],
      tiles: { binHours: 24, valueAt: () => null },
    },
    missing: [{ firstHour: 200, lastHour: 220, reason: 'gap' }],
    parts: [],
    marks: { from: 0, to: 400, width: 400, clusters: [{ firstHour: 300, lastHour: 300, firstOrdinal: '0', count: '1', bins: [[300, 1]] }] },
  };
  const quietCursor = new BoundedSeriesCursor();
  const at50 = quietCursor.advance(quietLine, 50);
  assert.ok(at50.validInterval[0] <= 50 && at50.validInterval[1] >= 150, `validInterval spans quiet window: [${at50.validInterval}]`);

  const quietCounter: BoundedWorkCounter = { visits: 0, allocations: 0 };
  for (let h = 51; h <= 150; h++) {
    const next = quietCursor.advance(quietLine, h, { counter: quietCounter });
    assert.equal(next, at50, `quiet advance at hour ${h} reuses exact BoundedSeries reference`);
  }
  assert.equal(quietCounter.visits, 0, 'quiet advance inside validInterval does zero visits');
  assert.equal(quietCounter.allocations, 0, 'quiet advance inside validInterval does zero allocations');
});

test('straddling bucket is cut from witnessed prefix runs/points without leaking future peaks, and withheld as loading when no prefix exists', () => {
  const straddlingBucket = makeBucket(100, 123, 10, 500, 5, 500);
  const prefixRuns: Run[] = [
    { firstHour: 100, lastHour: 103, value: 10 },
    { firstHour: 104, lastHour: 110, value: 20 },
    { firstHour: 111, lastHour: 123, value: 500 },
  ];

  const cut = cutStraddlingBucket(straddlingBucket, displayedHour(105), prefixRuns);
  assert.equal(cut.straddling, 'cut');
  assert.ok(cut.bucket);
  assert.equal(cut.bucket.firstHour, 100);
  assert.equal(cut.bucket.lastHour, 105);
  assert.equal(cut.bucket.minValue, 10);
  assert.equal(cut.bucket.maxValue, 20);
  assert.equal(cut.bucket.lastValue, 20);

  const withoutPrefix = cutStraddlingBucket(straddlingBucket, displayedHour(105));
  assert.equal(withoutPrefix.straddling, 'loading');
  assert.equal(withoutPrefix.bucket, null);

  const line: TiledLine = {
    tag: 'GER',
    head: 'GER',
    decimals: 0,
    summary: {
      from: 76,
      to: 123,
      width: 200,
      buckets: [makeBucket(76, 99, 8, 9, 8, 9), straddlingBucket],
      tiles: { binHours: 24, valueAt: () => null },
    },
    missing: [],
    parts: [],
    marks: null,
  };
  const loadingSeries = boundTiledSeries(line, displayedHour(105));
  assert.equal(loadingSeries.straddling, 'loading');
  assert.equal(loadingSeries.buckets.length, 1);
  assert.equal(loadingSeries.buckets.at(0)?.maxValue, 9);

  const cutSeries = boundTiledSeries(line, displayedHour(105), { prefixRuns });
  assert.equal(cutSeries.straddling, 'cut');
  assert.equal(cutSeries.buckets.length, 2);
  assert.equal(cutSeries.buckets.at(1)?.maxValue, 20);
});

test('BoundedReadView rejects any read after displayed hour H across panels, hovers, rows, and who(), and registers pending who()/art() demands', () => {
  const rowSource: TableRowSource<DatedLogRow> = {
    identity: 'log:test',
    total: 2,
    liveCount: 2,
    at: 100,
    missing: [{ firstHour: 50, lastHour: 150, reason: 'unobserved' }],
    refused: null,
    frozen: null,
    sliceAt: (_h, _f, _c) => [],
  };
  const futureRowSource: TableRowSource<DatedLogRow> = {
    ...rowSource,
    at: 101,
  };

  const identityDemands: [EntityKind, string, DisplayedHour][] = [];
  const artDemands: ArtHandle[] = [];

  const identityProvider: IdentityProvider = {
    resolve(kind, key, atHour) {
      if (key === 'GER') {
        return {
          kind,
          key,
          atHour,
          holder: { status: 'ready', value: 'GER' },
          tag: { status: 'ready', value: 'GER' },
          name: { status: 'ready', value: atHour < 50 ? 'Weimar' : 'Germany' },
          colour: { status: 'ready', value: '#445566' },
          flagChoice: { status: 'ready', value: 'GER_democratic' },
          validInterval: [0, 200],
          nextChangeAt: Infinity,
        };
      }
      return {
        kind,
        key,
        atHour,
        holder: { status: 'pending' },
        name: { status: 'pending' },
        validInterval: [atHour, atHour],
        nextChangeAt: Infinity,
      };
    },
  };

  const artProvider: ArtProvider = {
    resolve(handle) {
      if (handle.choice === 'ready-flag') {
        return {
          status: 'ready',
          assetId: 'flag-1',
          url: 'art://flag-1',
          origin: { kind: 'baseGame' },
          file: 'gfx/flags/GER.tga',
          frame: { frameIndex: 0, frameCount: 1, frameWidth: 41, frameHeight: 26, textureWidth: 41, textureHeight: 26, outputWidth: 41, outputHeight: 26 },
          provenance: { fromCurrentInstall: true, label: 'Current installation', scopeKey: 'current' },
        };
      }
      return { status: 'pending' };
    },
  };

  const view = createBoundedReadView({
    hour: 100,
    panels: new Map<string, unknown>([
      ['ok', { hour: 100, value: 1 }],
      ['future', { hour: 101, value: 2 }],
    ]),
    hovers: new Map([
      ['ok', { hour: 80, data: { x: 1 } }],
      ['future', { hour: 105, data: { x: 2 } }],
    ]),
    rows: new Map([
      ['log', rowSource],
      ['futureLog', futureRowSource],
    ]),
    identity: identityProvider,
    onIdentityDemand: (kind, key, atHour) => { identityDemands.push([kind, key, atHour]); },
    art: artProvider,
    onArtDemand: handle => { artDemands.push(handle); },
  });

  assert.deepEqual(view.panel('ok'), { hour: 100, value: 1 });
  assert.throws(() => view.panel('future'), FutureDataError);

  assert.deepEqual(view.hover('ok', 80), { hour: 80, data: { x: 1 } });
  assert.throws(() => view.hover('ok', 101), FutureDataError);
  assert.throws(() => view.hover('future'), FutureDataError);

  const boundedLog = view.rows('log')!;
  assert.equal(boundedLog.missing[0]!.lastHour, 100);
  assert.throws(() => boundedLog.sliceAt(101, 0, 10), FutureDataError);
  assert.throws(() => view.rows('futureLog'), FutureDataError);

  const gerAt20 = view.who('country', 'GER', 20);
  assert.deepEqual(gerAt20.name, { status: 'ready', value: 'Weimar' });
  assert.equal(gerAt20.atHour, 20);
  assert.equal(identityDemands.length, 0);

  const gerAtH = view.who('country', 'GER');
  assert.deepEqual(gerAtH.name, { status: 'ready', value: 'Germany' });
  assert.equal(gerAtH.atHour, 100);

  const sovPending = view.who('country', 'SOV', 90);
  assert.equal(sovPending.name.status, 'pending');
  assert.deepEqual(identityDemands, [['country', 'SOV', 90]]);

  assert.throws(() => view.who('country', 'GER', 101), FutureDataError);

  const readyArt = view.art({ scope: 'flag', choice: 'ready-flag' });
  assert.equal(readyArt.status, 'ready');
  assert.equal(artDemands.length, 0);

  const pendingHandle: ArtHandle = { scope: 'flag', choice: 'pending-flag' };
  const pendingArt = view.art(pendingHandle);
  assert.equal(pendingArt.status, 'pending');
  assert.deepEqual(artDemands, [pendingHandle]);
});

test('coverage contract rejects row coverage on population forms and top/least on keyed forms', () => {
  const src = fakeSource();
  const allow = (q: Readonly<Record<string, unknown>> | null) => ({ query: q as never, refused: null });

  assert.throws(
    () => assertValidCoverage('bad-pop', { mode: 'population', form: 'top', limit: 10 }, { kind: 'country-measure', coverage: 'row' }),
    CoverageContractError,
  );
  assert.throws(
    () => assertValidCoverage('bad-keyed', { mode: 'keyed', keys: ['GER'] }, { kind: 'country-measure', params: { top: 5 } }),
    CoverageContractError,
  );

  const badPlan: SourcePlan = {
    owner: 'test:1',
    entries: [{
      name: 'main',
      source: src,
      branch: '1',
      offset: 0,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage: { mode: 'population', form: 'all' },
        product: 'scoreboard',
        followQuery: { kind: 'country-measure', coverage: 'row', params: {} },
        params: {},
      },
    }],
  };
  assert.throws(() => compileFollowPlan(badPlan, allow), CoverageContractError);

  const okKeyedPlan: SourcePlan = {
    owner: 'test:2',
    entries: [{
      name: 'main',
      source: src,
      branch: '1',
      offset: 0,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage: { mode: 'keyed', keys: ['GER'] },
        product: 'scoreboard',
        followQuery: { kind: 'country-measure', coverage: 'row', params: { countries: ['GER'] } },
        params: { countries: ['GER'] },
      },
    }],
  };
  assert.equal(compileFollowPlan(okKeyedPlan, allow).subscriptions.length, 1);
});

test('planSignature compares plans without invoking closures, and FamilySourceBroker joins identical produced requests and drops stale slot replies', async () => {
  const src = fakeSource();
  let exactCalls = 0;
  const makePlan = (fnTag: number): SourcePlan => ({
    owner: 'sb:1',
    entries: [{
      name: 'main',
      source: src,
      branch: '1',
      offset: 0,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage: { mode: 'keyed', keys: ['GER'] },
        product: 'scoreboard',
        followQuery: { kind: 'country-measure', params: { b: 2, a: 1 } },
        params: fnTag === 1 ? { a: 1, b: 2 } : { b: 2, a: 1 },
        buildExactParams: () => { exactCalls++; return { a: 1 }; },
        validate: x => x,
      },
    }],
  });

  assert.equal(planSignature(makePlan(1)), planSignature(makePlan(2)));
  assert.equal(exactCalls, 0, 'planSignature never invokes buildExactParams');

  const key1 = planQueryKey(src, '1', 'scoreboard', { b: 2, a: 1 });
  const key2 = planQueryKey(src, '1', 'scoreboard', { a: 1, b: 2 });
  assert.equal(key1, key2);

  const broker = new FamilySourceBroker();
  let loads = 0;
  let aborted = 0;
  let resolveLoad: ((v: string) => void) | null = null;
  const loader = (signal: AbortSignal) => {
    loads++;
    signal.addEventListener('abort', () => { aborted++; });
    return new Promise<string>(resolve => { resolveLoad = resolve; });
  };

  const reqA = broker.requestProduced(src, '1', 'scoreboard', { a: 1, b: 2 }, loader);
  const reqB = broker.requestProduced(src, '1', 'scoreboard', { b: 2, a: 1 }, loader);
  assert.equal(loads, 1, 'two widgets requesting identical produced params share one in-flight load');
  assert.equal(broker.activeRequestCount, 1);

  reqA.release();
  assert.equal(aborted, 0, 'releasing one of two holders keeps shared request alive');
  resolveLoad!('payload');
  assert.equal(await reqB.promise, 'payload');
  assert.equal(broker.activeRequestCount, 0);

  let r1: ((v: string) => void) | null = null;
  let r2: ((v: string) => void) | null = null;
  const delivered: string[] = [];
  let doneResolve: (() => void) | null = null;
  const done = new Promise<void>(res => { doneResolve = res; });
  broker.runSlot('w1', 'k1', () => new Promise<string>(res => { r1 = res; }), v => delivered.push(v));
  broker.runSlot('w1', 'k2', () => new Promise<string>(res => { r2 = res; }), v => { delivered.push(v); doneResolve!(); });
  r1!('stale');
  r2!('fresh');
  await done;
  assert.deepEqual(delivered, ['fresh']);

  let abortOnDispose = false;
  broker.request('k3', signal => new Promise(() => { signal.addEventListener('abort', () => { abortOnDispose = true; }); }));
  broker.dispose();
  assert.equal(abortOnDispose, true);
  assert.equal(broker.activeRequestCount, 0);
});

test('mounting 20 same-kind widgets with per-instance keys balances evenly across 2 lanes', () => {
  const clock = new PlaybackController();
  const fakeWorker = (): LaneWorker => ({ onmessage: null, postMessage() {} });
  const host: PipelineHost = {
    lanes: [fakeWorker(), fakeWorker()],
    channel: () => ({ port1: { close() {} } as MessagePort, port2: { close() {} } as MessagePort }),
    open: () => Promise.resolve({ session: 's1' }),
    rate: () => 24,
    frameRate: () => 60,
  };
  const pipeline = new Pipeline(clock, {} as DashboardClient, host);
  try {
    const counts = [0, 0];
    for (let i = 1; i <= 20; i++) {
      const id = pipeline.mount({
        offset: 0,
        pin: null,
        kind: 'scoreboard:scoreboard',
        key: `widget-${i}|scoreboard|${i}`,
      });
      counts[pipeline.lane(id)]!++;
    }
    assert.deepEqual(counts, [10, 10]);
  } finally {
    pipeline.close();
  }
});
