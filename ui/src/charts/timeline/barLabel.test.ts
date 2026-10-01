import test from 'node:test';
import assert from 'node:assert/strict';
import { barContent } from './barLabel.ts';

test('a bar shows what its lane asks for, and its name only in place of missing art', () => {
  // Portrait and name; name only.
  assert.deepEqual(barContent(true, true, true), { icon: true, name: true });
  assert.deepEqual(barContent(false, true, true), { icon: false, name: true });
  // Portrait only: the portrait, or the name when the content has no portrait.
  assert.deepEqual(barContent(true, false, true), { icon: true, name: false });
  assert.deepEqual(barContent(true, false, false), { icon: false, name: true });
  // Neither asked for (national spirits history): the bar stays plain, art or not.
  assert.deepEqual(barContent(false, false, true), { icon: false, name: false });
  assert.deepEqual(barContent(false, false, false), { icon: false, name: false });
});
