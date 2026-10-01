import assert from 'node:assert/strict';
import test from 'node:test';
import { FactRequests, HourStore, MAX_READING, nameKey, readFacts, RETRY_MOST_MS, retryDelay } from './countryFacts.ts';
import type { FactClock, FactRead, NameAt } from './countryFacts';
import type { DashboardClient, ProductRequest } from './client';
import { BROWSER_BRIDGE_BUSY, NATIVE_QUEUE_FULL, TRANSIENT_ERRORS } from '../generated/dashboard.ts';

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

test('a product error completes a read with nothing; a refusal for want of room is a failure to try again', async () => {
  const spec = { product: 'country-flags', perRequest: 8, params: () => ({}), extract: () => new Map<string, string>() };
  const source = { sourceId: 's', revision: '1', branchId: '1' };
  const outcome = async (error: unknown) => {
    const client = { product: async () => { throw error; } } as unknown as DashboardClient;
    const chunks: (ReadonlyMap<string, string> | null)[] = [];
    await readFacts(client, source, spec, [[100, ['GER']]], new AbortController().signal, (_hour, _tags, found) => chunks.push(found));
    return chunks;
  };
  // Flags without the game folders: the product's answer, so the placeholder shows.
  assert.deepEqual(await outcome(new Error('Choose the game and user folders in Settings first')), [new Map()]);
  // The desktop out of room: nothing decided, tried again.
  assert.deepEqual(await outcome(new Error(`${TRANSIENT_ERRORS[0]}. Retry this view.`)), [null]);
  assert.deepEqual(await outcome(new TypeError('Failed to fetch')), [null]);
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
