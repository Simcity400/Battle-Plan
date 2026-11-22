import assert from 'node:assert/strict';
import test from 'node:test';
import { StructureOutbox } from './structureOutbox.ts';

test('structure waits in the worker while the page has not taken the last, and is built when it goes', () => {
  const sent: number[][] = [], changed = new Map<number, number>();
  const outbox = new StructureOutbox(() => {
    if (!changed.size) return false;
    sent.push([...changed].map(([id, hour]) => id * 10000 + hour));
    changed.clear();
    return true;
  });
  changed.set(1, 10); changed.set(2, 10); outbox.flush();
  assert.deepEqual(sent, [[10010, 20010]], 'structure goes at once while nothing is waiting on the page');
  for (let hour = 11; hour < 1500; hour++) { changed.set(hour % 3, hour); outbox.flush(); }
  assert.equal(sent.length, 1, 'a page that has not taken the last message is sent nothing more');
  outbox.taken();
  assert.deepEqual(sent.at(-1)!.sort((a, b) => a - b), [1497, 11498, 21499], 'then the newest of each, built when it goes');
  outbox.taken();
  assert.equal(sent.length, 2, 'nothing changed, nothing sent');
  changed.set(4, 1500); outbox.flush();
  assert.deepEqual(sent.at(-1), [41500], 'once taken, structure goes at once again');
});
