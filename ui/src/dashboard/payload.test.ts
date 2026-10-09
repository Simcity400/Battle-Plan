import { test } from 'node:test';
import assert from 'node:assert/strict';
import { i32Value, nullableValue, orderedHourSpans, pngDataValue, raw64Value, recordValue, textValue, tokenValue } from './payload.ts';

test('signed i32 values preserve boundaries independently of date meaning', () => {
  for (const value of [-(2 ** 31), -1, 0, 2 ** 31 - 1]) assert.equal(i32Value(value), true);
  for (const value of [-(2 ** 31) - 1, 2 ** 31, 1.5, Infinity, NaN, '1', null]) assert.equal(i32Value(value), false);
});

test('record, text and token shapes preserve unknown extra fields and reject NUL', () => {
  assert.equal(recordValue({ extension: 3 }), true);
  for (const value of [null, [], 'value']) assert.equal(recordValue(value), false);
  assert.equal(textValue(''), true);
  assert.equal(textValue('A mod token with spaces'), true);
  assert.equal(tokenValue(''), false);
  assert.equal(tokenValue('A mod token with spaces'), true);
  assert.equal(textValue('bad\0name'), false);
  assert.equal(nullableValue(tokenValue)(null), true);
  assert.equal(nullableValue(tokenValue)(undefined), false);
});

test('raw integer strings retain the exact signed 64-bit boundary', () => {
  for (const value of ['0', '-0', '9223372036854775807', '-9223372036854775808']) assert.equal(raw64Value(value), true);
  for (const value of ['9223372036854775808', '-9223372036854775809', '1.5', '1e5', '', '+1', 0, null]) assert.equal(raw64Value(value), false);
  assert.equal(pngDataValue('data:image/png;base64,synthetic'), true);
  assert.equal(pngDataValue('guessed-art-name'), false);
});

test('inclusive history spans admit adjacency and gaps but reject overlaps and invalid hours', () => {
  assert.equal(orderedHourSpans([]), true);
  assert.equal(orderedHourSpans([{ firstHour: 1, lastHour: 2 }, { firstHour: 3, lastHour: 4 }, { firstHour: 7, lastHour: 9, extension: 'value' }]), true);
  for (const value of [
    [{ firstHour: 1, lastHour: 2 }, { firstHour: 2, lastHour: 3 }],
    [{ firstHour: 3, lastHour: 3 }, { firstHour: 1, lastHour: 2 }],
    [{ firstHour: 2, lastHour: 1 }], [{ firstHour: 1.5, lastHour: 2 }],
    [{ firstHour: -2147483649, lastHour: 1 }], [{ firstHour: 1, lastHour: 2147483648 }], null,
  ]) assert.equal(orderedHourSpans(value), false);
});
