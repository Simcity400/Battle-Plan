import { test } from 'node:test';
import { deepEqual } from 'node:assert/strict';
import { timelineGaps, timelinePrepared } from './timelineGaps.ts';

test('gap markers clip inclusive hours after track alignment', () => {
  deepEqual(timelineGaps([
    { firstHour: 99, lastHour: 101, reason: 'loss' },
    { firstHour: 109, lastHour: 113, reason: 'tail' },
    { firstHour: 90, lastHour: 94, reason: 'outside' },
  ], 100, 0, 9), [
    { firstHour: 99, lastHour: 101, reason: 'loss', left: 0, width: 150 / 9 },
    { firstHour: 109, lastHour: 113, reason: 'tail', left: 850 / 9, width: 50 / 9 },
  ]);
});

test('a single-hour gap remains visible and invalid bounds have no markers', () => {
  const gaps = [{ firstHour: 3, lastHour: 3, reason: 'single' }];
  deepEqual(timelineGaps(gaps, 0, 3, 3), [{ ...gaps[0], left: 0, width: 100 }]);
  deepEqual(timelineGaps(gaps, 0, 4, 3), []);
});


test('prepared timeline coverage follows alignment and excludes every recorded gap', () => {
  deepEqual(timelinePrepared({ firstHour: 100, lastHour: 106 }, [
    { firstHour: 102, lastHour: 103, reason: 'loss' }, { firstHour: 103, lastHour: 104, reason: 'overlap' },
  ], 100, 0, 9).map(span => [span.firstHour, span.lastHour, span.left, span.width]), [
    [100, 101, 0, 150 / 9], [105, 106, 450 / 9, 200 / 9],
  ]);
  deepEqual(timelinePrepared(null, [], 0, 0, 9), []);
});
