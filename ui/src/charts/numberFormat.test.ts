import assert from 'node:assert/strict';
import test from 'node:test';
import { enTwoDecimals, gbNumber, gbTwoSignificant, usFiveDecimals, usNumber, usWhole } from './numberFormat.ts';

test('shared number formats write exactly what toLocaleString writes', () => {
  for (const value of [0, -0, 7, -1234.56789, 0.0001234567, 98765432.123456, 2 ** 53 + 2, 12345678901234567890n, -42n]) {
    assert.equal(usNumber.format(value), value.toLocaleString('en-US'));
    assert.equal(usNumber.format(value), value.toLocaleString('en-US', { maximumFractionDigits: 3 }));
    assert.equal(usNumber.format(value), value.toLocaleString('en'));
    assert.equal(usWhole.format(value), value.toLocaleString('en-US', { maximumFractionDigits: 0 }));
    assert.equal(usFiveDecimals.format(value), value.toLocaleString('en-US', { maximumFractionDigits: 5 }));
    assert.equal(gbNumber.format(value), value.toLocaleString('en-GB'));
    assert.equal(gbTwoSignificant.format(value), value.toLocaleString('en-GB', { maximumSignificantDigits: 2 }));
    assert.equal(enTwoDecimals.format(value), value.toLocaleString('en', { maximumFractionDigits: 2 }));
  }
});
