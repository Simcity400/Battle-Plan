import assert from 'node:assert/strict';
import test from 'node:test';
import { minesText } from './minefieldsData.ts';

test('mines read exactly on their scale', () => {
  assert.equal(minesText(1234567n, 100000), '12.34567');
  assert.equal(minesText(-100000n, 100000), '−1');
  assert.equal(minesText(0n, 100000), '0');
});
