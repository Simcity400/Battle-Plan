import assert from 'node:assert/strict';
import test from 'node:test';
import { TableMotion } from './paint.ts';
import type { Motion, PaintRow } from './paint.ts';

test('a restarted presentation matches a fresh model after moving and retiring rows', () => {
  const motion: Motion = { glide: 100, bar: 200, enter: 300, leave: 400 };
  const row = (key: string, bar: number): PaintRow => ({ key, cells: [{ text: key, bar }], data: key });
  const a = row('a', 0.2), b = row('b', 0.4), c = row('c', 0.6), d = row('d', 0.8);
  const retained = new TableMotion();
  retained.show({ rows: [a, b, c] }, 0, motion, true);
  retained.frame(0, motion.enter);
  retained.show({ rows: [b, a] }, 25, motion, false);
  retained.frame(50, motion.enter);

  retained.restart();
  assert.deepEqual(retained.frame(60, motion.enter), { rows: [], moving: false });

  const next = { rows: [c, a, d] };
  const fresh = new TableMotion();
  retained.show(next, 100, motion, true);
  fresh.show(next, 100, motion, true);
  const actual = retained.frame(100, motion.enter);
  assert.deepEqual(actual, fresh.frame(100, motion.enter));
  assert.deepEqual(actual.rows.map(item => [item.row.key, item.y, item.alpha, item.flash, item.leaving]),
    [['c', 0, 1, 0, false], ['a', 1, 1, 0, false], ['d', 2, 1, 0, false]]);
});
