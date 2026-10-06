import assert from 'node:assert/strict';
import test from 'node:test';
import { int, object, opaqueTag, signed32, signed64, unsigned32, unsigned64 } from './validation.ts';

test('JSON shape primitives admit objects and safe integers without coercion', () => {
  for (const value of [null, undefined, [], 1, '1', true]) assert.equal(object(value), false);
  assert.equal(object({ missing: null, zero: 0 }), true);
  for (const value of [null, undefined, '0', false, .5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) assert.equal(int(value), false);
  for (const value of [0, -1, Number.MIN_SAFE_INTEGER, Number.MAX_SAFE_INTEGER]) assert.equal(int(value), true);
});

test('native integer operands keep their exact signed and unsigned domains', () => {
  for (const value of [-0x8000_0000, 0, 0x7fff_ffff]) assert.equal(signed32(value), true);
  for (const value of [-0x8000_0001, 0x8000_0000, '0', null, .5]) assert.equal(signed32(value), false);
  for (const value of [0, 0xffff_ffff]) assert.equal(unsigned32(value), true);
  for (const value of [-1, 0x1_0000_0000, '0', null, .5]) assert.equal(unsigned32(value), false);
  for (const value of ['0', '9007199254740993', '18446744073709551615']) assert.equal(unsigned64(value), true);
  for (const value of ['-1', '18446744073709551616', '1.5', '1e3', '', 0, null]) assert.equal(unsigned64(value), false);
});

test('signed recorded integers preserve the full 64-bit range and refuse coercion', () => {
  for (const value of ['0', '-1', '-9223372036854775808', '9223372036854775807', '9007199254740993']) {
    assert.equal(signed64(value), true);
  }
  for (const value of [0, null, '', '1.0', '+1', ' 1', '9223372036854775808', '-9223372036854775809', '1e5']) {
    assert.equal(signed64(value), false);
  }
});

test('opaque country tags accept mod text without assigning tag syntax', () => {
  for (const value of ['Mod country', '国', 'A:|B']) assert.equal(opaqueTag(value), true);
  for (const value of ['', 'A\0B', 123, null]) assert.equal(opaqueTag(value), false);
});
