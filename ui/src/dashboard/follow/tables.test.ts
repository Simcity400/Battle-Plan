import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CELL, cellState, cellValue, parseAnswer, rowKeys, rowName, rowReason } from './message.ts';
import type { FollowedRows } from './message';
import { mergeChanged } from './tables.ts';
import { answerBody } from './messages.fixture.ts';

const rows = (spec: Parameters<typeof answerBody>[0]) => parseAnswer(answerBody(spec)) as FollowedRows;
const wide = (n: bigint) => (n << 96n) + n;

test('review 3: a changed row keeps its reason from its own answer, not the index into the base', () => {
  const base = rows({ keys: [1n, 2n, 3n], columns: 1, values: [10n, 20n, 30n], names: ['C000', 'C001', 'C002'] });
  const changed = rows({ keys: [2n], columns: 1, values: [0n], states: [CELL.unrecorded], reasons: ['This country was not recorded at this hour'], changed: true });
  const merged = parseAnswer(mergeChanged(base, changed)) as FollowedRows;
  assert.deepEqual([0, 1, 2].map(row => rowReason(merged, row)), [null, 'This country was not recorded at this hour', null]);
  assert.deepEqual([0, 1, 2].map(row => rowName(merged, row)), ['C000', 'C001', 'C002'], 'names stay the base');
  assert.deepEqual([cellState(merged, 1), cellValue(merged, 0), cellValue(merged, 2)], [CELL.unrecorded, 10n, 30n]);
});

test('review 3: full-width keys and values survive a merge walk, and the base is never changed', () => {
  const base = rows({ keys: [wide(1n), wide(2n), wide(3n)], columns: 2, values: [1n, -(1n << 62n), 2n, 3n, 4n, 5n] });
  const before = rowKeys(base);
  const changed = rows({ keys: [wide(3n)], columns: 2, values: [(1n << 62n) + 7n, -9n], changed: true, window: 2 });
  const merged = parseAnswer(mergeChanged(base, changed)) as FollowedRows;
  assert.equal(rowKeys(merged), before);
  assert.deepEqual([cellValue(merged, 1), cellValue(merged, 4), cellValue(merged, 5)], [-(1n << 62n), (1n << 62n) + 7n, -9n]);
  assert.deepEqual([merged.canonical, merged.window, cellValue(base, 4)], [true, 2, 4n]);
});

test('changed rows out of key order, of another shape or of an unheld key are refused by name', () => {
  const base = rows({ keys: [1n, 2n], columns: 1, values: [1n, 2n] });
  assert.throws(() => mergeChanged(base, rows({ keys: [9n], columns: 1, values: [1n], changed: true })), /does not hold/);
  assert.throws(() => mergeChanged(base, rows({ keys: [2n, 1n], columns: 1, values: [1n, 1n], changed: true })), /key order/);
  assert.throws(() => mergeChanged(base, rows({ keys: [1n], columns: 2, values: [1n, 1n], changed: true })), /columns/);
});

test('merging a thousand changes appends reasons, compacts the stale ones, and keeps every name and reason right', () => {
  const keys = Array.from({ length: 50 }, (_, at) => BigInt(at + 1));
  let base = rows({ keys, columns: 1, values: keys.map(() => 0n), names: keys.map(key => `C${key}`) });
  for (let change = 0; change < 1000; change++) {
    const key = keys[change % keys.length]!;
    const merged = mergeChanged(base, rows({ keys: [key], columns: 1, values: [0n], states: [CELL.unrecorded], reasons: [`reason ${change}`], changed: true }));
    base = parseAnswer(merged) as FollowedRows;
    assert.ok(base.ends.length - 1 <= keys.length * 3 + 17, `${base.ends.length - 1} texts after ${change + 1} changes`);
  }
  assert.deepEqual([rowName(base, 7), rowReason(base, 7), rowReason(base, 49)], ['C8', 'reason 957', 'reason 999']);
});
