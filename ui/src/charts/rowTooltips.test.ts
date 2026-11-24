import assert from 'node:assert/strict';
import test from 'node:test';
import { rowDescriptionId } from './rowTooltips.ts';



test('row description ids encode opaque keys without whitespace or collisions', () => {
  const keys = [':token 42', ':token%2042', 'a b', 'a-b', 'a\tb\nc', '🙂', '\ud800', 42, '42', NaN, Infinity, -Infinity];
  const ids = keys.map(key => rowDescriptionId('widget owner', key));
  assert.equal(new Set(ids).size, keys.length);
  for (let index = 0; index < keys.length; index++) {
    assert.match(ids[index], /^\S+$/);
    assert.equal(rowDescriptionId('widget owner', keys[index]), ids[index]);
  }
  assert.notEqual(rowDescriptionId('a-b', 'c'), rowDescriptionId('a', 'b-c'));
  assert.notEqual(rowDescriptionId('other owner', ':token 42'), ids[0]);
});
