import assert from 'node:assert/strict';
import test from 'node:test';
import { keepWanted, windowDisplay } from './productWindows.ts';
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

test('display keeps the supplying hour until the next window arrives and clears on identity changes', () => {
  const first = windowDisplay(null, 'branch-a', { window: 0 }, 123)!;
  assert.equal(windowDisplay(first, 'branch-a', undefined, 124), first);
  assert.equal(windowDisplay(first, 'branch-a', undefined, 900), first);
  assert.equal(windowDisplay(first, 'branch-b', undefined, 124), null);
  assert.equal(windowDisplay(first, null, undefined, 124), null);
  const next = { window: 1 };
  assert.deepEqual(windowDisplay(first, 'branch-a', next, 124), { identity: 'branch-a', data: next, hour: 124 });
  assert.equal(windowDisplay(first, 'branch-a', first.data, 123), first);
});
