import assert from 'node:assert/strict';
import test from 'node:test';
import { exactDecimal, formatAmount, formatAxisPercent, formatAxisValue, formatNativeFixed5, formatSigned, linearTicks, rescale } from './valueAxis.ts';

const axis = (ticks: number[]) => ticks.map(tick => formatAxisValue(tick, ticks));

test('axis labels keep the precision of their ticks, so neighbours never read the same', () => {
  assert.deepEqual(axis([0, 0.1, 0.2, 0.3, 0.4]), ['0.0', '0.1', '0.2', '0.3', '0.4']);
  assert.deepEqual(axis([0, 0.25, 0.5, 0.75]), ['0.00', '0.25', '0.50', '0.75']);
  assert.deepEqual(axis([0, 250, 500, 750]), ['0', '250', '500', '750']);
  assert.deepEqual(axis([0, 1000, 2000, 3000]), ['0', '1k', '2k', '3k']);
  assert.deepEqual(axis([0, 500, 1000, 1500]), ['0', '500', '1.0k', '1.5k']);
  assert.deepEqual(axis([10000, 10000.01, 10000.02]), ['10,000.00', '10,000.01', '10,000.02']);
  assert.deepEqual(axis([10000, 10010, 10020]), ['10,000', '10,010', '10,020']);
  assert.deepEqual(axis([1000000, 1000100, 1000200]), ['1,000.0k', '1,000.1k', '1,000.2k']);
  assert.deepEqual(axis([0.00001, 0.00002]), ['0.00001', '0.00002']);
  assert.deepEqual(axis([0.1, 0.30000000000000004]), ['0.1', '0.3']);
  assert.equal(formatAxisValue(-0.5), '−0.5');
  assert.equal(formatAxisValue(-2000, [-2000, 0, 2000]), '−2k');
  assert.equal(formatAxisValue(1500), '1.5k');
});

test('ticks apart only past twelve significant digits still read apart', () => {
  assert.deepEqual(axis([10000000.00002, 10000000.00004]), ['10,000,000.00002', '10,000,000.00004']);
  assert.deepEqual(axis([123456789.0001, 123456789.0002]), ['123,456,789.0001', '123,456,789.0002']);
  const close = [2 ** 40, 2 ** 40 + 0.001];
  assert.equal(new Set(axis(close)).size, 2);
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

test('amounts in binary fixed point are written exactly in decimals', () => {
  assert.equal(rescale('1', 32768, 2), 0n);
  assert.equal(rescale('1', 32768, 5), 3n);
  assert.equal(rescale(String(1.5 * 32768), 32768, 0), 2n);
  assert.equal(rescale(String(-1.5 * 32768), 32768, 0), -2n);
  assert.equal(rescale(16384n, 32768n, 2), 50n);
  assert.equal(rescale(2n, 3n, 3), 667n);
});

test('percent axis labels keep small ticks apart and hold their sign', () => {
  const ticks = [0, 0.1, 0.2, 0.3, 0.4];
  assert.deepEqual(ticks.map(tick => formatAxisPercent(tick, ticks)), ['0.0%', '0.1%', '0.2%', '0.3%', '0.4%']);
  assert.deepEqual([-50, 0, 50].map(tick => formatAxisPercent(tick, [-50, 0, 50])), ['−50%', '0%', '50%']);
});

test('native fixed-digit amounts truncate toward zero and retain two digits', () => {
  assert.equal(formatNativeFixed5('123999', 2), '1.23');
  assert.equal(formatNativeFixed5('199999', 2), '1.99');
  assert.equal(formatNativeFixed5('100000', 2), '1.00');
  assert.equal(formatNativeFixed5('1234567890', 2), '12345.67');
  assert.equal(formatNativeFixed5('-123999', 2), '-1.23');
  assert.equal(formatNativeFixed5('0', 2), '0.00');
  assert.equal(formatNativeFixed5('-1', 2), '-0.00');
  assert.equal(formatNativeFixed5('214748364899999', 2), '-2147483648.99');
});

test('K4: linearTicks supports 2.5 steps so 0..100 at count 4 gives 0, 25, 50, 75, 100', () => {
  assert.deepEqual(linearTicks(0, 100, 4), [0, 25, 50, 75, 100]);
  assert.deepEqual(linearTicks(0, 100, 2), [0, 50, 100]);
  assert.deepEqual(linearTicks(0, 1, 4), [0, 0.25, 0.5, 0.75, 1]);
  assert.deepEqual(linearTicks(-1, 1, 4), [-1, -0.5, 0, 0.5, 1]);
});
