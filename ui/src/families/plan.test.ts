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
import { RefusedError } from '../dashboard/queryError.ts';
import { axisAt, lastRecordedHour, resolveRegion, SPEC } from '../dashboard/regionTime.ts';
import type { TiledLine } from '../dashboard/tiles/view';
import { Lane, type LaneView } from '../draw/lane.ts';
import type { FromLane } from '../draw/laneMessages';
import type { MissingSpan } from '../generated/dashboard';
import { LaneArtCache } from './art/laneCache.ts';
import { artUrlForHandle, createLaneArtProvider, flagArtHandle } from './art/provider.ts';
import { readyArt } from './art/types.ts';
import { AuditSnapshotStore } from './audit.ts';
import {
  BoundedSeriesCursor,
  boundedUnrecorded,
  boundTiledSeries,
  createBoundedReadView,
  cutStraddlingBucket,
  displayedHour,
  FutureDataError,
  type BoundedWorkCounter,
} from './boundedView.ts';
import {
  assertValidCoverage,
  boundedEntries,
  buildPlaybackBoundedRequest,
  composeProof,
  compileFollowPlan,
  CoverageContractError,
  FamilySourceBroker,
  needsPlaybackBoundedRead,
  currentHovers,
  hoverAsks,
  planQueryKey,
  planSignature,
  playbackIdleInterval,
  readPlanExact,
  resolvePlaybackProvenInterval,
  stabilizeCanvasDimension,
} from './broker.ts';
import { createBoundAuditReader, createPlanAuditVerifier, type BoundAuditEntry, type PlanAuditSnapshot } from './host/auditBinding.ts';
import { clientPointToInstalled, resolveInstalledAction } from './host/pointer.ts';
import { IdentityIndex } from './identity/index.ts';
import { identityResultFromIndex } from './identity/provider.ts';
import type {
  ArtHandle,
  ArtProvider,
  DisplayedHour,
  EntityKind,
  IdentityProvider,
  InstalledPicture,
  SourcePlan,
} from './plan.ts';
import { resolveDisplayedStaticSnapshot, type FrozenGate } from './sources.ts';
import { TailWindowController, type DatedLogRow, type TableRowSource } from './table/rowSource.ts';
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
  assert.ok(initialCounter.allocations <= 8, `binding a 10-year line copies no history: ${initialCounter.allocations} allocations`);
  for (const slice of [initial.buckets, initial.missing, initial.marks]) {
    assert.ok(Object.values(slice).every(value => !Array.isArray(value)), 'a bounded slice exposes no backing array');
  }

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
    demandOwner: 'view',
    resolve(handle) {
      if (handle.kind === 'flag' && handle.choice === 'ready-flag') {
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

  const readyArtResult = view.art({ kind: 'flag', choice: 'ready-flag', size: 'panel' });
  assert.equal(readyArtResult.status, 'ready');
  assert.equal(artDemands.length, 0);

  const pendingHandle: ArtHandle = { kind: 'flag', choice: 'pending-flag', size: 'panel' };
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
  assert.throws(
    () => assertValidCoverage('row-every', { mode: 'keyed', keys: ['GER'] }, { product: 'series-state', coverage: 'row', params: { reads: 'every' } }),
    CoverageContractError,
    'row coverage over every read is a population request, which the product refuses',
  );
  assert.throws(
    () => assertValidCoverage('row-every-outer', { mode: 'keyed', keys: ['GER'] }, { product: 'series-state', coverage: 'row', reads: 'every' }),
    CoverageContractError,
  );
  assertValidCoverage('row-keys', { mode: 'keyed', keys: ['GER'] }, { product: 'series-state', coverage: 'row', params: { reads: [{ country: 'GER' }] } });

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
  assert.notEqual(planSignature({ ...makePlan(1), staticEntry: null }), planSignature(makePlan(1)), 'an explicit null static entry is not an omitted one');
  assert.notEqual(planSignature({ ...makePlan(1), exactEntry: null }), planSignature(makePlan(1)), 'an explicit null exact entry is not an omitted one');

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

  const replaced: string[] = [];
  let resolveOld: ((v: string) => void) | null = null;
  let resolveNew: ((v: string) => void) | null = null;
  let newDelivered: (() => void) | null = null;
  const newDone = new Promise<void>(res => { newDelivered = res; });
  const oldHandle = broker.runSlot('w2', 'old', () => new Promise<string>(res => { resolveOld = res; }), v => replaced.push(v));
  oldHandle.cancel();
  broker.runSlot('w2', 'new', () => new Promise<string>(res => { resolveNew = res; }), v => { replaced.push(v); newDelivered!(); });
  oldHandle.cancel();
  resolveOld!('old');
  resolveNew!('new');
  await newDone;
  assert.deepEqual(replaced, ['new'], 'cancelling an old handle twice never drops its replacement');

  const src2 = fakeSource('src-2');
  const hoverPlan = (params: Record<string, unknown>, extra: SourcePlan['entries'] = []): SourcePlan => ({
    owner: 'hover:1',
    entries: [{ name: 'hover', source: src2, branch: '1', offset: 0, read: { shape: 'exact-hover', product: 'series-state', params } }, ...extra],
  });
  const asksA = hoverAsks(hoverPlan({ countries: ['GER'] }), 40.7);
  assert.equal(asksA.length, 1);
  assert.equal(asksA[0]!.hour, 40);
  const heldA = { hover: { key: asksA[0]!.key, hour: 40, data: 'GER at 40' } };
  assert.deepEqual(currentHovers(asksA, heldA), { hover: { hour: 40, data: 'GER at 40' } });
  assert.deepEqual(currentHovers(hoverAsks(hoverPlan({ countries: ['SOV'] }), 40), heldA), {}, 'a hover answered for another selection is not shown');
  assert.deepEqual(currentHovers(hoverAsks(hoverPlan({ countries: ['GER'] }), 41), heldA), {}, 'a hover answered for another hour is not shown');
  assert.deepEqual(currentHovers(hoverAsks(hoverPlan({ countries: ['GER'] }), null), heldA), {});

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

test('seeking through a gap and returning to the same hour with a new owner clears installed state and inspection, and rejects late entries from the old owner', () => {
  const posted: { readonly inspect?: readonly (readonly [number, string])[] }[] = [];
  const page = new Page<FakeBmp>({ lanes: 1, post: (_lane, message) => { posted.push(message as { readonly inspect?: readonly (readonly [number, string])[] }); } });
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
  page.inspect(1, 'row:GER');
  assert.deepEqual(posted.filter(message => message.inspect).at(-1)?.inspect, [[1, 'row:GER']]);
  page.rebind(1, { owner: 'country:SOV' });
  assert.deepEqual(posted.filter(message => message.inspect).at(-1)?.inspect, [], 'owner B never inherits owner A\'s inspected row');
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

test('no-follow reads during playback ask nothing after the displayed hour, prove only what the reply evidences, and a reply stays bound to the request that produced it', () => {
  const src = fakeSource();
  const gate: FrozenGate = {
    recorded: (h: number | null | undefined) => (typeof h === 'number' ? h : null) as never,
    span: (firstHour: number, lastHour: number) => ({ firstHour, lastHour } as never),
  };
  const branch = { firstHour: 0, lastHour: 1000 };
  const ask = (plan: SourcePlan, currentHour: number, effectiveEndHour = 1000) => buildPlaybackBoundedRequest(plan.entries[0]!, { branch, effectiveEndHour, currentHour, gate });
  const planOf = (read: SourcePlan['entries'][number]['read']): SourcePlan => ({
    owner: `${read.shape}:1`,
    entries: [{ name: 'main', source: src, branch: '1', offset: 0, read }],
  });

  const branchPlan = planOf({
    shape: 'branch-to-hour',
    coverage: { mode: 'population', form: 'all' },
    product: 'regional-weather',
    followQuery: null,
    params: { branchId: '1' },
  });
  const compiled = compileFollowPlan(branchPlan, () => ({ query: null, refused: null }));
  assert.equal(compiled.subscriptions.length, 0);
  assert.equal(needsPlaybackBoundedRead(branchPlan, compiled.subscriptions), true);
  assert.equal(needsPlaybackBoundedRead({ ...branchPlan, allowEmptyFollow: true }, []), false);

  const atFifty = ask(branchPlan, 50);
  assert.ok(atFifty);
  assert.deepEqual(atFifty.span, [0, 50], 'an aggregate is asked through the displayed hour, never past it');
  assert.equal((atFifty.request.params as { lastHour: number }).lastHour, 50);
  assert.equal(atFifty.anchorHour, 50);
  assert.deepEqual(atFifty.dynamicKeys, ['firstHour', 'lastHour'], 'the produced span moves without dropping the held reply');
  const replyWithLaterRow = { rows: [{ hour: 20 }, { hour: 200 }] };
  assert.deepEqual(resolvePlaybackProvenInterval(atFifty.entry.read, replyWithLaterRow, atFifty, 1000), [50, 50], 'the requested span is not proof: without evidence a reply proves only its own hour');
  assert.deepEqual(ask(branchPlan, 120)?.span, [0, 120]);

  const evidenced = planOf({
    shape: 'branch-to-hour',
    coverage: { mode: 'population', form: 'all' },
    product: 'regional-weather',
    followQuery: null,
    params: { branchId: '1' },
    provenInterval: (data, requested, hour) => ((data as { until?: number }).until !== undefined ? [requested[0], (data as { until: number }).until] : [hour + 5, hour + 9]),
  });
  const evidencedAsk = ask(evidenced, 50)!;
  assert.deepEqual(resolvePlaybackProvenInterval(evidencedAsk.entry.read, { until: 80 }, evidencedAsk, 1000), [50, 80], 'evidence extends proof forward, never back before the asked hour');
  assert.deepEqual(resolvePlaybackProvenInterval(evidencedAsk.entry.read, {}, evidencedAsk, 1000), [50, 50], 'evidence that does not cover the asked hour proves nothing more');

  const trailingPlan = planOf({
    shape: 'trailing-window',
    windowHours: 168,
    coverage: { mode: 'population', form: 'all' },
    product: 'naval-supremacy',
    followQuery: null,
    params: { branchId: '1' },
  });
  assert.equal(needsPlaybackBoundedRead(trailingPlan, []), true);
  assert.deepEqual(ask(trailingPlan, 200)?.span, [33, 200]);

  const rangePlan = planOf({
    shape: 'range',
    coverage: { mode: 'population', form: 'all' },
    product: 'regional-weather',
    firstHour: 100,
    lastHour: 500,
    params: { branchId: '1' },
  });
  assert.equal(needsPlaybackBoundedRead(rangePlan, []), true);
  assert.equal(ask(rangePlan, 50), null, 'nothing is asked before a fixed range starts');
  assert.deepEqual(playbackIdleInterval(rangePlan.entries[0]!, { branch, effectiveEndHour: 1000, currentHour: 50, gate }), [-Infinity, 99], 'before the range the next boundary is its first hour');
  const rangeInside = ask(rangePlan, 250)!;
  assert.deepEqual(rangeInside.span, [100, 250]);
  assert.deepEqual(resolvePlaybackProvenInterval(rangeInside.entry.read, {}, rangeInside, 1000), [250, 250]);
  const rangeDone = ask(rangePlan, 700)!;
  assert.deepEqual(rangeDone.span, [100, 500]);
  assert.deepEqual(resolvePlaybackProvenInterval(rangeDone.entry.read, {}, rangeDone, 1000), [700, Infinity], 'a finished range never re-arms its boundary');

  const pastEnd = ask(branchPlan, 1200, 900)!;
  assert.deepEqual(pastEnd.span, [0, 900]);
  assert.deepEqual(resolvePlaybackProvenInterval(pastEnd.entry.read, {}, pastEnd, 900), [900, Infinity], 'the last recorded hour holds for the rest of playback');

  const pointPlan = planOf({
    shape: 'point',
    channel: 'followed',
    coverage: { mode: 'population', form: 'all' },
    product: 'scoreboard',
    exactProduct: 'scoreboard',
    followQuery: null,
    params: { branchId: '1' },
  });
  const gappedGate: FrozenGate = {
    ...gate,
    recorded: (h: number | null | undefined) => (typeof h === 'number' && (h < 300 || h > 340) ? h : null) as never,
    recordedRun: (h: number) => (h < 300 ? [0, 299] : h <= 340 ? [300, 340] : [341, 1000]),
  };
  assert.equal(buildPlaybackBoundedRequest(pointPlan.entries[0]!, { branch, effectiveEndHour: 1000, currentHour: 310, gate: gappedGate }), null);
  assert.deepEqual(playbackIdleInterval(pointPlan.entries[0]!, { branch, effectiveEndHour: 1000, currentHour: 310, gate: gappedGate }), [300, 340], 'inside a gap the next boundary is the gap end, not the next hour');

  const followedHead: SourcePlan['entries'][number] = {
    name: 'head',
    source: src,
    branch: '1',
    offset: 0,
    read: { shape: 'point', channel: 'followed', coverage: { mode: 'population', form: 'all' }, product: 'scoreboard', followQuery: { product: 'scoreboard', params: { branchId: '1' } }, params: { branchId: '1' } },
  };
  const history = { ...rangePlan.entries[0]!, name: 'history' };
  const mixed: SourcePlan = { owner: 'mixed:1', entries: [followedHead, history] };
  const headFollowed = [{ name: 'head', source: src, branch: '1', offset: 0, query: {} as never, channel: 'followed' as const }];
  assert.deepEqual(boundedEntries(mixed, true, headFollowed).map(entry => entry.name), ['history'], 'a followed point never stops an independent range from loading');
  assert.equal(needsPlaybackBoundedRead(mixed, headFollowed), true);
  assert.deepEqual(boundedEntries(mixed, false, []).map(entry => entry.name), ['head', 'history'], 'pinned, the range is read beside the primary');
  assert.deepEqual(boundedEntries(mixed, true, []).map(entry => entry.name), ['history'], 'an entry with a follow query is never read by the bounded path while following');
  assert.deepEqual(boundedEntries({ ...mixed, refused: 'no' }, true, headFollowed), []);
  const exactPrimary: SourcePlan = { owner: 'exact:1', entries: [followedHead], staticEntry: pointPlan.entries[0]! };
  assert.deepEqual(boundedEntries(exactPrimary, true, headFollowed), [], 'a static entry standing in for followed entries is not read while they follow');
  assert.deepEqual(boundedEntries(exactPrimary, false, []).map(entry => entry.name), ['main']);

  assert.deepEqual(composeProof([{ anchorHour: 50, proof: [50, 80] }, { anchorHour: 60, proof: [60, 60] }]), [60, 60], 'each entry keeps its own proof; the picture holds where all of them do');
  assert.deepEqual(composeProof([{ anchorHour: 50, proof: [50, 80] }, { anchorHour: 90, proof: [90, 90] }]), null, 'a reply proven only before another entry’s asked hour is not installed with it');
  assert.equal(composeProof([]), null);

  const ctx = { owner: 'a' };
  const held = resolveDisplayedStaticSnapshot(null, {
    data: { rows: [{ hour: 480 }] }, moment: JSON.stringify([0, 500]), requestKey: 'asked-at-500', anchorHour: 500, satisfiesCurrent: true,
    requestedSpan: [0, 500], context: ctx, gate, extra: undefined,
  });
  const afterSeekBack = resolveDisplayedStaticSnapshot(held, {
    data: { rows: [{ hour: 480 }] }, moment: JSON.stringify([0, 500]), requestKey: 'asked-at-500', anchorHour: 500, satisfiesCurrent: false,
    requestedSpan: [0, 100], context: ctx, gate, extra: undefined,
  });
  assert.equal(afterSeekBack?.anchorHour, 500, 'after a seek back the held reply keeps the hour it answered, not the new request’s');
  assert.deepEqual(afterSeekBack?.span, [0, 500]);
});

test('a family cannot reach data after the displayed hour through a slice, a tile status or a frozen row source', () => {
  const peak = 999;
  const line: TiledLine = {
    tag: 'GER',
    head: 'GER',
    decimals: 0,
    summary: {
      from: 0,
      to: 119,
      width: 100,
      buckets: [makeBucket(0, 23, 1, 1, 1, 1), null, null, null, makeBucket(96, 119, peak, peak, peak, peak)],
      tiles: { binHours: 24, valueAt: () => null },
    },
    missing: [{ firstHour: 30, lastHour: 40, reason: 'later gap' }],
    parts: [],
    marks: { from: 0, to: 119, width: 100, clusters: [{ firstHour: 99, lastHour: 99, firstOrdinal: '0', count: '1', bins: [[99, 1]] }] },
  };
  const early = boundTiledSeries(line, displayedHour(5));
  assert.equal(early.buckets.length, 0);
  const reachable = (root: unknown): unknown[] => {
    const seen = new Set<unknown>();
    const stack = [root];
    while (stack.length) {
      const value = stack.pop();
      if (value === null || typeof value !== 'object' || seen.has(value)) continue;
      seen.add(value);
      for (const child of Object.values(value)) stack.push(child);
    }
    return [...seen];
  };
  assert.ok(!reachable(early).some(value => (value as { maxValue?: number }).maxValue === peak), 'the hour-99 peak is unreachable at H=5');
  assert.ok(!reachable(early).some(value => (value as { reason?: string }).reason === 'later gap'), 'a later gap is unreachable at H=5');
  assert.ok(!reachable(early).some(value => (value as { firstHour?: number }).firstHour === 99), 'a later mark is unreachable at H=5');

  const reasons = { summary: { from: 0, to: 90 }, missing: [{ firstHour: 0, lastHour: 10, reason: 'A' }, { firstHour: 11, lastHour: 90, reason: 'B' }] };
  assert.deepEqual(boundedUnrecorded([reasons], 4), { reason: 'A', validInterval: [0, 10] }, 'the body names the reason in force at H, not a later one');
  assert.deepEqual(boundedUnrecorded([reasons], 50), { reason: 'B', validInterval: [11, Infinity] });
  const recordedLater = { summary: { from: 0, to: 90 }, missing: [{ firstHour: 0, lastHour: 29, reason: 'C' }] };
  assert.deepEqual(boundedUnrecorded([reasons, recordedLater], 40), { reason: null, validInterval: [30, Infinity] }, 'once any line is recorded the body is ready for good');
  assert.deepEqual(boundedUnrecorded([recordedLater, reasons], 4).validInterval, [0, 10]);
  assert.deepEqual(boundedUnrecorded([], 4), { reason: null, validInterval: [-Infinity, Infinity] });

  const rows = new TailWindowController<{ key: string; hour: number }>('log', 2);
  const reply = (hour: number) => ({ total: 10, offset: 0, baseOrdinal: 0, firstOrdinal: 0, rows: [{ key: `a${hour}`, hour }, { key: `b${hour}`, hour }], facetTotals: new Map<string, number>(), missing: [] });
  rows.applyHead(100, reply(100), 3);
  rows.setWindow({ first: 0, count: 5 });
  rows.applyHead(200, reply(200), 9);
  const frozen = createBoundedReadView({ hour: 150, rows: new Map([['log', rows.snapshot()]]) }).rows('log')!;
  assert.equal(frozen.at, 100);
  assert.equal(frozen.liveCount, 3, 'a source frozen at 100 counts what its own hour held, not a later head');
});

test('the exact read of a pixel-binned chart asks its exact product at the hour, in the shape of its head read', async () => {
  const src = fakeSource();
  const asked: { product: string; params: Record<string, unknown> }[] = [];
  const reply = { values: [{ country: 'GER', value: 42 }] };
  const client = {
    product: async (request: { product: string; params: Record<string, unknown> }) => {
      asked.push({ product: request.product, params: request.params });
      return { data: reply };
    },
  } as unknown as DashboardClient;
  const family = { kind: 'country-measure', measure: 'politicalPower' };
  const chartPlan: SourcePlan = {
    owner: 'political-power:1',
    entries: [{
      name: 'main',
      source: src,
      branch: '1',
      offset: 0,
      read: {
        shape: 'pixel-binned',
        coverage: { mode: 'keyed', keys: ['GER'] },
        family,
        tags: ['GER'],
        headQuery: { product: 'series-state', params: { branchId: '1', family, reads: [{ country: 'GER' }] } },
        exactProduct: 'series-state',
      },
    }],
  };
  const gate: FrozenGate = {
    recorded: (h: number | null | undefined) => (typeof h === 'number' ? h : null) as never,
    span: (firstHour: number, lastHour: number) => ({ firstHour, lastHour } as never),
  };
  const view = await readPlanExact(chartPlan, 120, {
    client,
    following: true,
    timeMode: 'follow',
    branch: { firstHour: 0, lastHour: 1000 },
    window: null,
    staticSpan: null,
    gate,
  });
  assert.equal(asked.length, 1, 'one exact request, not none');
  assert.deepEqual(asked[0], { product: 'series-state', params: { branchId: '1', family, reads: [{ country: 'GER', hour: 120 }] } });
  assert.equal(view?.hour, 120);
  assert.equal(view?.panel('main'), reply);
});

test('advancing through an open bucket and a growing mark cluster costs what changed, and replaced prefix points are never served from the cache', () => {
  const runs: Run[] = [];
  for (let h = 0; h < 1024; h++) runs.push({ firstHour: h, lastHour: h, value: h % 97 });
  const bins: [number, number][] = [];
  for (let h = 0; h < 1024; h++) bins.push([h, 1]);
  const line: TiledLine = {
    tag: 'GER',
    head: 'GER',
    decimals: 0,
    summary: { from: 0, to: 1023, width: 1, buckets: [makeBucket(0, 1023, 0, 0, 0, 96)], runs, tiles: { binHours: 1024, valueAt: () => null } },
    missing: [],
    parts: [],
    marks: { from: 0, to: 1023, width: 1, clusters: [{ firstHour: 0, lastHour: 1023, firstOrdinal: '0', count: '1024', bins }] },
  };
  const cursor = new BoundedSeriesCursor();
  cursor.advance(line, 100);
  let worst = 0;
  for (let h = 101; h <= 900; h++) {
    const counter: BoundedWorkCounter = { visits: 0, allocations: 0 };
    const bounded = cursor.advance(line, h, { counter });
    worst = Math.max(worst, counter.visits);
    if (h === 900) {
      const cut = bounded.buckets.at(0)!;
      assert.deepEqual([cut.lastHour, cut.maxValue], [900, 96], 'the resumed cut equals a fresh one');
      const tail = bounded.marks.at(0)!;
      assert.equal(tail.count, '901');
      assert.equal(tail.bins?.length, 901);
      assert.ok(!Array.isArray(tail.bins), 'the clipped cluster shares its bins rather than copying them');
    }
  }
  assert.ok(worst <= 48, `one more hour inside an open bucket visits a constant number of items, not the bucket: ${worst}`);
  const fresh = boundTiledSeries(line, displayedHour(900));
  assert.deepEqual(fresh.buckets.at(0), cursor.advance(line, 900).buckets.at(0));

  const pointLine: TiledLine = { ...line, summary: { ...line.summary, runs: undefined } };
  const pointCursor = new BoundedSeriesCursor();
  const low = pointCursor.advance(pointLine, 105, { prefixPoints: [{ hour: 100, value: 1 }] });
  assert.equal(low.buckets.at(0)?.maxValue, 1);
  const high = pointCursor.advance(pointLine, 105, { prefixPoints: [{ hour: 100, value: 1 }, { hour: 104, value: 8 }] });
  assert.equal(high.buckets.at(0)?.maxValue, 8, 'new prefix points at the same hour are read, as a fresh cursor would');
});

test('canvas pixel dimensions stay stable across sub-pixel jitter without changing planSignature', () => {
  const src = fakeSource();
  const makePlan = (width: number | undefined, height: number | undefined): SourcePlan => ({
    owner: 'chart:1',
    entries: [{
      name: 'main',
      source: src,
      branch: '1',
      offset: 0,
      read: {
        shape: 'branch-to-hour',
        coverage: { mode: 'population', form: 'all' },
        product: 'regional-weather',
        followQuery: null,
        params: { width, height },
      },
    }],
  });

  const w0 = stabilizeCanvasDimension(400.2, undefined);
  const h0 = stabilizeCanvasDimension(240.3, undefined);
  assert.equal(w0, 400);
  assert.equal(h0, 240);
  const baseSig = planSignature(makePlan(w0, h0));

  for (const jitter of [400.49, 400.51, 400.9, 399.2, 401.1]) {
    const wNext = stabilizeCanvasDimension(jitter, w0);
    assert.equal(wNext, w0, `jitter ${jitter} keeps width stable`);
    assert.equal(planSignature(makePlan(wNext, h0)), baseSig);
  }

  const wSubPixelDeadband1 = stabilizeCanvasDimension(400.8, 400, 1);
  assert.equal(wSubPixelDeadband1, 400);
  assert.equal(planSignature(makePlan(wSubPixelDeadband1, h0)), baseSig);

  const wResized = stabilizeCanvasDimension(520, w0);
  assert.equal(wResized, 520);
  assert.notEqual(planSignature(makePlan(wResized, h0)), baseSig);
});

test('FieldStatus failed distinguishes retryable transport errors and non-retryable refusals from genuine absence', () => {
  const scope = { sourceId: 's1', revision: 'r1', branchId: '1', content: 1 };
  const retryableIndex = new IdentityIndex(scope, [0, 500]);
  retryableIndex.setFactsFailure(new Error('IPC timeout'));
  const retryableRes = identityResultFromIndex(retryableIndex, 'country', 'GER', displayedHour(100));
  assert.deepEqual(retryableRes.name, { status: 'failed', reason: 'IPC timeout', retryable: true });
  assert.deepEqual(retryableRes.holder, { status: 'failed', reason: 'IPC timeout', retryable: true });

  const refusedIndex = new IdentityIndex(scope, [0, 500]);
  refusedIndex.setFactsFailure(new RefusedError('Unsupported mod table'));
  const refusedRes = identityResultFromIndex(refusedIndex, 'country', 'GER', displayedHour(100));
  assert.deepEqual(refusedRes.name, { status: 'failed', reason: 'Unsupported mod table', retryable: false });

  const absentIndex = new IdentityIndex(scope, [0, 500]);
  absentIndex.setFacts({ branchId: '1', known: [[0, 500]], names: [], reasons: [], flags: [], countries: [], warnings: [] }, true);
  absentIndex.setIdentities({ branchId: '1', known: [], countries: [], unknown: [], warnings: [] });
  const absentRes = identityResultFromIndex(absentIndex, 'country', 'ZZZ', displayedHour(100));
  assert.equal(absentRes.name.status, 'absent');
});

test('ArtHandle discriminated union covers flag, sprite, portrait, path, and url, and BoundedReadView fires demand callbacks once', () => {
  const handles: readonly ArtHandle[] = [
    flagArtHandle('GER_fascism', 'shield'),
    { kind: 'sprite', key: 'GFX_pol_power', frame: 0, recorded: 'pol_power', text: true },
    { kind: 'portrait', key: 'GFX_portrait_ger', owner: 'GER', recorded: 'leader_1' },
    { kind: 'path', path: 'gfx/interface/icons/icon.dds', frame: 1, frames: 2 },
    { kind: 'url', url: 'https://art.test/direct.png' },
  ];
  assert.equal(handles[0]!.kind, 'flag');
  assert.equal(artUrlForHandle(handles[4]!), 'https://art.test/direct.png');
  assert.equal(artUrlForHandle(handles[1]!), null);

  const cache = new LaneArtCache();
  const seenKinds: string[] = [];
  const artProvider = createLaneArtProvider(cache, {
    resolveHandle(handle) {
      seenKinds.push(handle.kind);
      if (handle.kind === 'sprite' && handle.text) {
        return readyArt('art://sprite-text');
      }
      return { status: 'pending' };
    },
  });
  assert.equal(artProvider.demandOwner, 'view');

  const identityResolveArgs: number[] = [];
  const identityProvider: IdentityProvider = {
    demandOwner: 'view',
    resolve(kind, key, atHour) {
      identityResolveArgs.push(arguments.length);
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

  let identityDemandCount = 0;
  let artDemandCount = 0;
  const view = createBoundedReadView({
    hour: 100,
    identity: identityProvider,
    onIdentityDemand: () => { identityDemandCount++; },
    art: artProvider,
    onArtDemand: () => { artDemandCount++; },
  });

  const whoRes = view.who('country', 'GER', 80);
  assert.equal(whoRes.name.status, 'pending');
  assert.equal(identityDemandCount, 1);
  assert.deepEqual(identityResolveArgs, [3]);

  const spriteRes = view.art(handles[1]!);
  assert.equal(spriteRes.status, 'pending');
  assert.equal(artDemandCount, 1);
  assert.deepEqual(seenKinds, ['sprite']);

  const portraitRes = view.art(handles[2]!);
  assert.equal(portraitRes.status, 'pending');
  assert.equal(artDemandCount, 2);

  cache.dispose();
});
