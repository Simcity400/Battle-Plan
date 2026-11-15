import test from 'node:test';
import assert from 'node:assert/strict';
import { ideaChangeHours } from './ideaHistory.ts';
import { settledHour } from './settledHour.ts';

test('a followed idea view settles on the last hour its holdings, gaps or identity changed', () => {
  const hours = ideaChangeHours({
    spans: [{ token: 'a', firstHour: 100, lastHour: 114, slot: null, character: null }, { token: 'b', firstHour: 110, lastHour: 130, slot: null, character: null }],
    unobserved: [{ firstHour: 115, lastHour: 115 }],
    identityStarts: [100, 116],
  });
  assert.deepEqual(hours, [100, 110, 115, 116, 131]);
  assert.equal(settledHour(hours, 109), 100, 'the hours between two changes show the same holdings: no render');
  assert.equal(settledHour(hours, 110), 110);
  assert.equal(settledHour(hours, 115), 115);
  assert.equal(settledHour(hours, 129), 116);
  assert.equal(settledHour(hours, 99), 99, 'before the first change the hour itself');
});
