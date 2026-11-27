import assert from 'node:assert/strict';
import test from 'node:test';
import { exactDecimal, formatAmount, formatSigned, linearTicks, rescale } from './valueAxis.ts';

test('an exact decimal drops binary noise and trailing zeros', () => {
  assert.deepEqual(exactDecimal(0.1 + 0.2), { units: 3n, scale: 1 });
  assert.deepEqual(exactDecimal(1500), { units: 1500n, scale: 0 });
  assert.deepEqual(exactDecimal(1e21), { units: 10n ** 21n, scale: 0 });
  assert.deepEqual(exactDecimal(-0.00025), { units: -25n, scale: 5 });
  assert.deepEqual(exactDecimal(0), { units: 0n, scale: 0 });
});

test('amounts show at most two decimals; signs only where they mean something', () => {
  assert.equal(formatAmount(30100000n, 5), '301');
  assert.equal(formatAmount(390000n, 5), '3.9');
  assert.equal(formatAmount(2199633n, 5), '22');
  assert.equal(formatAmount('30000000', 5), '300');
  assert.equal(formatAmount(123456n, 5, 4), '1.2346');
  assert.equal(formatSigned(19600000n, 5), '+196');
  assert.equal(formatSigned(-500000n, 5), '−5');
  assert.equal(formatSigned(1n, 5), '0');
});

test('amounts in binary fixed point are written exactly in decimals', () => {
  assert.equal(rescale('1', 32768, 2), 0n);
  assert.equal(rescale('1', 32768, 5), 3n);
  assert.equal(rescale(String(1.5 * 32768), 32768, 0), 2n);
  assert.equal(rescale(String(-1.5 * 32768), 32768, 0), -2n);
  assert.equal(rescale(16384n, 32768n, 2), 50n);
  assert.equal(rescale(2n, 3n, 3), 667n);
});

test('K4: linearTicks supports 2.5 steps so 0..100 at count 4 gives 0, 25, 50, 75, 100', () => {
  assert.deepEqual(linearTicks(0, 100, 4), [0, 25, 50, 75, 100]);
  assert.deepEqual(linearTicks(0, 100, 2), [0, 50, 100]);
  assert.deepEqual(linearTicks(0, 1, 4), [0, 0.25, 0.5, 0.75, 1]);
  assert.deepEqual(linearTicks(-1, 1, 4), [-1, -0.5, 0, 0.5, 1]);
});
