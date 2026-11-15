import assert from 'node:assert/strict';
import test from 'node:test';
import { AHEAD_ROWS, AHEAD_SECONDS, aheadEnd } from './reveal.ts';

test('a block spans the playback held ahead, fewer hours where rows crowd, and paused ends at the next row', () => {
  const sparse = [10, 500, 4_000];
  assert.equal(aheadEnd(sparse, 100, 1440, true), 100 + 1440 * AHEAD_SECONDS, 'two seconds of playback at 1,440 hours a second');
  assert.equal(aheadEnd(sparse, 100, 24, true), 100 + 24 * AHEAD_SECONDS);
  assert.equal(aheadEnd(sparse, 100, 1440, false), 500, 'paused: the rows stand until the next one');
  assert.equal(aheadEnd(sparse, 4_000, 1440, false), Infinity, 'paused after the last row: nothing more arrives');
  const crowded = Array.from({ length: 1_000 }, (_, at) => 200 + at);
  const end = aheadEnd(crowded, 199, 1440, true);
  assert.equal(end, 200 + AHEAD_ROWS, 'at most so many rows drawn ahead of a render');
  assert.equal(crowded.filter(hour => hour > 199 && hour < end).length, AHEAD_ROWS);
});

test('rows not known yet end the block at the last known row\'s hour, and a block always moves past its first hour', () => {
  const known = [110, 120, 130, 999];
  assert.equal(aheadEnd(known, 100, 1440, true, 3, false), 130, 'the third row\'s fellows at its hour may wait in a page not held');
  assert.equal(aheadEnd(known, 100, 1440, true, 0, false), 101, 'nothing known ahead: the head alone');
  assert.equal(aheadEnd(known, 100, 0, true), 110, 'no motion: as paused');
  assert.equal(aheadEnd([100, 100, 100], 99, 1440, true, 3, false), 100);
  assert.ok(aheadEnd([], 5, 1440, true) > 5);
});
