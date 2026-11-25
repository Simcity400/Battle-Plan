import assert from 'node:assert/strict';
import test from 'node:test';
import { entryAt } from './reads.ts';

test('the entry at an hour is the latest recorded by then, never a later one', () => {
  const entries = [{ firstHour: 10, name: 'A' }, { firstHour: 20, name: null }, { firstHour: 30, name: 'C' }];
  assert.equal(entryAt(entries, 9), null);
  assert.equal(entryAt(entries, 10)?.name, 'A');
  assert.equal(entryAt(entries, 25)?.name, null);
  assert.equal(entryAt(entries, 25)?.firstHour, 20);
  assert.equal(entryAt(entries, 99)?.name, 'C');
  assert.equal(entryAt([], 5), null);
});
