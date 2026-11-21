import assert from 'node:assert/strict';
import test from 'node:test';
import { ENTRY_MS, entryAlpha, entryEase } from './entryLayer.ts';

test('the shared entry layer eases as the rows\' CSS entry does: cubic-bezier(.2, .8, .2, 1) over 700 ms, the cover clearing as the row fades in', () => {
  assert.equal(ENTRY_MS, 700);
  assert.deepEqual([entryEase(-1), entryEase(0), entryEase(1), entryEase(2)], [0, 0, 1, 1]);
  let last = 0;
  for (let step = 1; step <= 100; step++) {
    const eased = entryEase(step / 100);
    assert.ok(eased >= last, `monotone at ${step}%`);
    last = eased;
  }
  const s = 0.3, x = 3 * (1 - s) ** 2 * s * 0.2 + 3 * (1 - s) * s ** 2 * 0.2 + s ** 3, y = 3 * (1 - s) ** 2 * s * 0.8 + 3 * (1 - s) * s ** 2 + s ** 3;
  assert.ok(Math.abs(entryEase(x) - y) < 1e-5, `${entryEase(x)} against ${y}`);
  for (const age of [0, 100, 350, 699, 700]) assert.ok(Math.abs(entryAlpha(age) - (1 - entryEase(age / ENTRY_MS))) < 1e-12, 'a cover at 1 − e(t) leaves the row at opacity e(t)');
  assert.equal(entryAlpha(0), 1, 'a row revealed this frame is wholly covered, as at opacity 0');
});
