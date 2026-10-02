import assert from 'node:assert/strict';
import test from 'node:test';
import { isMenuKey, rowDescriptionId, rowPoint } from './rowTooltips.ts';

test('the menu key and Shift+F10 open a row’s menu; F10 alone does not', () => {
  assert.equal(isMenuKey({ key: 'ContextMenu', shiftKey: false }), true);
  assert.equal(isMenuKey({ key: 'F10', shiftKey: true }), true);
  assert.equal(isMenuKey({ key: 'F10', shiftKey: false }), false);
  assert.equal(isMenuKey({ key: 'Enter', shiftKey: true }), false);
});

test('a focused row’s tooltip opens inside its right edge, halfway down', () => {
  assert.deepEqual(rowPoint({ right: 400, top: 100, height: 30 }), { x: 392, y: 115 });
});

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
