import { test } from 'node:test';
import { deepEqual, equal } from 'node:assert/strict';
import { timelineGaps, timelineHourAt } from './timelineGaps.ts';

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



test('a pointer on the timeline names one hour for its tooltip and its seek, within the recording', () => {
  const rect = { left: 100.4, width: 1450 };
  const first = 60_759_371, last = 60_864_912;
  const at = timelineHourAt(100.4 + 7 + 0.3 * 1436, rect, first, last);
  equal(at.hour, Math.round(first + 0.3 * (last - first)));
  equal(Number.isInteger(at.hour), true);
  deepEqual(timelineHourAt(0, rect, first, last), { left: 7, hour: first });
  deepEqual(timelineHourAt(5000, rect, first, last), { left: 1443, hour: last });
});
