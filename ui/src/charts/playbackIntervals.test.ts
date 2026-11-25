import assert from 'node:assert/strict';
import test from 'node:test';
import { PlaybackIntervals, playbackRows, spanChanges } from './playbackIntervals.ts';
import { deepFreeze } from '../test-support/frozen.ts';

test('playback intervals retain models until a recorded or computed display boundary', () => {
  let reads = 0;
  const spans = deepFreeze([{ firstHour: 100, lastHour: 149 }]);
  const playback = new PlaybackIntervals(spanChanges(spans), hour => { reads++; return { day: Math.floor(hour / 24) }; },
    (_value, hour) => ({ first: Math.floor(hour / 24) * 24, end: (Math.floor(hour / 24) + 1) * 24 }));
  const first = playback.at(100);
  assert.equal(playback.at(119), first);
  assert.equal(reads, 1);
  assert.notEqual(playback.at(120), first);
  assert.equal(reads, 2);
  assert.deepEqual(playback.at(101), first);
  assert.equal(reads, 3);
});

test('unchanged empty rows retain identity across irrelevant recorded changes', () => {
  const playback = playbackRows([1, 2, 3], () => []);
  assert.equal(playback.at(1), playback.at(2));
});

test('a count read at an hour holds over the span its rows show the same, and not past it', () => {
  const intervals = new PlaybackIntervals([10, 20, 30], hour => hour >= 20);
  assert.deepEqual(intervals.span(25), { first: 20, end: 30 });
  assert.deepEqual(intervals.span(5), { first: -Infinity, end: 10 });
  const daily = new PlaybackIntervals([10, 20, 30], hour => hour >= 20, () => ({ first: 24, end: 26 }));
  daily.at(25);
  assert.deepEqual(daily.span(25), { first: 20, end: 30 });
});
