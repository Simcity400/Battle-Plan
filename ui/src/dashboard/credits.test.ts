import assert from 'node:assert/strict';
import { test } from 'node:test';
import { credits } from './credits.ts';

test('review 5 finding 2: a refund the service did not acknowledge is kept and sent again, never lost', async () => {
  const sent: number[] = [], runs: (() => void)[] = [];
  let accept = false;
  const owed = credits(async bytes => { sent.push(bytes); return accept; }, run => runs.push(run));
  owed.owe(40); owed.owe(2);
  runs.shift()!(); await Promise.resolve(); await Promise.resolve();
  assert.deepEqual([sent, owed.owed()], [[42], 42], 'a failed refund is owed again');
  accept = true; owed.owe(8);
  runs.shift()!(); await Promise.resolve(); await Promise.resolve();
  assert.deepEqual([sent, owed.owed()], [[42, 50], 0]);
});
