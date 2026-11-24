import assert from 'node:assert/strict';
import test from 'node:test';
import { logPlaybackAt, logPlaybackIndex } from './logPlayback.ts';
import type { LogRow } from './logData';

test('playback cuts supplying reads without reordering exact dates or observing future rows', () => {
  const rows: LogRow<string>[] = [
    { key: 'new', hour: 30, revealedHour: 30, item: 'new' },
    { key: 'old-late', hour: 10, revealedHour: 40, item: 'old-late', by: false },
    { key: 'old', hour: 5, item: 'old' },
  ];
  const index = logPlaybackIndex(rows);
  assert.deepEqual(logPlaybackAt(index, null).rows, []);
  assert.deepEqual(logPlaybackAt(index, 4).rows, []);
  const early = logPlaybackAt(index, 30);
  assert.deepEqual(early.rows.map(row => row.key), ['new', 'old']);
  assert.equal(logPlaybackAt(index, 39, early), early);
  assert.equal(logPlaybackAt(index, 40, early).rows, rows);
  assert.deepEqual(logPlaybackAt(index, 9).rows.map(row => row.key), ['old']);
});

