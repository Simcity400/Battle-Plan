import assert from 'node:assert/strict';
import test from 'node:test';
import { HeldPointer } from './pointer.ts';

test('a held pointer tracks the presented rows through the final frame and forgets the old owner', () => {
  const hits: (string | null)[] = [];
  const pointer = new HeldPointer(key => hits.push(key));
  pointer.move(12, 8);
  for (const top of [10, 5, 0]) pointer.present((x, y) => x > 0 && y < top ? 'first' : top ? 'moving' : 'last');
  pointer.clear();
  pointer.present(() => 'other');
  assert.deepEqual(hits, ['first', 'moving', 'last', null]);
});
