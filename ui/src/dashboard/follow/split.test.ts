import assert from 'node:assert/strict';
import { test } from 'node:test';
import { parseDatedPanels, parseFollowed } from './message.ts';
import { datedPanel, message } from './messages.fixture.ts';
import { charge, laneOf, resetStamps, splitForLanes } from './split.ts';

const one = (key: bigint) => ({ keys: [key], columns: 1, values: [key] });

test('each lane gets its own answers and every confirmation, and the bytes it owns sum with the header to what native charged', () => {
  const cell = 0x8000_0003;
  const bytes = message(40, [{ id: 4, answer: one(4n) }, { id: 5, answer: one(5n) }, { id: cell, answer: one(9n) }], true, 7, 3);
  const split = splitForLanes(bytes, 2);
  assert.ok(split.kind === 'follow');
  const [even, odd] = split.lanes.map(lane => parseFollowed(lane!));
  assert.deepEqual([even!.answers.map(a => a.id), odd!.answers.map(a => a.id)], [[4], [5, cell]]);
  assert.deepEqual([even!.hour, even!.reset, even!.route, even!.seek, odd!.hour, odd!.reset], [40, true, 7, 3, 40, true], 'route-wide parts reach every lane');
  assert.equal(split.owned[0]! + split.owned[1]! + split.header, charge(bytes));
  assert.equal(charge(bytes), 24 + 3 * 16 + 2 * (bytes.byteLength - 24 - 3 * 16), 'each answer is charged for the lane\'s copy and the page\'s one copy');
  assert.equal(laneOf(cell, 2), 1, 'a cell group belongs to the lane class of its id without the high bit');
});

test('a completed step goes to every lane and is credited at once', () => {
  const panel = datedPanel(3, 10, { n: 1 });
  const split = splitForLanes(panel, 4);
  assert.ok(split.kind === 'panels');
  assert.deepEqual(split.lanes.map(lane => lane && parseDatedPanels(lane).panels.map(p => p.id)), [null, null, null, [3]]);
  assert.equal(split.owned[3]! + split.header, charge(panel));
  assert.equal(charge(panel), 2 * panel.byteLength - 24);
});

test('a failure is not split', () => {
  const bytes = new Uint8Array([0x46, 0x45, 0x4e, 0x31, 0x6e, 0x6f]);
  assert.deepEqual(splitForLanes(bytes, 3), { kind: 'failure', reason: 'no' });
});

test('should-fix d: dated panels of a version the page does not read are refused by name, never split as if they were', () => {
  const bytes = datedPanel(3, 10, { rows: [1] });
  new DataView(bytes.buffer).setUint16(4, 5, true);
  assert.throws(() => splitForLanes(bytes, 2), /version 5 are not supported/);
});

test('review 11: a reset is stamped once per route before the split, so every lane carries the same count', () => {
  const stamp = resetStamps();
  assert.equal(stamp(message(10, [], true, 3, 1)), 1);
  assert.equal(stamp(message(11, [], false, 3, 1)), 1, 'a later message carries the count for a lane that joins now');
  assert.equal(stamp(message(5, [], true, 3, 0)), 1, 'a reset of an older seek is never counted');
  assert.equal(stamp(message(12, [], true, 3, 2)), 2);
  assert.equal(stamp(message(12, [], true, 4, 2)), 1, 'each route keeps its own count');
});
