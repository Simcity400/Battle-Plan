import assert from 'node:assert/strict';
import test from 'node:test';
import { StructureOutbox } from './structureOutbox.ts';

test('structure waits in the worker while the page has not taken the last, and goes as one message, the newest of each subscription', () => {
  const sent: [number, number][][] = [];
  const outbox = new StructureOutbox(updates => sent.push(updates.map(update => [update.id, update.hour])));
  const read = (id: number, hour: number) => ({ id, seek: 1, hour, through: hour });
  outbox.add([read(1, 10), read(2, 10)]);
  assert.deepEqual(sent, [[[1, 10], [2, 10]]], 'structure goes at once while nothing is waiting on the page');
  for (let hour = 11; hour < 1500; hour++) outbox.add([read(hour % 3, hour)]);
  assert.equal(sent.length, 1, 'a page that has not taken the last message is sent nothing more');
  outbox.taken();
  assert.deepEqual(sent.at(-1), [[0, 1497], [1, 1498], [2, 1499]]);
  outbox.taken();
  assert.equal(sent.length, 2, 'nothing waiting, nothing sent');
  outbox.add([read(4, 1500)]);
  assert.deepEqual(sent.at(-1), [[4, 1500]], 'once taken, structure goes at once again');
});
