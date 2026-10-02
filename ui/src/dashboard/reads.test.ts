import assert from 'node:assert/strict';
import test from 'node:test';
import { formatDateTime } from './calendar.ts';
import { entryAt, readAt, readNote } from './reads.ts';

test('a value is dated by the latest read at or before its hour', () => {
  const reads = [100, 124, 148];
  assert.equal(readAt(reads, 99), null);
  assert.equal(readAt(reads, 100), 100);
  assert.equal(readAt(reads, 123), 100);
  assert.equal(readAt(reads, 124), 124);
  assert.equal(readAt(reads, 1000), 148);
  assert.equal(readAt([], 5), null);
});

test('a value read earlier says when, and one read at its hour says nothing', () => {
  assert.equal(readNote(100, 110, 'resources are read once a day.'), `Read ${formatDateTime(100)}; resources are read once a day.`);
  assert.equal(readNote(110, 110, 'x'), undefined);
  assert.equal(readNote(null, 110, 'x'), undefined);
});

test('the entry at an hour is the latest recorded by then, never a later one', () => {
  const entries = [{ firstHour: 10, name: 'A' }, { firstHour: 20, name: null }, { firstHour: 30, name: 'C' }];
  assert.equal(entryAt(entries, 9), null);
  assert.equal(entryAt(entries, 10)?.name, 'A');
  assert.equal(entryAt(entries, 25)?.name, null);
  assert.equal(entryAt(entries, 25)?.firstHour, 20);
  assert.equal(entryAt(entries, 99)?.name, 'C');
  assert.equal(entryAt([], 5), null);
});
