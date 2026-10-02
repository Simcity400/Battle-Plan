import assert from 'node:assert/strict';
import test from 'node:test';
import { keepWanted } from './productWindows.ts';
import type { Held } from './productWindows.ts';

test('windows are held while wanted and released when not, and a new identity starts afresh', () => {
  const held: Held<string> = { identity: 'a', data: new Map([[3, 'march'], [4, 'april']]), errors: new Map([[5, 'failed']]) };
  // Wanting what is held changes nothing (the same object: no new render).
  assert.equal(keepWanted(held, 'a', new Set([3, 4, 5])), held);
  // Playback moves on a month: the one behind is released, the one ahead is kept ready.
  const moved = keepWanted(held, 'a', new Set([4, 5]));
  assert.deepEqual([...moved.data], [[4, 'april']]);
  assert.deepEqual([...moved.errors], [[5, 'failed']]);
  // A failed window no longer wanted is released, so wanting it again asks again.
  assert.deepEqual([...keepWanted(held, 'a', new Set([3])).errors], []);
  // Another country or recording: nothing carries over.
  const other = keepWanted(held, 'b', new Set([3, 4]));
  assert.deepEqual([other.identity, other.data.size, other.errors.size], ['b', 0, 0]);
});
