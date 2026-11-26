import assert from 'node:assert/strict';
import test from 'node:test';
import type { DashboardSource } from '../dashboard/client';
import { inProcess } from '../dashboard/follow/inProcess.fixture.ts';
import type { PageSurface } from '../dashboard/follow/page.ts';
import type { LaneView } from '../draw/lane';
import { createBoundedReadView, FutureDataError } from './boundedView.ts';
import { FamilySourceBroker } from './broker.ts';
import type { CompiledFollowSubscription, SourceEntry, SourcePlan, SourceTimeShape } from './plan.ts';
import {
  askUnits,
  ReadTarget,
  settleUnits,
  shownView,
  unitEntries,
  UnitReads,
  type ReadUnit,
  type TargetClock,
  type TargetConfig,
  type UnitResult,
} from './readUnits.ts';
import type { FrozenGate } from './sources.ts';
import { revisionOf } from '../dashboard/follow/payloads.ts';
import type { FamilyStaticSources } from './host/adapter.ts';
import { boundDated, ownProof, StaticSupply } from './host/dated.ts';

const END = 10_000;
const branch = { firstHour: 0, lastHour: END };
const source = { id: 'src', revision: 'rev', name: 'Test', firstHour: 0, lastHour: END, branches: [{ id: '1', firstHour: 0, lastHour: END }] } as unknown as DashboardSource;
const gate: FrozenGate = {
  recorded: (hour: number | null | undefined) => (typeof hour === 'number' ? hour : null) as never,
  span: (firstHour: number, lastHour: number) => (firstHour <= lastHour ? { firstHour, lastHour } : null) as never,
};
const all = { mode: 'population', form: 'all' } as const;
const PLAYBACK = { branch, effectiveEndHour: END, gate };

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
function aggregate(name: string, offset = 0, proven?: (hour: number) => readonly [number, number], extra: Partial<Extract<SourceTimeShape, { shape: 'branch-to-hour' }>> = {}): SourceEntry {
  return entry(name, {
    shape: 'branch-to-hour',
    coverage: all,
    product: 'regional-weather',
    followQuery: null,
    params: { branchId: '1', of: name },
    ...(proven ? { provenInterval: (_data: unknown, _requested: readonly [number, number], hour: number) => proven(hour) } : {}),
    ...extra,
  }, offset);
}
function subscribed(name: string): CompiledFollowSubscription {
  return { name, source, branch: '1', offset: 0, query: {} as never, channel: 'followed' };
}

function following(entries: readonly SourceEntry[], hour: number, trackOffset = 0): readonly ReadUnit[] {
  return askUnits(entries, { following: true, trackOffset, content: 0, hour, playback: () => PLAYBACK });
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
  settleUnits(units, results, { following: true, moving, hour, subscribed });
const settlePinned = (units: readonly ReadUnit[], results: ReadonlyMap<string, UnitResult>, hour: number) =>
  settleUnits(units, results, { following: false, moving: false, hour, subscribed: false });
const NONE: ReadonlyMap<string, UnitResult> = new Map();
const settled = () => new Promise<void>(resolve => { setImmediate(resolve); });

function fakeReads() {
  const calls: { readonly unit: ReadUnit; readonly signal: AbortSignal; readonly resolve: (data: unknown) => void; readonly reject: (error: unknown) => void }[] = [];
  const reads = new UnitReads(
    (asked, signal) => new Promise((resolve, reject) => { calls.push({ unit: asked, signal, resolve, reject }); }),
    () => {},
  );
  return { reads, calls };
}

function fakeClock(start: number, playing = true) {
  let hour = start;
  let moving = playing;
  let seek = 0;
  let crossings = 0;
  const anchors = new Set<() => void>();
  const watches = new Set<{ readonly lo: number; readonly hi: number; readonly left: () => void }>();
  const clock: TargetClock = {
    now: () => hour,
    playing: () => moving,
    seek: () => seek,
    subscribe(changed) {
      anchors.add(changed);
      return () => { anchors.delete(changed); };
    },
    watch([lo, hi], left) {
      const watch = { lo, hi, left };
      watches.add(watch);
      return () => { watches.delete(watch); };
    },
  };
  const cross = () => {
    for (const watch of [...watches]) {
      if (hour >= watch.lo && hour <= watch.hi) continue;
      watches.delete(watch);
      crossings++;
      watch.left();
    }
  };
  const anchor = () => { for (const listener of [...anchors]) listener(); };
  return {
    clock,
    get crossings() { return crossings; },
    advance(to: number) { hour = to; cross(); },
    pause() { moving = false; anchor(); },
    play() { moving = true; anchor(); },
    seekTo(to: number) { hour = to; seek++; anchor(); cross(); },
  };
}

interface Call { readonly unit: ReadUnit; readonly resolve: (data: unknown) => void; readonly reject: (error: unknown) => void }

function fakeServer(auto?: (unit: ReadUnit) => unknown) {
  const calls: Call[] = [];
  const pending: Call[] = [];
  const load = (asked: ReadUnit) => new Promise<unknown>((resolve, reject) => {
    const call = { unit: asked, resolve, reject };
    calls.push(call);
    if (auto) {
      const reply = auto(asked);
      if (reply instanceof Error) reject(reply);
      else resolve(reply);
    } else pending.push(call);
  });
  const answer = (name: string, data: unknown = { from: name }) => {
    const at = pending.findIndex(call => call.unit.name === name);
    assert.ok(at >= 0, `no pending read for ${name}`);
    pending.splice(at, 1)[0]!.resolve(data);
  };
  const refuse = (name: string, error = new Error('lost')) => {
    const at = pending.findIndex(call => call.unit.name === name);
    assert.ok(at >= 0, `no pending read for ${name}`);
    pending.splice(at, 1)[0]!.reject(error);
  };
  return { calls, pending, load, answer, refuse };
}

function followConfig(entries: readonly SourceEntry[], extra: Partial<Extract<TargetConfig, { following: true }>> = {}): TargetConfig {
  return { inputs: 'test', following: true, entries, active: true, content: 0, trackOffset: 0, subscribed: false, playback: () => PLAYBACK, ...extra };
}
function pinConfig(entries: readonly SourceEntry[], hour: number): TargetConfig {
  return {
    inputs: 'test',
    following: false,
    entries,
    active: true,
    content: 0,
    trackOffset: 0,
    displayHour: hour,
    pinned: { timeMode: 'pin', branch, window: { firstHour: hour, lastHour: hour }, pinHour: hour, gate },
  };
}

function watched(target: ReadTarget) {
  const counts = { views: 0, proofs: 0, bundles: new Set<unknown>() };
  target.subscribe(() => {
    counts.views++;
    const bundle = target.view().bundle;
    if (bundle) counts.bundles.add(bundle);
  });
  target.subscribeProof(() => { counts.proofs++; });
  return counts;
}

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
});

test('a unit not yet applicable bounds the bundle by its known absence, and is asked when it begins (R2-S7)', async () => {
  const head = point('head', { provenInterval: (_data, hour) => [hour, 200] });
  const later = range('later', 100, 500);
  const { clock, advance } = fakeClock(50);
  const server = fakeServer();
  const target = new ReadTarget(clock, server.load);
  target.configure(followConfig([head, later], { subscribed: true }));
  assert.deepEqual(server.calls.map(call => call.unit.name), ['head'], 'the range is not asked before it begins');
  server.answer('head', { hour: 50 });
  await settled();
  assert.deepEqual(target.proof()?.proof, [50, 99], 'the sibling’s proof through 200 does not cover the bundle past the hour the range begins');
  assert.equal(target.view().bundle?.panels.has('later'), false);
  advance(99);
  assert.equal(server.calls.length, 1);
  advance(100);
  await settled();
  assert.deepEqual(server.calls.map(call => call.unit.name), ['head', 'later'], 'once applicable, the range is asked; the head’s request is unchanged');
  assert.equal(target.view().bundle?.panels.has('later'), false, 'while it is pending, the bundle proven before it began is held');
  server.answer('later', { rows: [] });
  await settled();
  assert.deepEqual([target.proof()?.hour, target.proof()?.proof, target.proof()?.bundle === target.view().bundle], [100, [100, 100], true]);
  assert.equal(target.view().bundle?.panels.has('later'), true);
  target.dispose();

  const idle = new ReadTarget(fakeClock(50).clock, fakeServer().load);
  idle.configure(followConfig([later]));
  assert.equal(idle.view().kind, 'outside', 'with nothing followed and nothing to ask, the widget is outside');
  idle.dispose();
});

test('a lagging bundle is ordinary while playing, but at rest both proof bounds must hold the displayed hour (host R2-S6, R3-S1)', async () => {
  const entries = [aggregate('history', 0, hour => [hour, hour + 20])];
  const time = fakeClock(100);
  const server = fakeServer();
  const target = new ReadTarget(time.clock, server.load);
  target.configure(followConfig(entries));
  server.answer('history');
  await settled();
  const shown = target.view().bundle;
  assert.ok(shown);
  time.advance(150);
  await settled();
  assert.equal(server.calls.length, 2, 'leaving the proof asks the clock’s hour');
  assert.equal(target.view().bundle, shown, 'while the clock moves, the last complete bundle stays shown, dated by its own proof');
  assert.deepEqual(target.proof()?.proof, [100, 120]);
  assert.equal(target.view().held, false);
  time.pause();
  assert.equal(target.view().held, true, 'paused at 150, a bundle proven 100 to 120 is held as not current');
  server.answer('history');
  await settled();
  assert.deepEqual(target.proof()?.proof, [150, 170], 'it settles when that reply lands');
  assert.equal(target.view().held, false);

  const behind = fakeClock(100);
  const late = fakeServer();
  const stopped = new ReadTarget(behind.clock, late.load);
  stopped.configure(followConfig([aggregate('flat')]));
  late.answer('flat');
  await settled();
  behind.advance(101);
  behind.pause();
  assert.equal(stopped.view().held, true, 'R3-S1: paused at 101 with a proof ending at 100, the bundle is not passed off as current');
  assert.equal(late.calls.length, 2, 'and the hour it stopped at is asked');
  stopped.dispose();
  target.dispose();
});

test('a re-pin never shows a held bundle dated after the new pin (R3-S1)', async () => {
  const entries = [point('head', { channel: 'followed' })];
  const server = fakeServer();
  const target = new ReadTarget(fakeClock(0, false).clock, server.load);
  target.configure(pinConfig(entries, 500));
  server.answer('head', { hour: 500 });
  await settled();
  assert.equal(target.view().bundle?.hour, 500);
  target.configure(pinConfig(entries, 100));
  assert.equal(target.view().bundle, null, 're-pinned from 500 to 100, the hour-500 bundle is withheld while 100 loads, so its hour is never passed back as the pin');
  assert.equal(target.view().kind, 'hold');
  server.answer('head', { hour: 100 });
  await settled();
  assert.equal(target.view().bundle?.hour, 100);
  target.configure(pinConfig(entries, 300));
  assert.equal(target.view().bundle?.hour, 100, 'a held bundle dated before the new pin may stay while it loads');
  assert.equal(target.view().held, true);
  target.dispose();
});

test('two units with staggered replies settle every bundle: nothing advances until the target settles (R3-S3)', async () => {
  const entries = [point('a'), point('b')];
  const time = fakeClock(10);
  const server = fakeServer();
  const target = new ReadTarget(time.clock, server.load);
  const counts = watched(target);
  target.configure(followConfig(entries));
  const hours: number[] = [];
  for (let tick = 0; tick < 20; tick++) {
    const first = tick % 2 ? 'a' : 'b';
    const second = tick % 2 ? 'b' : 'a';
    server.answer(first);
    await settled();
    time.advance(time.clock.now() + 1);
    await settled();
    assert.equal(server.pending.length, 1, `tick ${tick}: the unit that answered is not advanced while its sibling is pending`);
    server.answer(second);
    await settled();
    hours.push(target.proof()!.hour);
    assert.equal(target.inFlight, 2, `tick ${tick}: once the bundle settles, both units are asked for the clock’s hour`);
  }
  assert.deepEqual(hours, Array.from({ length: 20 }, (_, at) => 10 + at), 'every target completes a bundle, one hour behind the clock');
  assert.equal(counts.bundles.size, 20);
  assert.equal(server.calls.length, 42, 'one request per unit per target, never more');
  target.dispose();
});

test('a failed read is asked again once a seek leaves its hour, even with the same request (R2-S8, R3-S4)', async () => {
  const head = [point('head')];
  const time = fakeClock(50);
  const server = fakeServer();
  const target = new ReadTarget(time.clock, server.load);
  target.configure(followConfig(head));
  server.refuse('head');
  await settled();
  assert.equal(target.view().kind, 'failed');
  target.retry();
  assert.equal(server.calls.length, 2, 'retry asks the failed request again');
  server.refuse('head', new Error('lost again'));
  await settled();
  time.advance(80);
  await settled();
  assert.equal(server.calls.length, 3, 'a request naming the new hour is asked');
  assert.equal(target.view().kind, 'hold', 'the old failure does not stand for the new hour');
  target.dispose();

  const fixed = [range('fixed', 0, 500)];
  const later = fakeClock(700);
  const ranges = fakeServer();
  const ended = new ReadTarget(later.clock, ranges.load);
  ended.configure(followConfig(fixed));
  ranges.refuse('fixed');
  await settled();
  later.advance(701);
  await settled();
  assert.equal(ranges.calls.length, 1, 'playing on does not re-ask the same failed request every hour');
  later.seekTo(600);
  assert.equal(ranges.calls.length, 2, 'a seek to 600 asks the same request once more');
  assert.equal(ranges.calls[0]!.unit.key, ranges.calls[1]!.unit.key);
  ranges.answer('fixed', { rows: [] });
  await settled();
  assert.equal(ended.view().kind, 'complete');
  ended.dispose();
});

test('an identical request at a new target settles from the reply it has, without asking again (R2-S6, R3-S10)', async () => {
  const entries = [range('a', 0, 500), range('b', 100, 500)];
  const time = fakeClock(700);
  const server = fakeServer();
  const target = new ReadTarget(time.clock, server.load);
  target.configure(followConfig(entries));
  server.answer('a', { rows: [] });
  server.answer('b', { rows: [] });
  await settled();
  assert.equal(server.pending.length, 0, 'both flights are settled before the seek');
  assert.deepEqual(target.proof()?.proof, [500, Infinity], 'a finished range is proven from its last hour on');
  const shown = target.view().bundle;
  time.seekTo(600);
  await settled();
  assert.equal(target.target, 600);
  assert.equal(server.calls.length, 2, 'the same requests at 600 are answered by the replies already held');
  assert.equal(target.view().bundle, shown, 'and the bundle is the same object');
  target.dispose();
});

test('removing or adding a unit never cancels or re-asks a sibling in flight (R2-S11)', async () => {
  const a = aggregate('a');
  const b = aggregate('b');
  const c = aggregate('c');
  const { reads, calls } = fakeReads();
  reads.sync(following([a, b], 50), true);
  assert.equal(calls.length, 2);
  reads.sync(following([a], 50), true);
  assert.equal(calls[1]!.signal.aborted, true, 'the removed unit’s read is cancelled');
  assert.equal(calls[0]!.signal.aborted, false, 'its sibling’s read survives');
  reads.sync(following([a, c], 50), true);
  assert.equal(calls.length, 3, 'only the added unit is asked');
  assert.equal(calls[0]!.signal.aborted, false);
  calls[0]!.resolve({ rows: [] });
  await settled();
  assert.equal(reads.results.has('a'), true);
  assert.equal(reads.inFlight, 1);
  const slots = (reads as unknown as { broker: { slots: Map<string, unknown> } }).broker.slots;
  assert.equal(slots.has('a'), false, 'R3-S12: a settled flight releases its broker slot, so removing its unit later holds nothing');
  reads.dispose();
});

test('each unit is asked at its own track hour and proofs are joined on the common clock (R2-S12)', () => {
  const shifted = aggregate('shifted', 24, hour => [hour, hour + 6]);
  const plain = aggregate('plain', 0, hour => [hour, hour + 2]);
  const units = following([shifted, plain], 10);
  assert.equal(unit(units, 'shifted').anchor, 34, 'the shifted entry asks its own hour');
  assert.equal((unit(units, 'shifted').spec!.request.params as { lastHour: number }).lastHour, 34);
  assert.equal(unit(units, 'plain').anchor, 10);
  const joined = settleFollowing(units, answered(NONE, [unit(units, 'shifted'), {}], [unit(units, 'plain'), {}]), 10);
  assert.deepEqual(joined.kind === 'complete' && joined.bundle.proof, [10, 12]);
  assert.equal(joined.kind === 'complete' && joined.bundle.hour, 10);
});

test('each entry uses its own branch, gate and recording end, and its panel dates are checked on its own clock (R3-S8)', () => {
  const longer = { ...source, id: 'other', branches: [{ id: '2', firstHour: 0, lastHour: 1000 }] } as unknown as DashboardSource;
  const shifted: SourceEntry = { name: 'shifted', source: longer, branch: '2', offset: 24, read: point('shifted', { channel: 'panel', exactProduct: undefined }).read };
  const plain = aggregate('plain');
  const short = { branch: { firstHour: 0, lastHour: 100 }, effectiveEndHour: 100, gate };
  const long = { branch: { firstHour: 0, lastHour: 1000 }, effectiveEndHour: 1000, gate };
  const units = askUnits([shifted, plain], { following: true, trackOffset: 0, content: 0, hour: 90, playback: entry => (entry.branch === '2' ? long : short) });
  assert.equal(unit(units, 'shifted').anchor, 114, 'common hour 90 asks the longer track at its own 114, not the first track’s end');
  const joined = settleFollowing(units, answered(NONE, [unit(units, 'shifted'), { hour: 114 }], [unit(units, 'plain'), {}]), 90);
  assert.deepEqual(joined.kind === 'complete' && joined.bundle.proof, [90, 90], 'nor is it proven forever because the first track has ended');
  assert.deepEqual(joined.kind === 'complete' && joined.bundle.shifts, { shifted: 24 });
  const view = createBoundedReadView({ hour: 10, panels: new Map([['shifted', { hour: 34 }]]), panelShifts: { shifted: 24 } });
  assert.deepEqual(view.panel('shifted'), { hour: 34 }, 'a panel dated 34 on its own clock is hour 10 on the common one');
  assert.throws(() => createBoundedReadView({ hour: 10, panels: new Map([['shifted', { hour: 35 }]]), panelShifts: { shifted: 24 } }).panel('shifted'), FutureDataError);
});

test('a builder that returns nothing is re-run only when its inputs can change (R2-S9)', () => {
  const builtPoint = point('built', { buildExactParams: (base, hour) => (hour < 60 ? null : { ...base, hour }) });
  const idlePoint = unit(following([builtPoint], 50), 'built');
  assert.equal(idlePoint.spec, null);
  assert.deepEqual(idlePoint.idle, [50, 50], 'an hour-dependent point builder is asked again at the next hour, not never');
  const builtRange = range('built', 100, 500, { buildSpanParams: () => null });
  assert.deepEqual(unit(following([builtRange], 700), 'built').idle, [500, Infinity], 'after a fixed range ends its inputs are fixed, so it is not re-run every hour');
  assert.deepEqual(unit(following([builtRange], 300), 'built').idle, [300, 300]);
});

test('a hundred quiet playback hours ask nothing, notify nothing and cross no boundary (R3-S10)', async () => {
  const entries = [
    point('head', { provenInterval: (_data, hour) => [hour, 10_000] }),
    aggregate('history', 0, hour => [hour, 10_000]),
    range('later', 5000, 6000),
  ];
  const time = fakeClock(0);
  const server = fakeServer(() => ({}));
  const target = new ReadTarget(time.clock, server.load);
  const counts = watched(target);
  target.configure(followConfig(entries));
  await settled();
  const before = { views: counts.views, proofs: counts.proofs };
  for (let hour = 1; hour <= 100; hour++) {
    time.advance(hour);
    await settled();
  }
  assert.equal(server.calls.length, 2, 'one request per applicable unit, none per quiet hour');
  assert.deepEqual([counts.views - before.views, counts.proofs - before.proofs, time.crossings], [0, 0, 0], 'no view change (React commit), no proof change (lane message) and no boundary crossing');
  assert.deepEqual(target.proof()?.proof, [0, 4999]);
  target.dispose();
});

const surface = (): PageSurface<ImageBitmap> => ({ install() {}, clear() {} });

async function playHundredHours(entries: readonly SourceEntry[]) {
  const drawn: LaneView = { paint: input => ({ surfaces: input.binding.surfaces.map(() => ({ source: { width: 1, height: 1 }, content: 'drawn' })), fields: {}, motion: false }) };
  const lane = inProcess({ views: kind => kind === 'drawn' ? drawn : undefined });
  const time = fakeClock(0);
  let replies = 0;
  const server = fakeServer(asked => {
    replies++;
    return { asked: asked.key, rows: Array.from({ length: 50 }, (_, at) => ({ tag: `T${at}`, value: at })) };
  });
  const target = new ReadTarget(time.clock, server.load);
  const counts = watched(target);
  try {
    const id = lane.pipeline.acquire('widget|0', { offset: 0, pin: null });
    lane.pipeline.present(id, 'drawn');
    lane.pipeline.bind(id, [surface()], { surfaces: [{ width: 1, height: 1, dpr: 1 }] }, {}, { family: 'test', kind: 'test', options: {} });
    const generation = lane.pipeline.binding(id);
    const setup = lane.posted.length;
    const sent = new StaticSupply(change => lane.pipeline.supply(id, change));
    let shown: FamilyStaticSources | null = null;
    const datedNow = () => boundDated(shown, ownProof(shown, target.proof()), false, false, null) ?? null;
    target.subscribe(() => {
      const bundle = target.view().bundle;
      if (bundle === (shown?.stamp ?? null)) return;
      shown = bundle ? { stamp: bundle, proof: bundle.proof, panels: Object.fromEntries(bundle.panels), hour: bundle.hour, follows: true } : null;
      sent.statics(shown, datedNow());
    });
    target.subscribeProof(() => sent.date(shown, datedNow()));
    lane.pipeline.supply(id, { identity: { countries: Array.from({ length: 200 }, (_, at) => `T${at}`) } });
    target.configure(followConfig(entries));
    await settled();
    for (let frame = 1; frame <= 100; frame++) {
      time.advance(frame * 24);
      await settled();
      await settled();
    }
    const posted = lane.posted.slice(setup);
    const statics = posted.filter(item => item.type === 'supply' && 'statics' in item.message);
    const dated = posted.filter(item => item.type === 'supply' && 'dated' in item.message && !('statics' in item.message));
    const named = new Set<number>();
    const collect = (revision: unknown): void => {
      if (typeof revision === 'number') named.add(revision);
      else if (revision && typeof revision === 'object') for (const inner of Object.values(revision)) collect(inner);
    };
    for (const item of statics) collect((item.message as { statics?: { revisions?: unknown } }).statics?.revisions);
    const payloads = posted.filter(item => item.type === 'payloads' && ((item.message as { put?: readonly (readonly [number, unknown])[] }).put ?? []).some(([id]) => named.has(id)));
    return {
      replies,
      requests: server.calls.length,
      notifications: counts.views,
      bundles: counts.bundles.size,
      identities: posted.filter(item => item.type === 'supply' && typeof (item.message as { identity?: unknown }).identity === 'number').length,
      staticBytes: payloads.reduce((sum, item) => sum + item.bytes, 0),
      staticMessages: statics.length,
      datedMessages: dated.length,
      datedBytes: Math.max(0, ...dated.map(item => item.bytes)),
      generationKept: lane.pipeline.binding(id) === generation,
      replyBytes: Math.max(...payloads.map(item => item.bytes)),
    };
  } finally {
    target.dispose();
    lane.close();
  }
}

test('Class N: a hundred hours at 24 hours a frame cost what the answers changed (binds and commits are counted on a mounted host in host.dom.test.ts)', async () => {
  const constant = await playHundredHours([aggregate('factions', 0, undefined, { buildSpanParams: params => params })]);
  assert.deepEqual([constant.requests, constant.replies, constant.bundles, constant.staticMessages, constant.identities], [1, 1, 1, 1, 1],
    'a request that never names the hour, with no proof: one request, one bundle, one statics message and one identity snapshot');
  assert.ok(constant.notifications <= 2, `view notifications bounded by 1 + changed answers (${constant.notifications})`);
  assert.ok(constant.datedBytes <= 80, `each proof move is a dated message of ${constant.datedBytes} bytes, never the panels`);
  assert.ok(constant.datedMessages <= 101);
  assert.equal(constant.generationKept, true, 'no supply bumps the binding generation');

  for (const [name, entries] of [
    ['hour-named request, no proof', [aggregate('relations')]],
    ['one-day proof', [point('focus', { provenInterval: (_data, hour) => [hour, hour + 23] })]],
  ] as const) {
    const played = await playHundredHours(entries);
    assert.ok(played.requests <= played.replies + 1, `${name}: requests (${played.requests}) are bounded by replies (${played.replies}): one outstanding target, lag instead of a queue`);
    assert.ok(played.notifications <= played.replies + 1, `${name}: view notifications (${played.notifications}) bounded by 1 + changed answers (${played.replies})`);
    assert.ok(played.staticMessages <= played.replies, `${name}: statics messages (${played.staticMessages}) bounded by changed answers`);
    assert.ok(played.staticBytes <= played.replies * played.replyBytes, `${name}: statics bytes bounded by changed answers × reply size`);
    assert.deepEqual([played.identities, played.generationKept], [1, true], `${name}: one identity snapshot, the generation kept`);
  }
});

type Proven = { readonly lo: number; readonly hi: number; readonly tag: string };

function pairRig(faults: { readonly paint?: (tag: string) => boolean; readonly install?: () => void } = {}) {
  let paints = 0;
  const drawn: LaneView = {
    paint: input => {
      const main = (input.model as { staticPanels?: { main?: Proven } }).staticPanels?.main;
      paints++;
      if (main && faults.paint?.(main.tag)) throw new Error(`a fault painting ${main.tag}`);
      return { surfaces: input.binding.surfaces.map(() => ({ source: { width: 1, height: 1 }, content: main?.tag ?? 'none' })), fields: { main: { value: main ?? null, content: main?.tag ?? 'none' } }, motion: false };
    },
  };
  const lane = inProcess({ views: kind => kind === 'drawn' ? drawn : undefined });
  lane.clock.setPlaying(false);
  const id = lane.pipeline.acquire('widget|0', { offset: 0, pin: null, owner: 'X' });
  lane.pipeline.present(id, 'drawn');
  lane.pipeline.bind(id, [faults.install ? { install: faults.install, clear() {} } : surface()], { surfaces: [{ width: 1, height: 1, dpr: 1 }] }, {}, { family: 'test', kind: 'test', options: {} });
  const sent = new StaticSupply(change => lane.pipeline.supply(id, change));
  const sources = (bundle: { readonly panels: ReadonlyMap<string, unknown>; readonly proof: readonly [number, number] | null; readonly hour: number }): FamilyStaticSources =>
    ({ stamp: bundle, proof: bundle.proof, panels: Object.fromEntries(bundle.panels), hour: bundle.hour, follows: true });
  const dated = (shown: FamilyStaticSources | null, proof: Parameters<typeof ownProof>[1]) => boundDated(shown, ownProof(shown, proof), false, false, null) ?? null;
  const wake = async (hour: number) => {
    if (lane.clock.getHour() !== hour) lane.clock.seek(hour);
    for (let round = 0; round < 6; round++) await settled();
    await lane.settle(() => false, 10);
  };
  const outsideOwnStamp = () => {
    const view = lane.pipeline.view(id);
    if (!view || view.mode !== 'picture' || view.dim) return null;
    const main = (view.fields?.main ?? null) as Proven | null;
    if (!main) return `a current picture with no panels at ${view.h}`;
    return view.h! < main.lo || (view.through ?? view.h!) > main.hi ? `${main.tag} (proven ${main.lo}..${main.hi}) accepted as current at ${view.h}..${view.through}` : null;
  };
  const current = () => {
    const view = lane.pipeline.view(id);
    return view && view.mode === 'picture' && !view.dim ? (view.fields?.main as Proven | null)?.tag ?? null : null;
  };
  const failed = () => lane.pipeline.view(id)?.mode === 'failed';
  return { lane, id, sent, sources, dated, wake, outsideOwnStamp, current, failed, paints: () => paints, close: () => lane.close() };
}

function proven(tag: string, lo: number, hi: number) {
  return { panels: new Map([['main', { lo, hi, tag }]]), proof: [lo, hi] as const, hour: lo };
}

test('class PAIR, proof and panels: a reply’s proof never dates the panels before it, in either order, with a worker wake between (R4-S1)', async () => {
  for (const order of ['proof first', 'panels first'] as const) {
    const rig = pairRig();
    try {
      const a = rig.sources(proven('A', 100, 100));
      rig.sent.statics(a, rig.dated(a, null));
      await rig.wake(100);
      assert.equal(rig.current(), 'A');
      const bBundle = proven('B', 101, 101);
      const b = rig.sources(bBundle);
      const bProof = { bundle: bBundle, hour: 101, proof: [101, 101] as const };
      if (order === 'proof first') {
        rig.sent.date(a, rig.dated(a, bProof));
        rig.lane.pipeline.supply(rig.id, { stamp: revisionOf(bBundle), dated: { h: 101, c: 101 } });
        await rig.wake(101);
        assert.equal(rig.outsideOwnStamp(), null, `${order}: the hour-101 proof does not date A`);
        assert.notEqual(rig.current(), 'A');
        rig.sent.statics(b, rig.dated(b, bProof));
      } else {
        rig.sent.statics(b, rig.dated(b, null));
        await rig.wake(101);
        assert.equal(rig.outsideOwnStamp(), null);
        rig.sent.date(b, rig.dated(b, { ...bProof, proof: [101, 130] }));
      }
      await rig.wake(101);
      assert.equal(rig.outsideOwnStamp(), null, order);
      assert.equal(rig.current(), 'B', `${order}: B installs at 101 once its panels and proof are both there`);
    } finally { rig.close(); }
  }
});

test('class PAIR, dates: a date for another stamp is dropped whichever arrives first', async () => {
  for (const order of ['stale date after new panels', 'new date before its panels'] as const) {
    const rig = pairRig();
    try {
      const aBundle = proven('A', 100, 100);
      const a = rig.sources(aBundle);
      rig.sent.statics(a, rig.dated(a, null));
      await rig.wake(100);
      const b = rig.sources(proven('B', 120, 120));
      if (order === 'stale date after new panels') {
        rig.sent.statics(b, rig.dated(b, null));
        rig.lane.pipeline.supply(rig.id, { stamp: revisionOf(aBundle), dated: { h: 100, c: 140 } });
      } else {
        rig.lane.pipeline.supply(rig.id, { stamp: revisionOf(b.stamp), dated: { h: 120, c: 120 } });
        await rig.wake(120);
        assert.equal(rig.outsideOwnStamp(), null, `${order}: A is not dated 120`);
        rig.sent.statics(b, rig.dated(b, null));
      }
      await rig.wake(120);
      assert.equal(rig.outsideOwnStamp(), null, order);
      assert.equal(rig.current(), 'B', order);
      await rig.wake(130);
      assert.equal(rig.current(), null, `${order}: B is not extended by A’s date`);
    } finally { rig.close(); }
  }
});

test('class PAIR, owner: the previous owner’s panels never paint under the new owner, in either order (R4-S2)', async () => {
  for (const order of ['owner first', 'panels first'] as const) {
    const rig = pairRig();
    try {
      const a = rig.sources(proven('A', 100, 100));
      rig.sent.statics(a, rig.dated(a, null));
      await rig.wake(100);
      assert.equal(rig.current(), 'A');
      const b = rig.sources(proven('B', 100, 100));
      if (order === 'owner first') {
        rig.lane.pipeline.retime(rig.id, { owner: 'Y' });
        await rig.wake(100);
        assert.notEqual(rig.current(), 'A', `${order}: X’s panels are cleared under Y`);
        assert.equal(rig.outsideOwnStamp(), null);
        rig.sent.statics(b, rig.dated(b, null));
      } else {
        rig.sent.statics(b, rig.dated(b, null));
        rig.lane.pipeline.retime(rig.id, { owner: 'Y' });
        await rig.wake(100);
        assert.equal(rig.current(), null, `${order}: panels sent before the owner changed are cleared with it`);
        rig.sent.statics(b, rig.dated(b, null));
      }
      await rig.wake(100);
      assert.equal(rig.current(), 'B', `${order}: Y’s panels install after the binding that names Y`);
    } finally { rig.close(); }
  }
});

test('class PAIR, span and inputs: the same replies under a new selected span are a new stamp over the same panel map; a stale view is withheld (R4-S2, R4-S3)', async () => {
  const fixed = aggregate('main', 0, undefined, { buildSpanParams: params => params });
  const server = fakeServer(() => ({ rows: [] }));
  const target = new ReadTarget(fakeClock(100, false).clock, server.load);
  const base = pinConfig([fixed], 100) as Extract<TargetConfig, { following: false }>;
  const window = (first: number): TargetConfig => ({ ...base, inputs: `window ${first}`, pinned: { ...base.pinned, timeMode: 'range', pinHour: null, window: { firstHour: first, lastHour: 100 } } });
  target.configure(window(0));
  await settled();
  const first = target.view().bundle!;
  assert.deepEqual(first.span, [0, 100]);
  target.configure(window(50));
  await settled();
  const second = target.view().bundle!;
  assert.deepEqual(second.span, [50, 100], 'R4-S3: the selected span moves although request and reply are the same');
  assert.notEqual(second, first, 'a new stamp');
  assert.equal(second.panels, first.panels, 'over the same panel map, so no panel is sent again');
  const view = target.view();
  assert.equal(shownView(view, 'window 50', true), view);
  assert.equal(shownView(view, 'window 0', true).bundle, null, 'R4-S2: a render whose inputs moved on withholds the view');
  assert.equal(shownView(view, 'window 0', false).kind, 'none', 'a widget with no units under its new inputs is not loading');
  target.dispose();
});

test('class PAIR, re-pin at the target: a configure for new inputs never holds the previous inputs’ bundle (R4-S2)', async () => {
  const head = point('head');
  const server = fakeServer();
  const target = new ReadTarget(fakeClock(500, false).clock, server.load);
  target.configure({ ...pinConfig([head], 500), inputs: 'pin 500' });
  server.answer('head', { hour: 500 });
  await settled();
  assert.equal(target.view().bundle?.hour, 500);
  target.configure({ ...pinConfig([head], 100), inputs: 'pin 100' });
  assert.deepEqual([target.view().inputs, target.view().bundle, target.proof()], ['pin 100', null, null], 'the 500 bundle is not held for the pin at 100');
  server.answer('head', { hour: 100 });
  await settled();
  assert.deepEqual([target.view().inputs, target.view().bundle?.hour], ['pin 100', 100]);
  target.dispose();
});

test('R5-S2, R5-S5: a widget whose paint or installation failed recovers on its newer bundle, whichever reaches the main thread first; the same bundle failing again stays failed', async () => {
  for (const where of ['paint', 'install'] as const) {
    for (const order of ['failure first', 'bundle first'] as const) {
      let installFaults = 0;
      const rig = pairRig(where === 'paint'
        ? { paint: tag => tag === 'A' }
        : { install: () => { if (installFaults > 0) { installFaults--; throw new Error('an installation fault'); } } });
      try {
        if (order === 'bundle first') rig.lane.holdBack(message => message.type === (where === 'paint' ? 'widget-failed' : 'presentation'));
        if (where === 'install') installFaults = 1;
        const a = rig.sources(proven('A', 100, 200));
        rig.sent.statics(a, rig.dated(a, null));
        await rig.wake(100);
        if (order === 'failure first') assert.equal(rig.failed(), true, `${where}, ${order}: A failed before B is supplied`);
        const b = rig.sources(proven('B', 100, 200));
        rig.sent.statics(b, rig.dated(b, null));
        await rig.wake(100);
        rig.lane.release();
        await rig.wake(100);
        assert.ok(await rig.lane.settle(() => rig.current() === 'B'), `${where}, ${order}: paused on B, the widget draws B (${rig.lane.pipeline.view(rig.id)?.mode})`);
      } finally { rig.close(); }
    }
  }
  const rig = pairRig({ paint: () => true });
  try {
    rig.lane.holdBack(message => message.type === 'widget-failed');
    const a = rig.sources(proven('A', 100, 200));
    rig.sent.statics(a, rig.dated(a, null));
    await rig.wake(100);
    const b = rig.sources(proven('B', 100, 200));
    rig.sent.statics(b, rig.dated(b, null));
    await rig.wake(100);
    rig.lane.release();
    await rig.wake(100);
    await rig.lane.settle(() => false, 20);
    assert.equal(rig.failed(), true, 'B failing too keeps the failure');
    assert.ok(rig.paints() <= 2, `B is retried once, not in a loop (${rig.paints()} paints)`);
  } finally { rig.close(); }
});

test('sweep-2 P0: a seek back never hands the worker a panel dated after the hour it draws, and a widget that failed recovers on its next bundle', async () => {
  let throwOnce = false;
  const bounded: LaneView = {
    paint: input => {
      const statics = (input.model as { staticPanels?: Readonly<Record<string, unknown>> }).staticPanels ?? {};
      if (throwOnce) {
        throwOnce = false;
        throw new Error('a projection fault');
      }
      const view = createBoundedReadView({ hour: input.hour, panels: new Map(Object.entries(statics)) });
      const main = view.panel<{ hour: number }>('main');
      return { surfaces: input.binding.surfaces.map(() => ({ source: { width: 1, height: 1 }, content: `main:${main?.hour}` })), fields: { main: { value: main?.hour ?? null, content: String(main?.hour) } }, motion: false };
    },
  };
  const lane = inProcess({ views: kind => kind === 'drawn' ? bounded : undefined });
  const id = lane.pipeline.acquire('widget|0', { offset: 0, pin: null });
  lane.pipeline.present(id, 'drawn');
  lane.pipeline.bind(id, [surface()], { surfaces: [{ width: 1, height: 1, dpr: 1 }] }, {}, { family: 'test', kind: 'test', options: {} });
  const cache = new Map<string, unknown>();
  const time = fakeClock(100);
  const target = new ReadTarget(time.clock, async asked => {
    const data = { hour: asked.anchor };
    cache.set(asked.key!, data);
    return data;
  }, asked => cache.get(asked.key!));
  const sent = new StaticSupply(change => lane.pipeline.supply(id, change));
  let shown: FamilyStaticSources | null = null;
  const datedNow = () => boundDated(shown, ownProof(shown, target.proof()), false, false, null) ?? null;
  const render = () => {
    const bundle = target.view().bundle;
    if (bundle === (shown?.stamp ?? null)) return;
    shown = bundle ? { stamp: bundle, proof: bundle.proof, panels: Object.fromEntries(bundle.panels), hour: bundle.hour, follows: true } : null;
    sent.statics(shown, datedNow());
  };
  target.subscribe(() => { setImmediate(render); });
  target.subscribeProof(() => sent.date(shown, datedNow()));
  const failed = () => lane.pipeline.view(id)?.mode === 'failed';
  const drawn = () => lane.pipeline.view(id)?.fields?.main;
  try {
    target.configure(followConfig([point('main', { provenInterval: (_data, hour) => [hour, hour + 23] })]));
    lane.clock.seek(100);
    lane.clock.setPlaying(false);
    for (let hour = 100; hour <= 500; hour += 24) {
      time.advance(hour);
      lane.clock.seek(hour);
      for (let round = 0; round < 4; round++) await settled();
    }
    assert.ok(await lane.settle(() => drawn() === 484), `playing forward draws each bundle (${String(drawn())})`);
    time.seekTo(196);
    lane.clock.seek(196);
    for (let round = 0; round < 8; round++) await settled();
    await lane.settle(() => drawn() === 196 || failed(), 100);
    assert.equal(failed(), false, `the seek back drew no panel dated after 196 (${lane.pipeline.view(id)?.reason ?? ''})`);
    assert.equal(drawn(), 196, 'and draws the hour-196 bundle the cache answered');

    throwOnce = true;
    time.seekTo(292);
    lane.clock.seek(292);
    assert.ok(await lane.settle(() => failed()), 'a projection fault fails the widget');
    time.seekTo(388);
    lane.clock.seek(388);
    assert.ok(await lane.settle(() => drawn() === 388 && !failed()), `and its next bundle draws again (${lane.pipeline.view(id)?.mode})`);
  } finally {
    target.dispose();
    lane.close();
  }
});

test('a failed sibling never lets the clock re-ask a unit still pending: requests stay bounded by replies (R4-S5)', async () => {
  const tile = range('tile', 0, 50);
  const now = aggregate('now');
  const time = fakeClock(100);
  const server = fakeServer();
  const target = new ReadTarget(time.clock, server.load);
  target.configure(followConfig([tile, now]));
  assert.deepEqual(server.calls.map(call => call.unit.name).sort(), ['now', 'tile']);
  server.refuse('tile');
  await settled();
  assert.equal(target.view().failure?.name, 'tile');
  for (let hour = 101; hour <= 110; hour++) {
    time.advance(hour);
    await settled();
  }
  assert.equal(server.calls.length, 2, 'ten advancing frames while `now` is unanswered ask nothing more');
  assert.equal(server.pending.filter(call => call.unit.name === 'now').length, 1, 'and the pending read is not cancelled');
  server.answer('now', {});
  await settled();
  assert.ok(server.calls.length <= 3, `once it answers, at most one new ask for the clock's hour (${server.calls.length})`);
  time.seekTo(200);
  await settled();
  assert.ok(server.calls.some(call => call.unit.name === 'tile' && server.calls.indexOf(call) > 1), 'an explicit seek still asks the failed read again');
  target.dispose();
});

test('a pinned unit uses its own branch and coverage gate, so a point at +24 on a longer track is asked there (R4-S7)', () => {
  const longer = { ...source, id: 'other', branches: [{ id: '2', firstHour: 0, lastHour: 1000 }] } as unknown as DashboardSource;
  const shifted: SourceEntry = { name: 'shifted', source: longer, branch: '2', offset: 24, read: point('shifted').read };
  const plain = point('plain');
  const endsAt = (last: number): FrozenGate => ({ ...gate, recorded: (hour: number | null | undefined) => (typeof hour === 'number' && hour <= last ? hour : null) as never });
  const short = { branch: { firstHour: 0, lastHour: 100 }, gate: endsAt(100) };
  const long = { branch: { firstHour: 0, lastHour: 1000 }, gate: endsAt(1000) };
  const units = askUnits([shifted, plain], {
    following: false,
    trackOffset: 0,
    content: 0,
    displayHour: 90,
    pinned: { timeMode: 'pin', branch: short.branch, window: { firstHour: 90, lastHour: 90 }, pinHour: 90, gate: short.gate },
    track: entry => (entry.branch === '2' ? long : short),
  });
  assert.equal(unit(units, 'shifted').anchor, 114, 'the secondary is pinned at its own 114, recorded on its own track');
  assert.ok(unit(units, 'shifted').spec, 'and is asked, not gated away by the primary track ending at 100');
  assert.equal(unit(units, 'plain').anchor, 90);

  const aligned: SourceEntry = { ...shifted, name: 'aligned', offset: 0 };
  const gapAt90 = { branch: long.branch, gate: { ...gate, recorded: (hour: number | null | undefined) => (typeof hour === 'number' && hour !== 90 ? hour : null) as never } };
  const zero = askUnits([plain, aligned], {
    following: false,
    trackOffset: 0,
    content: 0,
    displayHour: 90,
    pinned: { timeMode: 'pin', branch: short.branch, window: { firstHour: 90, lastHour: 90 }, pinHour: 90, gate: short.gate },
    track: entry => (entry.branch === '2' ? gapAt90 : short),
  });
  assert.ok(unit(zero, 'plain').spec, 'the primary is recorded at 90 and asked');
  assert.equal(unit(zero, 'aligned').spec, null, 'R5-S3: at zero shift the secondary is gated on its own track, which did not record 90');
});

test('FamilySourceBroker.runSlot releases settled slots on both resolve and reject', async () => {
  const broker = new FamilySourceBroker();
  let gotValue = 0;
  let gotError: unknown = null;
  broker.runSlot(
    'ok-slot',
    'k1',
    () => Promise.resolve(42),
    value => { gotValue = value; },
  );
  broker.runSlot(
    'err-slot',
    'k2',
    () => Promise.reject(new Error('boom')),
    () => {},
    error => { gotError = error; },
  );
  assert.equal(broker.activeSlotCount, 2);
  await settled();
  assert.equal(gotValue, 42);
  assert.ok(gotError instanceof Error && gotError.message === 'boom');
  assert.equal(broker.activeSlotCount, 0, 'settled slots are removed immediately after resolving or rejecting');
  assert.equal(broker.activeRequestCount, 0);
});
