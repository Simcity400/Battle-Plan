import assert from 'node:assert/strict';
import test from 'node:test';
import { formatRate } from './valueAxis.ts';

test('a rate keeps whole numbers from 10 and two significant figures below, never reading as 0', () => {
  assert.deepEqual(['4932180', '172160', '78472', '8795', '3569', '10139'].map(raw => formatRate(raw, 5)),
    ['49', '1.7', '0.78', '0.088', '0.036', '0.1']);
  assert.equal(formatRate(1n, 5), '0.00001');
  assert.equal(formatRate(0n, 5), '0');
  assert.equal(formatRate(123456789n, 5), '1,235');
  assert.equal(formatRate(-78472n, 5), '−0.78');
  assert.equal(formatRate(7n, 0), '7');
  assert.equal(formatRate(1310n, 0), '1,310');
});
