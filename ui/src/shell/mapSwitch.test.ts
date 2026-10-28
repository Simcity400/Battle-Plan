import test from 'node:test';
import assert from 'node:assert/strict';
import { newestSwitch } from './mapSwitch.ts';

test('a map switch keeps the newest value the host numbered, whatever order they arrive in', () => {
  const shown: boolean[] = [];
  const take = newestSwitch(value => shown.push(value));
  assert.equal(take({ value: false, sequence: 0 }), true, 'the first read');
  assert.equal(take({ value: true, sequence: 2 }), true, 'a later change');
  assert.equal(take({ value: false, sequence: 1 }), false, 'an earlier change announced after it');
  assert.equal(take({ value: true, sequence: 2 }), true, 'the same change again, as a reply');
  assert.deepEqual(shown, [false, true, true]);
});
