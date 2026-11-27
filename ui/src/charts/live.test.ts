import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from './live.ts';
import { valuesOf } from '../test-support/liveValues.ts';

test('a key the values do not hold is missing with its reason', () => {
  const values = valuesOf(new Map([[liveKey('GER', 'opinion'), 12]]), key => `no ${key.split('\u0000')[0]}`);
  assert.equal(values.get(liveKey('GER', 'opinion')), 12);
  assert.equal(values.get(liveKey('SOV', 'opinion')), undefined);
  assert.equal(values.missing(liveKey('SOV', 'opinion')), 'no SOV');
});

