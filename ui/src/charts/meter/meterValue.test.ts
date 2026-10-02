import { test } from 'node:test';
import assert from 'node:assert/strict';
import { meterValue } from './meterValue.ts';

test('native negative progress clamps to an empty meter and unknown stays unknown', () => {
  assert.equal(meterValue(-1), 0);
  assert.equal(meterValue(0.37), 0.37);
  assert.equal(meterValue(2), 1);
  assert.equal(meterValue(null), null);
  assert.equal(meterValue(NaN), null);
});
