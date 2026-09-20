import test from 'node:test';
import assert from 'node:assert/strict';
import { ResourceCache } from './resourceCache.ts';
import { EffectLifetime } from './effectLifetime.ts';

const signal = () => new AbortController().signal;
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };

test('effect replay retains recording leases; actual unmount releases them once', async () => {
  const lifetime = new EffectLifetime();
  const client = {};
  let released = 0;
  const cleanup = () => { released++; };
  lifetime.attach(client, cleanup)();
  const detach = lifetime.attach(client, cleanup);
  await flush();
  assert.equal(released, 0);
  detach();
  await flush();
  assert.equal(released, 1);
});

test('changing transport disposes the old session before setting up the new one', async () => {
  const lifetime = new EffectLifetime();
  const order: string[] = [];
  lifetime.attach({}, () => order.push('old disposed'))();
  const detach = lifetime.attach({}, () => order.push('new disposed'));
  order.push('new attached');
  await flush();
  assert.deepEqual(order, ['old disposed', 'new attached']);
  detach(); await flush();
  assert.deepEqual(order, ['old disposed', 'new attached', 'new disposed']);
});

test('concurrent consumers and effect replay share one country scan', async () => {
  const cache = new ResourceCache<number>(4, 100, () => 1);
  let calls = 0;
  let finish!: (value: number) => void;
  let shared!: AbortSignal;
  const load = (input: AbortSignal) => { calls++; shared = input; return new Promise<number>(resolve => { finish = resolve; }); };
  const first = new AbortController();
  const abandoned = cache.get('["source","revision"]', load, first.signal);
  const rejection = assert.rejects(abandoned, { name: 'AbortError' });
  await flush();
  first.abort();
  const second = cache.get('["source","revision"]', load, signal());
  await flush();
  assert.equal(shared.aborted, false);
  finish(96);
  assert.equal(await second, 96); await rejection;
  assert.equal(await cache.get('["source","revision"]', load, signal()), 96);
  assert.equal(calls, 1);
});

test('last subscriber cancellation stops work; failures can retry', async () => {
  const cache = new ResourceCache<number>(4, 100, () => 1);
  const controller = new AbortController();
  let shared!: AbortSignal;
  const abandoned = cache.get('["source"]', input => { shared = input; return new Promise<number>((_, reject) => input.addEventListener('abort', () => reject(new Error('cancelled')))); }, controller.signal);
  const rejection = assert.rejects(abandoned, { name: 'AbortError' });
  await flush(); controller.abort(); await rejection; await flush();
  assert.equal(shared.aborted, true);
  await assert.rejects(cache.get('["source"]', async () => { throw new Error('offline'); }, signal()), /offline/);
  assert.equal(await cache.get('["source"]', async () => 96, signal()), 96);
});

test('bounded results isolate source revisions and clear on source release', async () => {
  const cache = new ResourceCache<number>(2, 2, () => 1);
  let calls = 0;
  const load = async () => ++calls;
  assert.equal(await cache.get('["a","r1"]', load, signal()), 1);
  assert.equal(await cache.get('["a","r2"]', load, signal()), 2);
  assert.equal(await cache.get('["b","r1"]', load, signal()), 3);
  assert.equal(await cache.get('["a","r1"]', load, signal()), 4);
  cache.deleteSource('a');
  assert.equal(await cache.get('["b","r1"]', load, signal()), 3);
  assert.equal(await cache.get('["a","r1"]', load, signal()), 5);
});

test('oversized artwork is served but not retained', async () => {
  const cache = new ResourceCache<number>(10, 4, value => value);
  let calls = 0;
  const load = async () => { calls++; return 5; };
  await cache.get('["a"]', load, signal());
  await cache.get('["a"]', load, signal());
  assert.equal(calls, 2);
});
