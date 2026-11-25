import assert from 'node:assert/strict';
import test from 'node:test';
import { MAX_WINDOW_CONCURRENCY, keepWanted, runBoundedQueue } from './productWindows.ts';
import type { Held } from './productWindows.ts';

test('windows are held while wanted and released when not, and a new identity starts afresh', () => {
  const held: Held<string> = { identity: 'a', data: new Map([[3, 'march'], [4, 'april']]), errors: new Map([[5, new Error('failed')]]) };
  assert.equal(keepWanted(held, 'a', new Set([3, 4, 5])), held);
  const moved = keepWanted(held, 'a', new Set([4, 5]));
  assert.deepEqual([...moved.data], [[4, 'april']]);
  assert.deepEqual([...moved.errors], [[5, new Error('failed')]]);
  assert.deepEqual([...keepWanted(held, 'a', new Set([3])).errors], []);
  const other = keepWanted(held, 'b', new Set([3, 4]));
  assert.deepEqual([other.identity, other.data.size, other.errors.size], ['b', 0, 0]);
});

test('runBoundedQueue caps in-flight window fetches regardless of how many windows are asked', async () => {
  let inFlight = 0;
  let peak = 0;
  const done: number[] = [];
  const resolvers: (() => void)[] = [];
  const items = Array.from({ length: 40 }, (_, index) => index);
  runBoundedQueue(items, MAX_WINDOW_CONCURRENCY, item => new Promise<void>(resolve => {
    inFlight++;
    peak = Math.max(peak, inFlight);
    resolvers.push(() => {
      inFlight--;
      done.push(item);
      resolve();
    });
  }));
  assert.equal(peak, MAX_WINDOW_CONCURRENCY);
  assert.equal(inFlight, MAX_WINDOW_CONCURRENCY);
  while (resolvers.length > 0) {
    resolvers.shift()!();
    await Promise.resolve();
  }
  assert.equal(peak, MAX_WINDOW_CONCURRENCY);
  assert.equal(done.length, items.length);
});
