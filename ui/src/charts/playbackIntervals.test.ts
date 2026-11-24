import assert from 'node:assert/strict';
import test from 'node:test';
import { PlaybackIntervals, playbackRows, retainPlaybackMap, spanChanges } from './playbackIntervals.ts';
import { LogPlayback } from './log/logPlayback.ts';
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

test('log playback retains visible row objects and respects each producer supplying boundary', () => {
  const rows = deepFreeze([{ key: 'late', hour: 10, revealedHour: 30, item: 'late' },
    { key: 'old', hour: 1, revealedHour: 5, item: 'old' }]);
  const playback = new LogPlayback(rows);
  const first = playback.at(5);
  assert.equal(playback.at(29), first);
  assert.equal(first[0], rows[1]);
  assert.equal(playback.at(30), rows);
  assert.equal(playback.at(40), rows);
  const historicalDate = deepFreeze([{ key: 'future date', hour: 100, revealedHour: 10, item: '' }]);
  const supplying = new LogPlayback(historicalDate, row => row.revealedHour!);
  assert.equal(supplying.at(10), historicalDate);
});

test('unchanged empty rows and maps retain identity across irrelevant recorded changes', () => {
  const playback = playbackRows([1, 2, 3], () => []);
  assert.equal(playback.at(1), playback.at(2));
  const rows = deepFreeze(['row']);
  const before = deepFreeze(new Map([[1, rows]]));
  assert.equal(retainPlaybackMap(before, new Map([[1, rows]])), before);
  assert.notEqual(retainPlaybackMap(before, new Map([[1, [...rows]]])), before);
});

test('a count read at an hour holds over the span its rows show the same, and not past it', () => {
  const rows = [10, 20, 30].map(hour => ({ key: String(hour), hour, item: hour }));
  const log = new LogPlayback(rows);
  assert.equal(log.size, 3);
  assert.deepEqual(log.span(25), { first: 20, end: 30 });
  assert.deepEqual(log.span(5), { first: -Infinity, end: 10 });
  assert.deepEqual(log.span(40), { first: 30, end: Infinity });
  const intervals = new PlaybackIntervals([10, 20, 30], hour => hour >= 20);
  assert.deepEqual(intervals.span(25), { first: 20, end: 30 });
  assert.deepEqual(intervals.span(5), { first: -Infinity, end: 10 });
  const daily = new PlaybackIntervals([10, 20, 30], hour => hour >= 20, () => ({ first: 24, end: 26 }));
  daily.at(25);
  assert.deepEqual(daily.span(25), { first: 20, end: 30 });
});
