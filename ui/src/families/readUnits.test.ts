import assert from 'node:assert/strict';
import test from 'node:test';
import type { DashboardSource } from '../dashboard/client';
import { inProcess } from '../dashboard/follow/inProcess.fixture.ts';
import type { PageSurface } from '../dashboard/follow/page.ts';
import type { LaneView } from '../draw/lane';
import { createBoundedReadView, FutureDataError } from './boundedView.ts';
import type { CompiledFollowSubscription, SourceEntry, SourcePlan, SourceTimeShape } from './plan.ts';
import {
  askUnits,
  ReadTarget,
  settleUnits,
  unitEntries,
  UnitReads,
  type ReadUnit,
  type TargetClock,
  type TargetConfig,
  type UnitResult,
} from './readUnits.ts';
import type { FrozenGate } from './sources.ts';

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
  return { following: true, entries, active: true, content: 0, trackOffset: 0, subscribed: false, playback: () => PLAYBACK, ...extra };
}
function pinConfig(entries: readonly SourceEntry[], hour: number): TargetConfig {
  return {
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
  assert.deepEqual(target.proof(), { hour: 100, proof: [100, 100] });
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
    let supplied: unknown = null;
    target.subscribe(() => {
      const bundle = target.view().bundle;
      if (bundle === supplied) return;
      supplied = bundle;
      lane.pipeline.supply(id, { statics: bundle ? { staticPanels: Object.fromEntries(bundle.panels) } : null });
    });
    target.subscribeProof(() => {
      const proof = target.proof();
      lane.pipeline.supply(id, { dated: proof ? { h: proof.proof[0], c: proof.proof[1] } : null });
    });
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
    return {
      replies,
      requests: server.calls.length,
      notifications: counts.views,
      bundles: counts.bundles.size,
      identities: posted.filter(item => item.type === 'identity').length,
      staticBytes: statics.reduce((sum, item) => sum + item.bytes, 0),
      staticMessages: statics.length,
      datedMessages: dated.length,
      datedBytes: Math.max(0, ...dated.map(item => item.bytes)),
      generationKept: lane.pipeline.binding(id) === generation,
      replyBytes: Math.max(...statics.map(item => item.bytes)),
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
