import assert from 'node:assert/strict';
import test from 'node:test';
import type { DashboardSource } from '../dashboard/client';
import type { CompiledFollowSubscription, SourceEntry, SourcePlan, SourceTimeShape } from './plan.ts';
import {
  askUnits,
  reaskedUnits,
  settleUnits,
  unitEntries,
  UnitReads,
  watchSpan,
  type ReadUnit,
  type UnitResult,
} from './readUnits.ts';
import type { FrozenGate } from './sources.ts';

const END = 1000;
const branch = { firstHour: 0, lastHour: END };
const source = { id: 'src', revision: 'rev', name: 'Test', firstHour: 0, lastHour: END, branches: [{ id: '1', firstHour: 0, lastHour: END }] } as unknown as DashboardSource;
const gate: FrozenGate = {
  recorded: (hour: number | null | undefined) => (typeof hour === 'number' ? hour : null) as never,
  span: (firstHour: number, lastHour: number) => (firstHour <= lastHour ? { firstHour, lastHour } : null) as never,
};
const all = { mode: 'population', form: 'all' } as const;

function entry(name: string, read: SourceTimeShape, offset = 0): SourceEntry {
  return { name, source, branch: '1', offset, read };
}
function point(name: string, extra: Partial<Extract<SourceTimeShape, { shape: 'point' }>> = {}): SourceEntry {
  return entry(name, { shape: 'point', channel: 'followed', coverage: all, product: 'scoreboard', exactProduct: 'scoreboard', followQuery: null, params: { branchId: '1', of: name }, ...extra });
}
function followedPoint(name: string): SourceEntry {
  return point(name, { followQuery: { product: 'scoreboard', params: { branchId: '1' } } });
}
function range(name: string, firstHour: number, lastHour: number, extra: Partial<Extract<SourceTimeShape, { shape: 'range' }>> = {}): SourceEntry {
  return entry(name, { shape: 'range', coverage: all, product: 'regional-weather', firstHour, lastHour, params: { branchId: '1', of: name }, ...extra });
}
function aggregate(name: string, offset = 0, proven?: (hour: number) => readonly [number, number]): SourceEntry {
  return entry(name, {
    shape: 'branch-to-hour',
    coverage: all,
    product: 'regional-weather',
    followQuery: null,
    params: { branchId: '1', of: name },
    ...(proven ? { provenInterval: (_data: unknown, _requested: readonly [number, number], hour: number) => proven(hour) } : {}),
  }, offset);
}
function subscribed(name: string): CompiledFollowSubscription {
  return { name, source, branch: '1', offset: 0, query: {} as never, channel: 'followed' };
}

function following(entries: readonly SourceEntry[], asked: Readonly<Record<string, number>>, now: number, trackOffset = 0): readonly ReadUnit[] {
  return askUnits(entries, {
    following: true,
    trackOffset,
    content: 0,
    askedAt: name => asked[name] ?? now,
    playback: { branch, effectiveEndHour: END, gate },
  });
}
function pinned(entries: readonly SourceEntry[], hour: number): readonly ReadUnit[] {
  return askUnits(entries, {
    following: false,
    trackOffset: 0,
    content: 0,
    displayHour: hour,
    pinned: { timeMode: 'pin', branch, window: { firstHour: hour, lastHour: hour }, pinHour: hour, gate },
  });
}
function unit(units: readonly ReadUnit[], name: string): ReadUnit {
  const found = units.find(candidate => candidate.name === name);
  assert.ok(found, name);
  return found;
}
function answered(results: ReadonlyMap<string, UnitResult>, ...replies: (readonly [ReadUnit, unknown])[]): Map<string, UnitResult> {
  const out = new Map(results);
  for (const [asked, data] of replies) out.set(asked.name, { key: asked.key!, data });
  return out;
}
const settleFollowing = (units: readonly ReadUnit[], results: ReadonlyMap<string, UnitResult>, hour: number, subscribed = false, moving = true) =>
  settleUnits(units, results, { following: true, moving, hour, subscribed, endHour: END });
const settlePinned = (units: readonly ReadUnit[], results: ReadonlyMap<string, UnitResult>, hour: number) =>
  settleUnits(units, results, { following: false, moving: false, hour, subscribed: false, endHour: END });
const NONE: ReadonlyMap<string, UnitResult> = new Map();

function fakeReads() {
  const calls: { readonly unit: ReadUnit; readonly signal: AbortSignal; readonly resolve: (data: unknown) => void; readonly reject: (error: unknown) => void }[] = [];
  const reads = new UnitReads(
    (asked, signal) => new Promise((resolve, reject) => { calls.push({ unit: asked, signal, resolve, reject }); }),
    () => {},
  );
  return { reads, calls };
}
const settled = () => new Promise<void>(resolve => { setImmediate(resolve); });

test('every entry the follow pipeline does not serve is the same kind of unit, whatever its order (R2-S4)', () => {
  const history = range('history', 100, 500);
  const head = followedPoint('head');
  const names = (plan: SourcePlan, follow: boolean, subs: readonly CompiledFollowSubscription[]) => unitEntries(plan, follow, subs).map(item => item.name);
  assert.deepEqual(names({ owner: 'a', entries: [history, head] }, true, [subscribed('head')]), ['history'], 'a range listed first is read beside a followed point');
  assert.deepEqual(names({ owner: 'b', entries: [head, history] }, true, [subscribed('head')]), ['history']);
  assert.deepEqual(names({ owner: 'a', entries: [history, head] }, false, []), ['history', 'head'], 'pinned, both are read in either order');
  assert.deepEqual(names({ owner: 'c', entries: [followedPoint('one'), followedPoint('two')] }, false, []), ['one', 'two'], 'pinned, two follow-capable points both answer');
  const substitute = point('main');
  assert.deepEqual(names({ owner: 'd', entries: [head, history], staticEntry: substitute }, false, []), ['main', 'history'], 'pinned, an explicit static substitute stands in for the followed entries only');
  assert.deepEqual(names({ owner: 'd', entries: [head, history], staticEntry: substitute }, true, [subscribed('head')]), ['history'], 'following, the substitute is not read');
  assert.deepEqual(names({ owner: 'e', entries: [head, history], staticEntry: null }, false, []), ['history']);
  assert.deepEqual(names({ owner: 'f', entries: [history], refused: 'no' }, true, []), []);
});

test('a bundle is complete only when every unit answers its own current request (R2-S1)', () => {
  const entries = [point('head'), aggregate('history')];
  const at500 = pinned(entries, 500);
  const answered500 = answered(NONE, [unit(at500, 'head'), { hour: 500 }], [unit(at500, 'history'), { through: 500 }]);
  const first = settlePinned(at500, answered500, 500);
  assert.equal(first.kind, 'complete');
  assert.equal(first.kind === 'complete' && first.bundle.hour, 500);

  const at100 = pinned(entries, 100);
  assert.notEqual(unit(at100, 'history').key, unit(at500, 'history').key);
  const headOnly = answered(answered500, [unit(at100, 'head'), { hour: 100 }]);
  assert.deepEqual(settlePinned(at100, headOnly, 100), { kind: 'hold' }, 're-pinned 500 to 100: an aggregate through 500 is never dated 100 beside the new head');
  const both = answered(headOnly, [unit(at100, 'history'), { through: 100 }]);
  const settledAt100 = settlePinned(at100, both, 100);
  assert.equal(settledAt100.kind, 'complete');
  if (settledAt100.kind !== 'complete') return;
  assert.equal(settledAt100.bundle.hour, 100);
  assert.deepEqual(settledAt100.bundle.panels.get('history'), { through: 100 });

  const cold = [range('early', 0, 100), point('head')];
  const coldUnits = pinned(cold, 500);
  const coldSettled = settlePinned(coldUnits, answered(NONE, [unit(coldUnits, 'early'), { rows: [] }], [unit(coldUnits, 'head'), { hour: 500 }]), 500);
  assert.equal(coldSettled.kind === 'complete' && coldSettled.bundle.hour, 500, 'a cold pin at 500: a range ending at 100 does not date its sibling’s hour-500 cells as hour 100');
  assert.deepEqual(coldSettled.kind === 'complete' && coldSettled.bundle.span, [0, 100]);

  const followed = [aggregate('a'), aggregate('b')];
  const at50 = following(followed, { a: 50, b: 60 }, 60);
  assert.deepEqual(settleFollowing(at50, answered(NONE, [unit(at50, 'a'), {}], [unit(at50, 'b'), {}]), 60), { kind: 'hold' }, 'following, replies proven at different hours are never joined');
});

test('a unit not yet applicable bounds the bundle by its known absence, and is asked when it begins (R2-S7)', () => {
  const head = point('head', { provenInterval: (_data, hour) => [hour, 200] });
  const later = range('later', 100, 500);
  const entries = [head, later];
  const at50 = following(entries, {}, 50);
  assert.equal(unit(at50, 'later').spec, null);
  const results = answered(NONE, [unit(at50, 'head'), { hour: 50 }]);
  const before = settleFollowing(at50, results, 50, true);
  assert.deepEqual(before.kind === 'complete' && before.bundle.proof, [50, 99], 'the sibling’s proof through 200 does not cover the bundle past the hour the range begins');
  assert.equal(before.kind === 'complete' && before.bundle.panels.has('later'), false);
  assert.deepEqual(watchSpan(at50, results, END), [50, 99]);
  assert.deepEqual(reaskedUnits(at50, results, 100, END), ['later']);
  const at100 = following(entries, { head: 50, later: 100 }, 100);
  assert.deepEqual(settleFollowing(at100, results, 100, true), { kind: 'hold' }, 'once applicable, the range’s first reply is required');
  const settledAt100 = settleFollowing(at100, answered(results, [unit(at100, 'later'), { rows: [] }]), 100, true);
  assert.deepEqual(settledAt100.kind === 'complete' && settledAt100.bundle.proof, [100, 100]);
  assert.equal(settledAt100.kind === 'complete' && settledAt100.bundle.hour, 100);
  assert.deepEqual(settleFollowing(at50, NONE, 50).kind, 'hold', 'with nothing followed, a pending sibling still holds the bundle');
  assert.deepEqual(settleFollowing(following([later], {}, 50), NONE, 50), { kind: 'outside' });
});

test('a lagging bundle is ordinary while playing, but at rest the displayed hour must lie inside its proof (host R2-S6)', async () => {
  const entries = [aggregate('history', 0, hour => [hour, hour + 20])];
  const { reads, calls } = fakeReads();
  const at100 = following(entries, {}, 100);
  reads.sync(at100, true);
  calls[0]!.resolve({});
  await settled();
  const playing = settleFollowing(at100, reads.results, 150);
  assert.deepEqual(playing.kind === 'complete' && playing.bundle.proof, [100, 120], 'while the clock moves, the last complete bundle is shown dated by its own proof');
  assert.deepEqual(settleFollowing(at100, reads.results, 150, false, false), { kind: 'hold' }, 'paused at 150, a bundle proven 100 to 120 is held as not current');
  assert.deepEqual(reaskedUnits(at100, reads.results, 150, END), ['history']);
  const at150 = following(entries, { history: 150 }, 150);
  reads.sync(at150, true);
  assert.equal(calls.length, 2, 'the unit is asked at 150');
  assert.deepEqual(settleFollowing(at150, reads.results, 150, false, false), { kind: 'hold' });
  calls[1]!.resolve({});
  await settled();
  const resting = settleFollowing(at150, reads.results, 150, false, false);
  assert.deepEqual(resting.kind === 'complete' && resting.bundle.proof, [150, 170], 'it settles when that reply lands');
  reads.dispose();
});

test('a failed read keeps a boundary watch, so a seek asks the new hour (R2-S8)', async () => {
  const entries = [point('head')];
  const { reads, calls } = fakeReads();
  const at50 = following(entries, {}, 50);
  reads.sync(at50, true);
  assert.equal(calls.length, 1);
  calls[0]!.reject(new Error('lost'));
  await settled();
  const failed = settleFollowing(at50, reads.results, 50);
  assert.equal(failed.kind, 'failed');
  assert.deepEqual(watchSpan(at50, reads.results, END), [50, 50]);
  assert.deepEqual(reaskedUnits(at50, reads.results, 80, END), ['head'], 'leaving the failed request’s hour re-asks the unit');
  reads.retry();
  assert.equal(calls.length, 2, 'retry asks the failed request again');
  assert.equal(settleFollowing(at50, reads.results, 50).kind, 'hold');
  calls[1]!.reject(new Error('lost again'));
  await settled();
  const at80 = following(entries, { head: 80 }, 80);
  reads.sync(at80, true);
  assert.equal(calls.length, 3, 'the new hour is asked');
  assert.equal(settleFollowing(at80, reads.results, 80).kind, 'hold', 'the old failure does not stand for the new hour');
  reads.dispose();
});

test('an identical request at a new anchor settles without being asked again (R2-S6)', () => {
  const entries = [range('a', 0, 500), range('b', 100, 500)];
  const { reads, calls } = fakeReads();
  const at700 = following(entries, {}, 700);
  reads.sync(at700, true);
  const results = answered(NONE, [unit(at700, 'a'), { rows: [] }], [unit(at700, 'b'), { rows: [] }]);
  const done = settleFollowing(at700, results, 700);
  assert.deepEqual(done.kind === 'complete' && done.bundle.proof, [500, Infinity], 'a finished range is proven from its last hour on');
  assert.deepEqual(reaskedUnits(at700, results, 600, END), [], 'seeking 700 to 600 stays inside both proofs');
  const atSix = settleFollowing(at700, results, 600);
  assert.equal(atSix.kind === 'complete' && atSix.bundle.hour, 500);
  const reanchored = following(entries, { a: 600, b: 600 }, 600);
  assert.equal(unit(reanchored, 'a').key, unit(at700, 'a').key);
  assert.equal(settleFollowing(reanchored, results, 600).kind, 'complete', 'the same request re-asked at 600 is answered by the reply it already has');
  reads.sync(reanchored, true);
  assert.equal(calls.length, 2, 'and nothing is fetched again');
  reads.dispose();
});

test('removing or adding a unit never cancels or re-asks a sibling in flight (R2-S11)', async () => {
  const a = aggregate('a');
  const b = aggregate('b');
  const c = aggregate('c');
  const { reads, calls } = fakeReads();
  reads.sync(following([a, b], {}, 50), true);
  assert.equal(calls.length, 2);
  reads.sync(following([a], {}, 50), true);
  assert.equal(calls[1]!.signal.aborted, true, 'the removed unit’s read is cancelled');
  assert.equal(calls[0]!.signal.aborted, false, 'its sibling’s read survives');
  reads.sync(following([a, c], {}, 50), true);
  assert.equal(calls.length, 3, 'only the added unit is asked');
  assert.equal(calls[0]!.signal.aborted, false);
  calls[0]!.resolve({ rows: [] });
  await settled();
  assert.equal(reads.results.has('a'), true);
  assert.equal(reads.inFlight, 1);
  reads.dispose();
});

test('each unit is asked at its own track hour and proofs are joined on the common clock (R2-S12)', () => {
  const shifted = aggregate('shifted', 24, hour => [hour, hour + 6]);
  const plain = aggregate('plain', 0, hour => [hour, hour + 2]);
  const units = following([shifted, plain], {}, 10);
  assert.equal(unit(units, 'shifted').anchor, 34, 'the shifted entry asks its own hour');
  assert.equal((unit(units, 'shifted').spec!.request.params as { lastHour: number }).lastHour, 34);
  assert.equal(unit(units, 'plain').anchor, 10);
  const joined = settleFollowing(units, answered(NONE, [unit(units, 'shifted'), {}], [unit(units, 'plain'), {}]), 10);
  assert.deepEqual(joined.kind === 'complete' && joined.bundle.proof, [10, 12]);
  assert.equal(joined.kind === 'complete' && joined.bundle.hour, 10);
});

test('a builder that returns nothing is re-run only when its inputs can change (R2-S9)', () => {
  const builtPoint = point('built', { buildExactParams: (base, hour) => (hour < 60 ? null : { ...base, hour }) });
  const idlePoint = unit(following([builtPoint], {}, 50), 'built');
  assert.equal(idlePoint.spec, null);
  assert.deepEqual(idlePoint.idle, [50, 50], 'an hour-dependent point builder is asked again at the next hour, not never');
  const builtRange = range('built', 100, 500, { buildSpanParams: () => null });
  assert.deepEqual(unit(following([builtRange], {}, 700), 'built').idle, [500, Infinity], 'after a fixed range ends its inputs are fixed, so it is not re-run every hour');
  assert.deepEqual(unit(following([builtRange], {}, 300), 'built').idle, [300, 300]);
});

test('a hundred quiet playback hours ask nothing again', async () => {
  const entries = [
    point('head', { provenInterval: (_data, hour) => [hour, 10_000] }),
    aggregate('history', 0, hour => [hour, 10_000]),
    range('later', 5000, 6000),
  ];
  const { reads, calls } = fakeReads();
  const asked: Record<string, number> = {};
  let units = following(entries, asked, 0);
  let reasks = 0;
  for (let hour = 0; hour <= 100; hour++) {
    const moved = reaskedUnits(units, reads.results, hour, END);
    if (moved.length) {
      reasks++;
      for (const name of moved) asked[name] = hour;
      units = following(entries, asked, hour);
    }
    reads.sync(units, true);
    for (const call of calls) call.resolve({});
    await settled();
    assert.equal(settleFollowing(units, reads.results, hour).kind, 'complete');
  }
  assert.equal(calls.length, 2, 'one request per applicable unit, none per quiet hour');
  assert.equal(reasks, 0);
  assert.deepEqual(watchSpan(units, reads.results, END), [0, 4999]);
  reads.dispose();
});
