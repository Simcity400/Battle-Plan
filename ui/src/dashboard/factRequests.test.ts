import assert from 'node:assert/strict';
import test from 'node:test';
import { FAILED_FACT, FactRequests, HourStore, MAX_READING, nameKey, readFacts, RETRY_MOST_MS, retryDelay } from './countryFacts.ts';
import type { FactClock, FactRead, NameAt } from './countryFacts';
import type { DashboardClient, ProductRequest } from './client';
import { BROWSER_BRIDGE_BUSY, CAPACITY_REACHED, MAP_CONTROLS_BUSY, NATIVE_QUEUE_FULL, TRANSIENT_ERRORS } from '../generated/dashboard.ts';

/** Reads that finish when the test's clock passes their end, counted, with a clock
 * whose waits run as it advances. Like the desktop, at most `limit` requests are
 * taken at once (a further one is rejected at once; `limit` may change) and a
 * cancelled request frees its place at once. `others` holds places other widgets use. */
function clockedReads(duration: number, limit = Infinity) {
  let now = 0;
  const waiting: { at: number; done: () => void }[] = [];
  const timers: { at: number; run: () => void }[] = [];
  const desk = { limit, others: 0 };
  const stats = { started: 0, cancelled: 0, completed: 0, rejected: 0, active: 0, most: 0, asked: [] as string[][], hours: [] as number[] };
  const read: FactRead<string> = async (plan, signal, onChunk) => {
    for (const [hour, tags] of plan) {
      stats.started++;
      stats.asked.push([...tags]);
      stats.hours.push(hour);
      if (stats.active + desk.others >= desk.limit) { stats.rejected++; onChunk(hour, tags, null); continue; }
      stats.active++;
      stats.most = Math.max(stats.most, stats.active);
      await new Promise<void>(done => {
        const entry = { at: now + duration, done };
        waiting.push(entry);
        // A cancelled request frees its place at once.
        signal.addEventListener('abort', () => { waiting.splice(waiting.indexOf(entry), 1); stats.active--; done(); }, { once: true });
      });
      if (signal.aborted) { stats.cancelled++; return; }
      stats.active--;
      stats.completed++;
      // The recording has no fact for a country named "NONE".
      onChunk(hour, tags, new Map(tags.filter(tag => tag !== 'NONE').map(tag => [tag, `name of ${tag}`])));
    }
  };
  const clock: FactClock = {
    now: () => now,
    after: (ms, run) => { const timer = { at: now + ms, run }; timers.push(timer); return () => { const index = timers.indexOf(timer); if (index >= 0) timers.splice(index, 1); }; },
  };
  const settle = async () => { for (let turn = 0; turn < 8; turn++) await Promise.resolve(); };
  const advance = async (to: number) => {
    // Every read and wait due by `to`, in time order.
    for (;;) {
      const next = Math.min(...waiting.map(item => item.at), ...timers.map(item => item.at));
      if (!(next <= to)) break;
      now = Math.max(now, next);
      for (const entry of waiting.filter(item => item.at <= now)) { waiting.splice(waiting.indexOf(entry), 1); entry.done(); }
      await settle();
      for (const timer of timers.filter(item => item.at <= now)) { timers.splice(timers.indexOf(timer), 1); timer.run(); }
      await settle();
    }
    now = to;
    await settle();
  };
  return { read, stats, advance, clock, desk, timers };
}

const at = (tags: readonly string[]): NameAt[] => tags.map(tag => ({ hour: 100, tag }));

test('a retained exact dated queue visits demand once instead of rebuilding it after each reply', async () => {
  let answeredChecks = 0;
  class CountedStore extends HourStore<string> {
    answered(hour: number, tag: string) { answeredChecks++; return super.answered(hour, tag); }
  }
  const { read, stats, advance, clock } = clockedReads(1);
  const store = new CountedStore(64);
  const requests = new FactRequests(store, read, () => {}, clock);
  const wanted = Array.from({ length: 5000 }, (_, hour) => ['A', 'B', 'NONE'].map(tag => ({ hour, tag }))).flat();
  requests.want(wanted, Infinity, 2);
  await advance(5000);
  assert.equal(stats.completed, 10000); assert.equal(stats.most, MAX_READING);
  assert.equal(requests.running, 0); assert.equal(store.size, 5000);
  assert.deepEqual(stats.hours, Array.from({ length: 5000 }, (_, hour) => [hour, hour]).flat());
  assert.ok(stats.asked.every((tags, index) => JSON.stringify(tags) === JSON.stringify(index % 2 ? ['NONE'] : ['A', 'B'])));
  // Count actual store lookups rather than machine-dependent wall time. One
  // admission scan and one queue-drain check per pair suffice for unchanged demand.
  assert.ok(answeredChecks <= wanted.length * 2, `Unchanged demand was rechecked ${answeredChecks} times`);
  const result = requests.pickRead(wanted);
  assert.equal(result.size, wanted.length);
  assert.equal(result.get(nameKey(4999, 'A')), 'name of A'); assert.equal(result.get(nameKey(4999, 'NONE')), null);
});

test('artwork queue fills visible batches first and still completes every dated row', async () => {
  const { read, stats, advance, clock } = clockedReads(50);
  const store = new HourStore<string>(8);
  const requests = new FactRequests(store, read, () => {}, clock);
  const wanted = [{ hour: 30, tag: 'Z' }, { hour: 30, tag: 'Y' }, { hour: 30, tag: 'A' },
    { hour: 10, tag: 'old completion' }, { hour: 20, tag: 'another completion' }];
  requests.want(wanted, Infinity, 2, 'wanted');
  assert.deepEqual(stats.hours, [30, 30]);
  assert.deepEqual(stats.asked, [['Z', 'Y'], ['A']]);
  await advance(200);
  assert.deepEqual(stats.hours, [30, 30, 10, 20]);
  assert.equal(requests.pickRead(wanted).size, wanted.length);
  assert.equal(stats.cancelled, 0);
  assert.equal(requests.want(wanted, Infinity, 2, 'wanted'), 0);
});

test('configuration failures are kept while wanted and pruned when playback leaves their hours', async () => {
  let calls = 0;
  const requests = new FactRequests(new HourStore<string>(4), async (plan, _signal, done) => {
    calls++;
    await Promise.resolve();
    done(plan[0][0], plan[0][1], null, false);
  }, () => {});
  const past: NameAt[] = [];
  // Ten years of monthly looks with unconfigured folders, plus several countries
  // requested at different label hours. Failures never accumulate behind playback.
  for (let hour = 100; hour < 100 + 10 * 365 * 24; hour += 30 * 24) {
    const wanted = [{ hour, tag: 'opaque,"tag' }, { hour: 90, tag: 'old line' }];
    requests.want(wanted, 64, 64);
    await Promise.resolve();
    assert.equal(requests.want(wanted, 64, 64), 0);
    assert.equal(requests.pickRead(wanted).get(nameKey(hour, 'opaque,"tag')), FAILED_FACT);
    assert.equal(requests.pickRead(past).size, 0);
    past.push(wanted[0]);
  }
  assert.equal(calls, past.length + 1);
  assert.equal(requests.pickRead([{ hour: 90, tag: 'old line' }]).get(nameKey(90, 'old line')), FAILED_FACT);
  assert.equal(requests.retry(), 2);
  requests.cancel();
});

test('same-hour demand drops departed failures and refetches returning countries', async () => {
  const calls: string[][] = [];
  const store = new HourStore<string>(4);
  store.record(0, ['keeper'], new Map([['keeper', 'kept']]));
  const requests = new FactRequests(store, async (plan, _signal, chunk) => {
    const [hour, tags] = plan[0]; calls.push([...tags]); await Promise.resolve(); chunk(hour, tags, null, false);
  }, () => {}, { now: () => 0, after: () => () => {} });
  const retained = [{ hour: 0, tag: 'keeper' }, { hour: 0, tag: 'still failed' }];
  requests.want(retained, Infinity, 64);
  await new Promise<void>(resolve => setImmediate(resolve));
  for (let turn = 0; turn < 3; turn++) {
    const failed = Array.from({ length: 5000 }, (_, index) => ({ hour: 0, tag: `failed ${turn} ${index}` }));
    requests.want([...retained, ...failed], Infinity, 64);
    await new Promise<void>(resolve => setImmediate(resolve));
    assert.equal(requests.pickRead(failed).size, 5000);
    requests.want(retained, Infinity, 64);
    assert.equal(requests.pickRead(failed).size, 0, 'another country at the same hour cannot retain departed failures');
    assert.equal(requests.pickRead(retained).get(nameKey(0, 'still failed')), FAILED_FACT);
    assert.equal(store.get(0, 'keeper'), 'kept');
    const before = calls.length;
    requests.want([...retained, failed[0]], Infinity, 64);
    await new Promise<void>(resolve => setImmediate(resolve));
    assert.equal(calls.length, before + 1, 'returning demand refetches the discarded failure');
    requests.want(retained, Infinity, 64);
  }
  requests.cancel();
});

test('a late failure is discarded when its country leaves but its hour stays demanded', async () => {
  const reads: { done: () => void }[] = [];
  const store = new HourStore<string>(4);
  store.record(0, ['keeper'], new Map([['keeper', 'kept']]));
  const requests = new FactRequests(store, (plan, _signal, chunk) => new Promise(resolve => {
    const [hour, tags] = plan[0]; reads.push({ done: () => { chunk(hour, tags, null, false); resolve(); } });
  }), () => {}, { now: () => 0, after: () => () => {} });
  const retained = [{ hour: 0, tag: 'keeper' }], departed = [{ hour: 0, tag: 'departed' }];
  requests.want([...retained, ...departed], Infinity, 64); requests.want(retained, Infinity, 64);
  reads[0].done(); await Promise.resolve();
  assert.equal(requests.pickRead(departed).size, 0);
  requests.want([...retained, ...departed], Infinity, 64); assert.equal(reads.length, 2);
  requests.cancel();
});

test('a top-N last place alternating every 50 ms never cancels reads taking 120 ms, and each country is asked once', async () => {
  const { read, stats, advance, clock } = clockedReads(120);
  const store = new HourStore<string>(8);
  let changes = 0;
  const requests = new FactRequests(store, read, () => changes++, clock);
  const top = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I'];
  // Tenth place changes between X and Y every 50 ms for half a second.
  for (let step = 0; step < 10; step++) {
    await advance(step * 50);
    requests.want(at([...top, step % 2 ? 'Y' : 'X']), 64, 64);
  }
  await advance(1000);
  assert.equal(stats.cancelled, 0);
  // The first ten, then Y alone: nobody is asked for twice.
  assert.deepEqual(stats.asked, [[...top, 'X'], ['Y']]);
  assert.equal(stats.completed, 2);
  assert.equal(changes, 2);
  for (const tag of [...top, 'X', 'Y']) assert.equal(store.get(100, tag), `name of ${tag}`, tag);
  assert.equal(requests.pending.size, 0);
  // Asked again once read, nothing is requested.
  assert.equal(requests.want(at(top), 64, 64), 0);
});

test('reads ending in the same frame are told as one change', async () => {
  const { read, advance, clock } = clockedReads(50);
  const frames: (() => void)[] = [];
  let changes = 0;
  const requests = new FactRequests(new HourStore<string>(8), read, () => changes++, { ...clock, frame: run => { frames.push(run); } });
  requests.want([{ hour: 100, tag: 'A' }, { hour: 101, tag: 'B' }], 64, 64);
  await advance(100);
  assert.equal(changes, 0);
  assert.equal(frames.length, 1);
  for (const run of frames.splice(0)) run();
  assert.equal(changes, 1);
  assert.deepEqual(requests.pickRead([{ hour: 100, tag: 'A' }, { hour: 101, tag: 'B' }]), new Map([[nameKey(100, 'A'), 'name of A'], [nameKey(101, 'B'), 'name of B']]));
});

test('only cancel stops reads, and what they would have answered is asked for again', async () => {
  const { read, stats, advance, clock } = clockedReads(120);
  const store = new HourStore<string>(8);
  const requests = new FactRequests(store, read, () => {}, clock);
  requests.want(at(['A', 'B']), 64, 64);
  assert.deepEqual([...requests.pending].sort(), [nameKey(100, 'A'), nameKey(100, 'B')]);
  requests.cancel();
  assert.equal(requests.pending.size, 0);
  await advance(200);
  assert.equal(stats.cancelled, 1);
  assert.equal(store.answered(100, 'A'), false);
  assert.equal(requests.want(at(['A']), 64, 64), 1);
  await advance(400);
  assert.equal(store.get(100, 'A'), 'name of A');
});

test('only a completed read decides a fact: one its answer lacks is missing; a failed read decides nothing', async () => {
  const { read, advance, clock } = clockedReads(50);
  const store = new HourStore<string>(8);
  const requests = new FactRequests(store, read, () => {}, clock);
  requests.want(at(['A', 'NONE']), 64, 64);
  await advance(100);
  assert.equal(store.pickRead(at(['A', 'NONE'])).get(nameKey(100, 'NONE')), null);
  assert.equal(store.get(100, 'A'), 'name of A');
  // A read that keeps failing is never read with nothing, however often it fails.
  const failing = new HourStore<string>(8);
  const clocked = clockedReads(50, 0);
  const tries = new FactRequests(failing, clocked.read, () => {}, clocked.clock);
  tries.want(at(['A']), 64, 64);
  await clocked.advance(60_000);
  assert.equal(failing.answered(100, 'A'), false);
  assert.ok(clocked.stats.rejected > 3, `${clocked.stats.rejected} tries`);
  // Waits double, up to the most.
  assert.deepEqual([1, 2, 3, 10].map(retryDelay), [250, 500, 1000, RETRY_MOST_MS]);
});

test('a look asked while other widgets hold every request is read once they free one, never cached as missing', async () => {
  // Other widgets hold all 64 of the desktop's requests for the first 3 s.
  const { read, stats, advance, clock, desk } = clockedReads(100, 64);
  desk.others = 64;
  const store = new HourStore<string>(8);
  const requests = new FactRequests(store, read, () => {}, clock);
  requests.want(at(['GER']), 64, 64);
  await advance(2900);
  assert.ok(stats.rejected >= 3, `${stats.rejected} rejections`);
  // Still loading, not missing: the look shown before stays.
  assert.equal(store.answered(100, 'GER'), false);
  assert.equal(store.pickRead(at(['GER'])).has(nameKey(100, 'GER')), false);
  desk.others = 0;
  await advance(20_000);
  assert.equal(store.get(100, 'GER'), 'name of GER');
});

test('a failing read is tried again at once on a new attempt, and not at all once its hour is no longer wanted', async () => {
  const { read, stats, advance, clock, desk, timers } = clockedReads(50, 0);
  const store = new HourStore<string>(8);
  const requests = new FactRequests(store, read, () => {}, clock);
  requests.want(at(['A']), 64, 64);
  await advance(10);
  const before = stats.started;
  desk.limit = 64;
  assert.equal(requests.retry(), 1);
  assert.equal(stats.started, before + 1);
  await advance(100);
  assert.equal(store.get(100, 'A'), 'name of A');
  // Another hour wanted: the failing one is not asked for again.
  desk.limit = 0;
  requests.want([{ hour: 200, tag: 'B' }], 64, 64);
  await advance(200);
  requests.want([{ hour: 300, tag: 'C' }], 64, 64);
  const asked = stats.hours.length;
  await advance(30_000);
  assert.ok(stats.hours.slice(asked).every(hour => hour === 300), JSON.stringify(stats.hours.slice(asked)));
  requests.cancel();
  assert.equal(timers.length, 0);
});

test('configuration errors stay failures; refusals for want of room can be retried automatically', async () => {
  const spec = { product: 'country-flags', perRequest: 8, params: () => ({}), extract: () => new Map<string, string>() };
  const source = { sourceId: 's', revision: '1', branchId: '1' };
  const outcome = async (error: unknown) => {
    const client = { product: async () => { throw error; } } as unknown as DashboardClient;
    const chunks: [ReadonlyMap<string, string> | null, boolean | undefined][] = [];
    await readFacts(client, source, spec, [[100, ['GER']]], new AbortController().signal, (_hour, _tags, found, retryable) => chunks.push([found, retryable]));
    return chunks;
  };
  assert.deepEqual(await outcome(new Error('Choose the game and user folders in Settings first')), [[null, false]]);
  // Every refusal for want of room, as its producer words it: nothing decided, tried again.
  for (const refusal of [
    ...TRANSIENT_ERRORS,
    CAPACITY_REACHED, // the data service
    `${CAPACITY_REACHED}. Retry this view.`, // the desktop's and the browser bridge's admission
    NATIVE_QUEUE_FULL, // the desktop's pipe to the native service
    `${MAP_CONTROLS_BUSY}; try again`, // the desktop's map controls
  ]) assert.deepEqual(await outcome(new Error(refusal)), [[null, true]], refusal);
  assert.deepEqual(await outcome(new TypeError('Failed to fetch')), [[null, true]]);
});

test('flags at the same source and hour are requested again after configuring folders and retrying', async () => {
  const { clock, advance, timers } = clockedReads(0);
  let configured = false;
  let calls = 0;
  const client = { product: async (request: import('./client').ProductRequest) => {
    calls++;
    if (!configured) throw new Error('Choose the game and user folders in Settings first');
    return { ...request, data: { ABC: 'flag of the current country' } };
  } } as unknown as DashboardClient;
  const spec = { product: 'country-flags', perRequest: 32, params: () => ({}),
    extract: (data: unknown) => new Map(Object.entries(data as Record<string, string>)) };
  const read: FactRead<string> = (plan, signal, onChunk) => readFacts(client, { sourceId: 'same', revision: 'same', branchId: '1' }, spec, plan, signal, onChunk);
  const store = new HourStore<string>(8);
  const requests = new FactRequests(store, read, () => {}, clock);
  requests.want(at(['ABC']), 64, 32);
  await advance(30_000);
  assert.equal(calls, 1);
  assert.equal(store.answered(100, 'ABC'), false);
  assert.equal(store.pickRead(at(['ABC'])).has(nameKey(100, 'ABC')), false);
  assert.equal(requests.pickRead(at(['ABC'])).get(nameKey(100, 'ABC')), FAILED_FACT);
  assert.equal(timers.length, 0);
  configured = true;
  assert.equal(requests.retry(), 1);
  await advance(30_001);
  assert.equal(calls, 2);
  assert.equal(store.get(100, 'ABC'), 'flag of the current country');
  requests.cancel();
});

test('a read the desktop’s full request queue refused stays wanted and is read once there is room', async () => {
  let now = 0;
  const timers: { at: number; run: () => void }[] = [];
  const clock: FactClock = {
    now: () => now,
    after: (ms, run) => { const timer = { at: now + ms, run }; timers.push(timer); return () => { const index = timers.indexOf(timer); if (index >= 0) timers.splice(index, 1); }; },
  };
  const settle = async () => { for (let turn = 0; turn < 8; turn++) await Promise.resolve(); };
  const advance = async (to: number) => {
    // Every wait due by `to`, in time order, including those a retry sets.
    await settle();
    for (let timer = timers.find(item => item.at <= to); timer; timer = timers.find(item => item.at <= to)) {
      for (const item of timers) if (item.at < timer.at) timer = item;
      now = timer.at;
      timers.splice(timers.indexOf(timer), 1);
      timer.run();
      await settle();
    }
    now = to;
    await settle();
  };
  // The desktop's queue to the native service is full until `full` is cleared.
  const desk = { full: true, asked: 0 };
  const client = {
    product: async (request: { sourceId: string; revision: string; product: string }) => {
      desk.asked++;
      if (desk.full) throw new Error(NATIVE_QUEUE_FULL);
      return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: { GER: 'flag of GER' } };
    },
  } as unknown as DashboardClient;
  const spec = { product: 'country-flags', perRequest: 8, params: () => ({}),
    extract: (data: unknown) => new Map(Object.entries(data as Record<string, string>)) };
  const read: FactRead<string> = (plan, signal, onChunk) => readFacts(client, { sourceId: 's', revision: '1', branchId: '1' }, spec, plan, signal, onChunk);
  const store = new HourStore<string>(8);
  let changes = 0;
  const requests = new FactRequests(store, read, () => changes++, clock);
  requests.want(at(['GER']), 64, 64);
  await advance(1000);
  // Refused, never cached as missing: still loading, and asked again after each wait.
  assert.ok(desk.asked >= 2, `${desk.asked} asks`);
  assert.equal(store.answered(100, 'GER'), false);
  assert.equal(store.pickRead(at(['GER'])).has(nameKey(100, 'GER')), false);
  assert.ok(changes > 0); // Failure is exposed as unknown, so consumers redraw.
  const failedChanges = changes;
  // Room again: the next wait reads it.
  desk.full = false;
  await advance(1000 + RETRY_MOST_MS);
  assert.equal(store.get(100, 'GER'), 'flag of GER');
  assert.equal(changes, failedChanges + 1);
  // A new attempt (`retry`) reads at once too, without waiting.
  const later = new HourStore<string>(8);
  const again = new FactRequests(later, read, () => {}, clock);
  desk.full = true;
  again.want(at(['FRA', 'GER']), 64, 64);
  await settle();
  assert.equal(later.answered(100, 'GER'), false);
  desk.full = false;
  assert.equal(again.retry(), 1);
  await settle();
  assert.equal(later.get(100, 'GER'), 'flag of GER');
  // The reply lacked FRA: a completed read, so it is read with nothing.
  assert.equal(later.pickRead(at(['FRA'])).get(nameKey(100, 'FRA')), null);
  again.cancel();
  requests.cancel();
});

test('busy browser and full native queue refusals are retried without caching an empty fact', async () => {
  for (const message of [BROWSER_BRIDGE_BUSY, NATIVE_QUEUE_FULL]) {
    let busy = true;
    let attempts = 0;
    const client = { product: async (request: ProductRequest) => {
      attempts++;
      if (busy) throw new Error(message);
      return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: { fact: 'Aardvark' } };
    } } as unknown as DashboardClient;
    const spec = { product: 'capacity-test', perRequest: 64, params: () => ({}), extract: (data: unknown) => new Map([['AAA', (data as { fact: string }).fact]]) };
    const source = { sourceId: 's', revision: '1', branchId: '1' };
    const { clock, advance, timers } = clockedReads(1);
    const store = new HourStore<string>(8);
    const requests = new FactRequests(store, (plan, signal, onChunk) => readFacts(client, source, spec, plan, signal, onChunk), () => {}, clock);
    requests.want(at(['AAA']), 64, 64);
    await advance(0);
    assert.equal(store.answered(100, 'AAA'), false, message);
    assert.equal(store.pickRead(at(['AAA'])).size, 0, message);
    assert.equal(timers.length, 1, message);
    busy = false;
    if (message === NATIVE_QUEUE_FULL) assert.equal(requests.retry(), 1, message);
    await advance(250);
    assert.equal(attempts, 2, message);
    assert.equal(store.get(100, 'AAA'), 'Aardvark', message);
    requests.cancel();
  }
});

test('five thousand sorting names drain in bounded chunks with at most two reads going', async () => {
  const { read, stats, advance, clock } = clockedReads(10, 64);
  const store = new HourStore<string>(8);
  const requests = new FactRequests(store, read, () => {}, clock);
  const tags = Array.from({ length: 5000 }, (_, index) => `T${index}`);
  requests.want(at(tags), 64, 64);
  await advance(1000);
  assert.equal(stats.completed, Math.ceil(tags.length / 64));
  assert.equal(stats.cancelled, 0);
  assert.equal(stats.rejected, 0);
  assert.ok(stats.most <= MAX_READING);
  assert.ok(stats.asked.every(chunk => chunk.length <= 64));
  assert.equal(new Set(stats.asked.flat()).size, 5000);
  assert.equal(store.get(100, 'T4999'), 'name of T4999');
  requests.cancel();
});

test('sweeping a tooltip across 40 hours keeps at most two reads going, cancels the hours left behind and reads the last one', async () => {
  // Reads take 120 ms and the desktop takes at most 64 at once; the pointer moves an
  // hour every 10 ms, and names and flags each ask per hour (two chunks an hour).
  const { read, stats, advance, clock } = clockedReads(120, 64);
  const store = new HourStore<string>(64);
  const requests = new FactRequests(store, read, () => {}, clock);
  const hourOf = (step: number) => [{ hour: 100 + step, tag: 'GER' }, { hour: 100 + step, tag: 'ITA' }];
  for (let step = 0; step < 40; step++) {
    await advance(step * 10);
    requests.want(hourOf(step), 64, 1);
    assert.ok(requests.running <= MAX_READING, `${requests.running} reads at step ${step}`);
  }
  // The pointer stops on the last hour; no render follows, yet it is read.
  await advance(2000);
  assert.ok(stats.most <= MAX_READING, `${stats.most} at once`);
  assert.equal(stats.rejected, 0);
  assert.ok(stats.cancelled > 0);
  assert.equal(store.get(139, 'GER'), 'name of GER');
  assert.equal(store.get(139, 'ITA'), 'name of ITA');
  // Hours left behind were not read to the end.
  assert.ok(stats.completed <= 4, `${stats.completed} completed`);
  assert.equal(requests.running, 0);
});

test('a read the desktop rejects is tried again while still wanted', async () => {
  // A desktop taking one request at a time rejects the second chunk at first.
  const { read, stats, advance, clock } = clockedReads(50, 1);
  const store = new HourStore<string>(8);
  const requests = new FactRequests(store, read, () => {}, clock);
  requests.want(at(['A', 'B']), 64, 1);
  assert.equal(stats.rejected, 1);
  await advance(3000);
  assert.equal(store.get(100, 'A'), 'name of A');
  assert.equal(store.get(100, 'B'), 'name of B');
});
