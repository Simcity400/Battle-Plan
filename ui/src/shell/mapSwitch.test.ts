import test from 'node:test';
import assert from 'node:assert/strict';
import { newestSwitch, type MapSwitch } from './mapSwitch.ts';

test('a map switch keeps the newest publication the host numbered, with its warning, whatever order they arrive in', () => {
  const shown: MapSwitch[] = [];
  const take = newestSwitch(next => shown.push(next));
  assert.equal(take({ value: false, sequence: 0, warning: null }), true, 'the first read');
  assert.equal(take({ value: true, sequence: 2, warning: 'The change was made, but Windows didn’t confirm it was saved' }), true, 'a later change');
  assert.equal(take({ value: false, sequence: 1, warning: null }), false, 'an earlier change announced after it');
  assert.equal(take({ value: true, sequence: 2, warning: null }), false, 'the same publication again, as a reply or a stale read, changes nothing');
  assert.deepEqual(shown.map(({ value, warning }) => [value, warning]), [[false, null], [true, 'The change was made, but Windows didn’t confirm it was saved']], 'so a read can’t clear the warning its publication carried');
});
