import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CELL, cellState, cellValue, parseAnswer, rowKeys, rowName, rowReason } from './message.ts';
import type { FollowedRows } from './message';
import { changedAnswerRows, mergeChanged, patchAnswer, serializeAnswer } from './tables.ts';
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
  const changed = rows({ keys: [wide(3n)], columns: 2, values: [(1n << 62n) + 7n, -9n], changed: true });
  const merged = parseAnswer(mergeChanged(base, changed)) as FollowedRows;
  assert.equal(rowKeys(merged), before);
  assert.deepEqual([cellValue(merged, 1), cellValue(merged, 4), cellValue(merged, 5)], [-(1n << 62n), (1n << 62n) + 7n, -9n]);
  assert.deepEqual([merged.canonical, cellValue(base, 4)], [true, 4n]);
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

test('review 5 finding 5: a sparse row whose reason leaves and returns never grows the merged table past the whole table native charged', () => {
  const keys = [1n, 2n, 3n], names = ['C1', 'C2', 'C3'], missing = 'This country was not recorded at this hour';
  let base = rows({ keys, columns: 1, values: [1n, 2n, 3n], names });
  for (let change = 0; change < 40; change++) {
    const gone = change % 2 === 0;
    const whole = answerBody({ keys, columns: 1, values: [1n, gone ? 0n : 2n, 3n], states: [0, gone ? CELL.unrecorded : 0, 0], reasons: [null, gone ? missing : null, null], names });
    const merged = mergeChanged(base, rows({ keys: [2n], columns: 1, values: [gone ? 0n : 2n], states: [gone ? CELL.unrecorded : 0], reasons: [gone ? missing : null], changed: true }), whole.length);
    assert.ok(merged.length <= whole.length, `${merged.length} bytes merged against ${whole.length} charged after ${change + 1} changes`);
    base = parseAnswer(merged) as FollowedRows;
    assert.equal(rowReason(base, 1), gone ? missing : null);
  }
});

test('patching one changed row in a 1,500-row table copies cells proportional to rows changed without mutating the base', () => {
  const keys = Array.from({ length: 1500 }, (_, i) => BigInt(i + 1));
  const values = keys.map(k => k * 10n);
  const names = keys.map(k => `C${k}`);
  const root = rows({ keys, columns: 1, values, names });
  let current: FollowedRows = root;
  const stats = { cellsCopied: 0 };
  for (let step = 0; step < 50; step++) {
    const idx = (step * 37) % 1500;
    const before = stats.cellsCopied;
    current = patchAnswer(current, rows({ keys: [keys[idx]!], columns: 1, values: [BigInt(100_000 + step)], changed: true }), stats);
    const copied = stats.cellsCopied - before;
    assert.ok(copied > 0 && copied <= 16, `copied ${copied} cells for 1 changed row of 1,500`);
  }
  assert.equal(cellValue(root, 0), 10n, 'base table is never mutated');
  assert.equal(cellValue(current, 0), 100_000n);
  assert.equal(cellValue(current, 37), 100_001n);
  assert.equal(cellValue(current, 1), 20n);
  assert.equal(rowName(current, 1499), 'C1500');
  const wire = serializeAnswer(current);
  const reparsed = parseAnswer(wire) as FollowedRows;
  assert.equal(cellValue(reparsed, 0), 100_000n);
  assert.equal(cellValue(reparsed, 1), 20n);
  assert.equal(rowName(reparsed, 1499), 'C1500');
});

test('N8: single-row patch index allocation stays logarithmic on a 10,000-row table rather than copying O(rows/64) root slots', () => {
  const N = 10_000;
  const keys = Array.from({ length: N }, (_, i) => BigInt(i + 1));
  const values = keys.map(k => k * 3n);
  const root = rows({ keys, columns: 1, values });
  const stats = { cellsCopied: 0, indexSlotsAllocated: 0 };
  const next = patchAnswer(root, rows({ keys: [keys[5432]!], columns: 1, values: [999_999n], changed: true }), stats);
  assert.ok(stats.cellsCopied > 0 && stats.cellsCopied <= 8, `copied ${stats.cellsCopied} cells`);
  assert.ok(stats.indexSlotsAllocated > 0 && stats.indexSlotsAllocated <= 48,
    `allocated ${stats.indexSlotsAllocated} index slots for 1 row of ${N} (expected O(log8 N) <= 48)`);
  assert.equal(cellValue(next, 5432), 999_999n);
  assert.equal(cellValue(next, 5431), 5432n * 3n);
  assert.equal(cellValue(root, 5432), 5433n * 3n);
});

test('serializeAnswer fast path preserves decoded answers via bulk tail copy and compacts when reasons change', () => {
  const keys = Array.from({ length: 1500 }, (_, i) => BigInt(i + 1));
  const names = keys.map(k => `C${k}`);
  const base = rows({ keys, columns: 2, values: keys.flatMap(k => [k, k * 2n]), names });
  const delta = rows({ keys: [keys[0]!, keys[777]!, keys[1499]!], columns: 2, values: [11n, 22n, 33n, 44n, 55n, 66n], changed: true });
  const fastParsed = parseAnswer(serializeAnswer(patchAnswer(base, delta))) as FollowedRows;
  const expected = rows({
    keys,
    columns: 2,
    values: keys.flatMap((k, i) => i === 0 ? [11n, 22n] : i === 777 ? [33n, 44n] : i === 1499 ? [55n, 66n] : [k, k * 2n]),
    names,
  });
  assert.equal(fastParsed.rows, expected.rows);
  assert.equal(fastParsed.columns, expected.columns);
  assert.equal(rowKeys(fastParsed), rowKeys(expected));
  for (let row = 0; row < expected.rows; row++) {
    assert.equal(rowName(fastParsed, row), rowName(expected, row));
    assert.equal(rowReason(fastParsed, row), rowReason(expected, row));
    for (let col = 0; col < expected.columns; col++) {
      const cell = row * expected.columns + col;
      assert.equal(cellValue(fastParsed, cell), cellValue(expected, cell));
      assert.equal(cellState(fastParsed, cell), cellState(expected, cell));
    }
  }
  const withReason = parseAnswer(serializeAnswer(patchAnswer(fastParsed, rows({ keys: [keys[777]!], columns: 2, values: [0n, 0n], states: [CELL.unrecorded, CELL.unrecorded], reasons: ['gone'], changed: true })))) as FollowedRows;
  assert.equal(rowReason(withReason, 777), 'gone');
  const clearedWire = serializeAnswer(patchAnswer(withReason, rows({ keys: [keys[777]!], columns: 2, values: [9n, 10n], changed: true })));
  const cleared = parseAnswer(clearedWire) as FollowedRows;
  assert.equal(rowReason(cleared, 777), null);
  assert.equal(cleared.ends.length - 1, 1500, 'cleared reason text is compacted out of the text table');
});

test('changedAnswerRows reports only exact mutated rows inside patched chunks and decodes ASCII and UTF-8 names', () => {
  const keys = Array.from({ length: 40 }, (_, i) => BigInt(i + 1));
  const names = keys.map((k, i) => (i === 5 ? 'Öst' : i === 12 ? 'GER' : `C${k}`));
  const base = rows({ keys, columns: 2, values: keys.flatMap(k => [k * 10n, k * 20n]), names });
  assert.equal(rowName(base, 5), 'Öst');
  assert.equal(rowName(base, 12), 'GER');
  assert.deepEqual(changedAnswerRows(base, base), []);

  const step1 = patchAnswer(base, rows({
    keys: [keys[2]!, keys[19]!],
    columns: 2,
    values: [999n, 30n, 190n, 400n],
    changed: true,
  }));
  assert.deepEqual(changedAnswerRows(base, step1), [2, 19], 'ignores unchanged sibling rows in the same 8-row chunks');

  const step2 = patchAnswer(step1, rows({
    keys: [keys[2]!, keys[31]!],
    columns: 2,
    values: [999n, 30n, 0n, 0n],
    states: [CELL.value, CELL.value, CELL.unrecorded, CELL.unrecorded],
    reasons: [null, 'offline'],
    changed: true,
  }));
  assert.deepEqual(changedAnswerRows(step1, step2), [31], 'no-op patch on row 2 is not reported as changed');
  assert.deepEqual(changedAnswerRows(base, step2), [2, 19, 31]);

  const otherBase = rows({ keys, columns: 2, values: keys.flatMap(k => [k * 10n, k * 20n]), names });
  assert.deepEqual(changedAnswerRows(base, otherBase), [], 'the same rows in another buffer (the worker\'s next hour) changed nothing');
  const reread = parseAnswer(serializeAnswer(step2)) as FollowedRows;
  assert.deepEqual(changedAnswerRows(base, reread), [2, 19, 31], 'rows compare across buffers: only the mutated ones');
  assert.deepEqual(changedAnswerRows(step1, reread), [31]);
  const fewer = rows({ keys: keys.slice(1), columns: 2, values: keys.slice(1).flatMap(k => [k * 10n, k * 20n]), names: names.slice(1) });
  assert.equal(changedAnswerRows(base, fewer), null, 'other rows cannot be compared row by row');
  const moved = rows({ keys: keys.map(k => k + 100n), columns: 2, values: keys.flatMap(k => [k * 10n, k * 20n]), names });
  assert.equal(changedAnswerRows(base, moved), null, 'other keys cannot be compared row by row');
});
