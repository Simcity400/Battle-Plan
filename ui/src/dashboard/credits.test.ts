import assert from 'node:assert/strict';
import { test } from 'node:test';
import { credits } from './credits.ts';

test('review 6 A: the page reports the total it has released, so a retry after a lost acknowledgement frees nothing twice', async () => {
  const sent: number[] = [], runs: (() => void)[] = [];
  let accept = false;
  const owed = credits(async released => { sent.push(released); return accept; }, run => runs.push(run));
  owed.owe(40); owed.owe(2);
  runs.shift()!(); await Promise.resolve(); await Promise.resolve();
  assert.deepEqual([sent, owed.owed()], [[42], 42], 'an unacknowledged release is still owed');
  runs.shift()!(); await Promise.resolve(); await Promise.resolve();
  assert.deepEqual(sent, [42, 42], 'the retry names the same total, which native applies once');
  accept = true; owed.owe(8);
  runs.shift()!(); await Promise.resolve(); await Promise.resolve();
  assert.deepEqual([sent, owed.owed()], [[42, 42, 50], 0]);
});
