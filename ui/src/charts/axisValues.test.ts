import assert from 'node:assert/strict';
import test from 'node:test';
import { exactDecimal, formatAmount, formatAxisValue, formatSigned, linearTicks } from './valueAxis.ts';

const axis = (ticks: number[]) => ticks.map(tick => formatAxisValue(tick, ticks));

test('axis labels keep the precision of their ticks, so neighbours never read the same', () => {
  assert.deepEqual(axis([0, 0.1, 0.2, 0.3, 0.4]), ['0.0', '0.1', '0.2', '0.3', '0.4']);
  assert.deepEqual(axis([0, 0.25, 0.5, 0.75]), ['0.00', '0.25', '0.50', '0.75']);
  assert.deepEqual(axis([0, 250, 500, 750]), ['0', '250', '500', '750']);
  assert.deepEqual(axis([0, 1000, 2000, 3000]), ['0', '1k', '2k', '3k']);
  assert.deepEqual(axis([0, 500, 1000, 1500]), ['0', '500', '1.0k', '1.5k']);
  // Tight ranges at large magnitudes fall back from k rather than collapse.
  assert.deepEqual(axis([10000, 10000.01, 10000.02]), ['10,000.00', '10,000.01', '10,000.02']);
  assert.deepEqual(axis([10000, 10010, 10020]), ['10,000', '10,010', '10,020']);
  assert.deepEqual(axis([1000000, 1000100, 1000200]), ['1,000.0k', '1,000.1k', '1,000.2k']);
  assert.deepEqual(axis([0.00001, 0.00002]), ['0.00001', '0.00002']);
  // Float noise from tick arithmetic is not precision.
  assert.deepEqual(axis([0.1, 0.30000000000000004]), ['0.1', '0.3']);
  assert.equal(formatAxisValue(-0.5), '−0.5');
  assert.equal(formatAxisValue(-2000, [-2000, 0, 2000]), '−2k');
  assert.equal(formatAxisValue(1500), '1.5k');
});

test('ticks apart only past twelve significant digits still read apart', () => {
  // The review's case: 12 significant digits made these both "10,000,000".
  assert.deepEqual(axis([10000000.00002, 10000000.00004]), ['10,000,000.00002', '10,000,000.00004']);
  assert.deepEqual(axis([123456789.0001, 123456789.0002]), ['123,456,789.0001', '123,456,789.0002']);
  // Past fifteen digits the doubles are read exactly.
  const close = [2 ** 40, 2 ** 40 + 0.001];
  assert.equal(new Set(axis(close)).size, 2);
  // Whatever the range, the ticks the axis draws never share a label.
  for (const [min, max] of [[10000000, 10000000.0001], [1e9, 1e9 + 0.5], [0, 1e-6], [-3.2, 7.9], [99999.99, 100000.01], [1e12, 1e12 + 3]]) {
    for (const count of [2, 4, 8]) {
      const ticks = linearTicks(min, max, count);
      assert.equal(new Set(axis(ticks)).size, ticks.length, `${min}..${max} by ${count}: ${axis(ticks).join(' ')}`);
    }
  }
});

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
