import assert from 'node:assert/strict';
import test from 'node:test';
import type { ChartBucket } from '../charts/chartSummary';
import type { MarkCluster } from '../charts/markSummary';
import type { Run } from '../charts/series';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import { Page, type Bitmap, type Entry, type PageSurface } from '../dashboard/follow/page.ts';
import { Pipeline, type LaneWorker, type PipelineHost } from '../dashboard/follow/pipeline.ts';
import { lastRecordedBranchHour, recordingEndTime, resolveWindow, type BoundTrack, type WidgetSpec } from '../dashboard/model.ts';
import { PlaybackController } from '../dashboard/playback.ts';
import { axisAt, lastRecordedHour, resolveRegion, SPEC } from '../dashboard/regionTime.ts';
import type { TiledLine } from '../dashboard/tiles/view';
import { Lane, type LaneView } from '../draw/lane.ts';
import type { FromLane } from '../draw/laneMessages';
import type { MissingSpan } from '../generated/dashboard';
import { AuditSnapshotStore } from './audit.ts';
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
import { createBoundAuditReader, createPlanAuditVerifier, type BoundAuditEntry, type PlanAuditSnapshot } from './host/auditBinding.ts';
import { clientPointToInstalled, resolveInstalledAction } from './host/pointer.ts';
import type {
  ArtHandle,
  ArtProvider,
  DisplayedHour,
  EntityKind,
  IdentityProvider,
  InstalledPicture,
  SourcePlan,
} from './plan.ts';
import type { DatedLogRow, TableRowSource } from './table/rowSource.ts';
import { knownValue } from './value.ts';

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

  const approxLine: TiledLine = {
    ...line,
    summary: {
      ...line.summary,
      approximate: [
        { firstHour: 80, lastHour: 90 },
        { firstHour: 100, lastHour: 120 },
        { firstHour: 121, lastHour: 140 },
      ],
    },
  };
  const approxBounded = boundTiledSeries(approxLine, displayedHour(105), { prefixRuns });
  assert.equal(approxBounded.approximate?.length, 2);
  assert.deepEqual(approxBounded.approximate?.at(0), { firstHour: 80, lastHour: 90 });
  assert.deepEqual(approxBounded.approximate?.at(1), { firstHour: 100, lastHour: 105 });
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
        return { status: 'ready', assetId: 'flag-1', origin: 'vanilla' };
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

interface FakeBmp extends Bitmap {
  readonly id: number;
  closed: boolean;
}

function makeBmp(id: number): FakeBmp {
  return { id, width: 100, closed: false, close() { this.closed = true; } };
}

function makeSurface(): PageSurface<FakeBmp> & { installed: FakeBmp | null; clears: number } {
  return {
    installed: null,
    clears: 0,
    install(bmp) { this.installed = bmp; },
    clear() { this.installed = null; this.clears++; },
  };
}

function makeEntry(overrides: Partial<Entry<FakeBmp>> & { widget: number; id: number }): Entry<FakeBmp> {
  return {
    binding: 1,
    h: 100,
    c: 120,
    lo: 100,
    hi: 120,
    gen: { seek: 0, reset: 0 },
    follows: true,
    mode: 'picture',
    surfaces: [makeBmp(overrides.id)],
    content: { 0: `s:${overrides.id}`, hour: String(overrides.h ?? 100) },
    fields: {},
    ...overrides,
  };
}

test('seeking through a gap and returning to the same hour with a new owner clears installed state and rejects late entries from the old owner', () => {
  const page = new Page<FakeBmp>({ lanes: 1 });
  const surface = makeSurface();
  page.mount(1, { route: 1, lane: 0, owner: 'country:GER', surfaces: [surface] });
  page.publish(100, { playing: false, seek: 1 });

  page.offer({
    lane: 0,
    serial: 1,
    entries: [makeEntry({ widget: 1, id: 1, binding: 1, owner: 'country:GER', h: 100, c: 120, lo: 100, hi: 120, gen: { seek: 1, reset: 0 } })],
  });
  page.frame();
  assert.equal(page.view(1).mode, 'picture');
  assert.equal(page.view(1).installedOwner, 'country:GER');

  page.publish(150, { playing: false, seek: 2 });
  assert.equal(page.view(1).mode, 'reading');

  page.publish(100, { playing: false, seek: 3 });
  page.rebind(1, { owner: 'country:SOV' });
  assert.equal(page.view(1).mode, 'reading');
  assert.equal(page.view(1).owner, 'country:SOV');
  assert.equal(page.view(1).installedOwner, undefined);

  page.offer({
    lane: 0,
    serial: 2,
    entries: [makeEntry({ widget: 1, id: 2, binding: 2, owner: 'country:GER', h: 100, c: 120, lo: 100, hi: 120, gen: { seek: 3, reset: 0 } })],
  });
  page.frame();
  assert.equal(page.view(1).mode, 'reading', 'late entry from old owner GER is rejected');
  assert.equal(page.stats.rejected.owner, 1);

  page.offer({
    lane: 0,
    serial: 3,
    entries: [makeEntry({ widget: 1, id: 3, binding: 2, owner: 'country:SOV', h: 100, c: 120, lo: 100, hi: 120, gen: { seek: 3, reset: 0 } })],
  });
  page.frame();
  assert.equal(page.view(1).mode, 'picture');
  assert.equal(page.view(1).installedOwner, 'country:SOV');
});

test('two tracks with different axisOffsets and pinned vs following widgets coexist cleanly on Page', () => {
  const page = new Page<FakeBmp>({ lanes: 1 });
  const sFollowA = makeSurface();
  const sFollowB = makeSurface();
  const sPinned = makeSurface();
  page.mount(1, { route: 1, lane: 0, owner: 'trackA', offset: 0, pin: null, bounds: [0, 500], surfaces: [sFollowA] });
  page.mount(2, { route: 2, lane: 0, owner: 'trackB', offset: 1000, pin: null, bounds: [1000, 1500], surfaces: [sFollowB] });
  page.mount(3, { route: 1, lane: 0, owner: 'pinned', offset: 0, pin: 50, surfaces: [sPinned] });
  page.publish(50, { playing: false, seek: 1 });

  page.offer({
    lane: 0,
    serial: 1,
    entries: [
      makeEntry({ widget: 1, id: 1, owner: 'trackA', h: 50, c: 60, lo: 50, hi: 60, gen: { seek: 1, reset: 0 } }),
      makeEntry({ widget: 2, id: 2, owner: 'trackB', h: 1050, c: 1060, lo: 1050, hi: 1060, gen: { seek: 1, reset: 0 } }),
      makeEntry({ widget: 3, id: 3, owner: 'pinned', h: 50, c: 50, lo: 50, hi: 50, gen: { seek: 1, reset: 0 }, follows: false }),
    ],
  });
  page.frame();
  assert.equal(page.view(1).through, 50);
  assert.equal(page.view(2).through, 1050);
  assert.equal(page.view(3).through, 50);

  page.publish(200, { playing: false, seek: 2 });
  assert.equal(page.view(1).mode, 'reading', 'following track A clears past its installed interval');
  assert.equal(page.view(2).mode, 'reading', 'following track B clears past its installed interval');
  assert.equal(page.view(3).mode, 'picture', 'pinned widget keeps its picture across seeks');
  assert.equal(page.view(3).through, 50);

  page.publish(600, { playing: false, seek: 3 });
  assert.equal(page.view(1).mode, 'outside', 'track A outside [0, 500]');
  assert.equal(page.view(2).mode, 'outside', 'track B (1600) outside [1000, 1500]');
  assert.equal(page.view(3).mode, 'picture', 'pinned widget stays painted regardless of playback bounds');
});

test('held reply during a resize maps stationary client clicks through installed viewport geometry, and clicks after an owner change are rejected', () => {
  const installedViewport = { width: 400, height: 200, dpr: 1, scrollLeft: 0, scrollTop: 0 };
  const resizedRect = { left: 100, top: 50, width: 800, height: 400 };
  const mapped = clientPointToInstalled(200, 150, resizedRect, installedViewport);
  assert.deepEqual(mapped, [50, 50], 'client (200, 150) inside 2x resized box maps back to (50, 50) in 400x200 installed picture');

  const hitRow = (point: readonly [number, number]): string | null => {
    const idx = Math.floor(point[1] / 40);
    return idx === 0 ? 'row:GER' : idx === 1 ? 'row:SOV' : null;
  };
  assert.equal(hitRow(mapped), 'row:SOV', 'stationary click hits visual row SOV under cursor during held resize');

  const installedPic: InstalledPicture = {
    id: 7,
    owner: 'country:GER',
    generation: { seek: 1, reset: 0, binding: 1 },
    factRevisions: {},
    validInterval: [100, 120],
    through: 100,
    hour: displayedHour(100),
    viewport: installedViewport,
    optionsRevision: 'opt-1',
    themeRevision: 'theme-1',
    actionKeys: new Map([['SOV', { subject: 'SOV', hour: displayedHour(100), label: 'Soviet Union', meta: { slot: 2 } }]]),
    accessibleDescription: null,
  };
  const action = resolveInstalledAction(installedPic, 'SOV', false);
  assert.deepEqual(action, {
    installedId: 7,
    owner: 'country:GER',
    subject: 'SOV',
    hour: 100,
    label: 'Soviet Union',
    meta: { slot: 2 },
  });

  const clock = new PlaybackController();
  const worker: LaneWorker = { onmessage: null, postMessage() {} };
  const host: PipelineHost = {
    lanes: [worker],
    channel: () => ({ port1: { close() {} } as MessagePort, port2: { close() {} } as MessagePort }),
    open: () => Promise.resolve({ session: 's1' }),
    rate: () => 24,
    frameRate: () => 60,
  };
  const pipeline = new Pipeline(clock, {} as DashboardClient, host);
  try {
    const wid = pipeline.mount({ offset: 0, pin: null, owner: 'country:GER', key: 'w1' });
    pipeline.page.offer({
      lane: 0,
      serial: 1,
      entries: [makeEntry({ widget: wid, id: 1, owner: 'country:GER', h: 0, c: 100, lo: 0, hi: 100, surfaces: [] }) as unknown as Entry<ImageBitmap>],
    });
    pipeline.page.frame();
    assert.equal(pipeline.view(wid)?.installedOwner, 'country:GER');

    pipeline.retime(wid, { owner: 'country:SOV' });
    assert.equal(pipeline.view(wid)?.mode, 'reading', 'owner change clears installed picture immediately');

    const hitResults: (string | null)[] = [];
    pipeline.pointer(wid, 0, [50, 50], key => { hitResults.push(key); });
    const staleHit: FromLane = { type: 'hit', widget: wid, key: 'row:GER', installedId: 1, owner: 'country:GER' };
    worker.onmessage?.({ data: staleHit } as MessageEvent<FromLane>);
    assert.deepEqual(hitResults, [null], 'late hit from old owner GER is rejected as null');
    assert.equal(pipeline.inspecting(wid), null, 'stale hit never mutates page.inspect');
  } finally {
    pipeline.close();
  }
});

test('quiet ticks inside validInterval perform zero projections and zero paints, and seeking back matches a cold exact audit read', async () => {
  let projectCount = 0;
  let paintCount = 0;
  const presented: Entry<FakeBmp>[] = [];
  const hitLog: { widget: number; key: string | null; installedId?: number; owner?: string }[] = [];

  const projection = {
    kind: 'test-quiet',
    family: 'table' as const,
    project(readView: ReturnType<typeof createBoundedReadView>, _opts: Record<string, never>) {
      projectCount++;
      const panel = readView.panel<{ score: number }>('main');
      return { hour: readView.hour, score: panel?.score ?? 0 };
    },
  };

  const view: LaneView = {
    paint(input, _state, content) {
      paintCount++;
      const panel = input.pictures[0]?.parts[0]?.value;
      const data = panel && panel.kind === 'panel' ? panel.data : undefined;
      const rv = createBoundedReadView({
        hour: input.hour,
        panels: new Map(data !== undefined ? [['main', data]] : []),
      });
      const model = projection.project(rv, {});
      return {
        mode: 'picture',
        nextChangeAt: input.hour < 50 ? 50 : Infinity,
        viewport: { width: 320, height: 200, dpr: 1, scrollLeft: 0, scrollTop: 0 },
        surfaces: [{ source: { width: 320, height: 200 }, content: content() }],
        fields: { model: { value: model, content: JSON.stringify(model) } },
        motion: false,
        hit: () => `row:${model.score}`,
      };
    },
  };

  const lane = new Lane<FakeBmp>({
    lane: 0,
    lanes: 1,
    views: () => view,
    crop: async sources => sources.map((_, i) => makeBmp(i + 1)),
    present: p => { presented.push(...p.entries); },
    release: () => {},
    fail: reason => { throw new Error(reason); },
    later: run => run(),
    hit: (widget, key, installedId, owner) => { hitLog.push({ widget, key, installedId, owner }); },
  });

  lane.follow(1, 'pinned', 'test');
  lane.mount(1, [1], 'test-quiet');
  lane.bind(1, { generation: 1, owner: 'owner-A', surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null });
  lane.exact(1, 10, { score: 42 }, null);

  lane.frame({ type: 'frame', serial: 1, now: 10, target: 10, exact: true, seek: 0, speed: 0, installed: [], refused: [], inspect: [] });
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(projectCount, 1);
  assert.equal(paintCount, 1);
  const firstEntry = presented[0]!;
  assert.equal(firstEntry.owner, 'owner-A');
  assert.equal(firstEntry.lo, 10);
  assert.equal(firstEntry.hi, 49);

  for (let h = 11; h <= 49; h++) {
    lane.frame({
      type: 'frame',
      serial: h,
      now: h,
      target: h,
      exact: false,
      seek: 0,
      speed: 24,
      installed: h === 11 ? [{ widget: 1, binding: 1, id: firstEntry.id }] : [],
      refused: [],
      inspect: [],
    });
  }
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(projectCount, 1, 'quiet ticks inside [10, 49] perform zero projections');
  assert.equal(paintCount, 1, 'quiet ticks inside [10, 49] perform zero paints');

  lane.pointer(1, 0, [10, 10]);
  assert.deepEqual(hitLog.at(-1), { widget: 1, key: 'row:42', installedId: firstEntry.id, owner: 'owner-A' });

  lane.bind(1, { generation: 2, owner: 'owner-B', surfaces: [{ width: 320, height: 200, dpr: 1 }], offset: 0, pin: null });
  lane.pointer(1, 0, [10, 10]);
  assert.deepEqual(hitLog.at(-1), { widget: 1, key: null, installedId: undefined, owner: undefined }, 'owner change clears lane installed hit model immediately');

  const store = new AuditSnapshotStore<BoundAuditEntry<PlanAuditSnapshot<{ hour: number; score: number }, Record<string, never>>>>();
  const auditId = store.create({
    snapshot: {
      projection,
      options: {},
      exactReadView: async (hour: number) => createBoundedReadView({ hour, panels: new Map([['main', { score: 42 }]]) }),
      deriveRows: (m: { hour: number; score: number }) => [{ row: 'score', key: 'score', value: JSON.stringify(knownValue(m.score)) }],
    },
    verify: createPlanAuditVerifier(),
  });
  const reader = createBoundAuditReader(store);
  const verified = await reader(10, auditId);
  assert.ok(verified);
  assert.equal(verified.values.get('score'), JSON.stringify(knownValue((firstEntry.fields.model as { score: number }).score)));
});

test('a recording ending in a trailing gap resolves campaign mode and end pins to its last recorded hour without hanging on outside', () => {
  const branch = {
    id: '1',
    firstHour: 100,
    lastHour: 205,
    gaps: [
      { firstHour: 120, lastHour: 125, reason: 'mid-campaign gap' },
      { firstHour: 201, lastHour: 205, reason: 'trailing shutdown gap' },
    ],
  };
  const track = {
    id: 'main',
    roleId: 'primary',
    label: 'Main',
    branchIndex: 0,
    anchorOffset: null,
    axisOffset: 0,
    selectedTag: null,
    source: fakeSource(),
    branch,
  } as unknown as BoundTrack;

  assert.equal(lastRecordedBranchHour(branch), 200);
  assert.equal(lastRecordedHour(track), 200);

  const campaignWidget = {
    id: 'w1',
    kind: 'campaign-clock',
    configVersion: 1,
    title: 'Clock',
    trackIds: ['main'],
    countries: { kind: 'all' },
    time: { mode: 'campaign' },
  } as WidgetSpec;

  const win = resolveWindow(campaignWidget, track, [track], 150);
  assert.deepEqual(win, { firstHour: 200, lastHour: 200 });

  const historyWidget = { ...campaignWidget, kind: 'unregistered-history-kind' } as WidgetSpec;
  assert.deepEqual(resolveWindow(historyWidget, track, [track], 150), { firstHour: 100, lastHour: 200 });

  const region = resolveRegion({ widget: campaignWidget, action: SPEC }, track, [track], axisAt(150));
  assert.equal(region.hour, 200);
  assert.equal(region.outside, null);

  const pinnedEnd = recordingEndTime({ timeModes: ['pin'], scope: 'moment' }, [track]);
  assert.deepEqual(pinnedEnd, { mode: 'pin', windows: { main: { firstOffset: 100, lastOffset: 100 } } });

  const allGapBranch = {
    id: '2',
    firstHour: 100,
    lastHour: 105,
    gaps: [{ firstHour: 100, lastHour: 105, reason: 'unrecorded' }],
  };
  assert.equal(lastRecordedBranchHour(allGapBranch), null);
});
