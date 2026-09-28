import assert from 'node:assert/strict';
import test from 'node:test';
import { commitRows, EMPTY_HISTORY, nextDrop, nextRows, shownRows } from './rowMotion.ts';
import type { MovingRow, RowHistory } from './rowMotion';

const items = (...keys: string[]) => keys.map(key => ({ key, item: key.toUpperCase() }));
const view = (rows: readonly MovingRow<string>[]) => rows.map(row => `${row.key}:${row.slot}:${row.phase}`);

test('rendering reads only committed history: a repeated render shows a new row entering on both passes', () => {
  const first = items('a');
  let history: RowHistory<string> = commitRows(EMPTY_HISTORY, first, 0, 450);
  const second = items('a', 'b');
  // StrictMode renders twice before committing; both passes agree.
  assert.deepEqual(view(shownRows(history, second, 10, 450)), ['a:0:staying', 'b:1:entering']);
  assert.deepEqual(view(shownRows(history, second, 10, 450)), ['a:0:staying', 'b:1:entering']);
  // Once committed (twice, as StrictMode runs effects), the new row stays entering until the next update.
  history = commitRows(history, second, 20, 450);
  history = commitRows(history, second, 21, 450);
  assert.deepEqual(view(shownRows(history, second, 30, 450)), ['a:0:staying', 'b:1:entering']);
  assert.deepEqual(view(shownRows(history, items('b', 'a'), 40, 450)), ['b:0:staying', 'a:1:staying']);
});

test('an abandoned render leaves no trace: a row it added is not reported as leaving', () => {
  const history = commitRows(EMPTY_HISTORY, items('a'), 0, 450);
  // Render [a, b], then suspend: nothing is committed.
  shownRows(history, items('a', 'b'), 10, 450);
  const again = items('a');
  assert.deepEqual(view(shownRows(history, again, 20, 450)), ['a:0:staying']);
  assert.deepEqual(view(commitRows(history, again, 20, 450).rows), ['a:0:staying']);
});

test('committed leaving rows fade for their time from the commit, then are dropped', () => {
  let history = commitRows(EMPTY_HISTORY, items('a', 'b'), 0, 450);
  const next = items('a');
  history = commitRows(history, next, 1000, 450);
  assert.deepEqual(view(shownRows(history, next, 1200, 450)), ['a:0:staying', 'b:1:leaving']);
  assert.equal(nextDrop(history.rows, 450), 1450);
  assert.deepEqual(view(shownRows(history, next, 1450, 450)), ['a:0:staying']);
  const pruned = commitRows(history, next, 1450, 450);
  assert.deepEqual(view(pruned.rows), ['a:0:staying']);
  // Nothing moved: committing again keeps the same history.
  assert.equal(commitRows(pruned, next, 1500, 450), pruned);
});

test('the first table is shown without flashing, then new rows flash in and moved rows take their new slots', () => {
  const first = nextRows<string>([], items('a', 'b'), 0, 450, true);
  assert.deepEqual(view(first), ['a:0:staying', 'b:1:staying']);
  const next = nextRows(first, items('c', 'b', 'a'), 100, 450);
  assert.deepEqual(view(next), ['c:0:entering', 'b:1:staying', 'a:2:staying']);
  // A row flashes only in the update it appears in.
  assert.deepEqual(view(nextRows(next, items('c', 'b', 'a'), 200, 450)), ['c:0:staying', 'b:1:staying', 'a:2:staying']);
});

test('a row that is gone fades out where it was, then is dropped; one that comes back stays', () => {
  const shown = nextRows<string>([], items('a', 'b', 'c'), 0, 450, true);
  const leaving = nextRows(shown, items('a', 'c'), 1000, 450);
  assert.deepEqual(view(leaving), ['a:0:staying', 'c:1:staying', 'b:1:leaving']);
  assert.equal(nextDrop(leaving, 450), 1450);
  // Still fading: it keeps the time it started to leave.
  const later = nextRows(leaving, items('a', 'c'), 1300, 450);
  assert.deepEqual(later.find(row => row.key === 'b')?.since, 1000);
  assert.deepEqual(view(nextRows(later, items('a', 'c'), 1450, 450)), ['a:0:staying', 'c:1:staying']);
  assert.equal(nextDrop(nextRows(later, items('a', 'c'), 1450, 450), 450), null);
  assert.deepEqual(view(nextRows(leaving, items('a', 'b', 'c'), 1100, 450)), ['a:0:staying', 'b:1:staying', 'c:2:staying']);
});
