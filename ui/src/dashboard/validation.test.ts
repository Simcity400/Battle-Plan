import assert from 'node:assert/strict';
import test from 'node:test';
import { object, int } from './validation.ts';

test('JSON shape primitives admit objects and safe integers without coercion', () => {
  for (const value of [null, undefined, [], 1, '1', true]) assert.equal(object(value), false);
  assert.equal(object({ missing: null, zero: 0 }), true);
  for (const value of [null, undefined, '0', false, .5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) assert.equal(int(value), false);
  for (const value of [0, -1, Number.MIN_SAFE_INTEGER, Number.MAX_SAFE_INTEGER]) assert.equal(int(value), true);
});
