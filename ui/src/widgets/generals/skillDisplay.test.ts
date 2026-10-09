import assert from 'node:assert/strict';
import { test } from 'node:test';
import { compactSkill } from './skillDisplay.ts';

test('stars use measured cell space and large mod ceilings use a compact count', () => {
  assert.equal(compactSkill(6, 72, 10), false);
  assert.equal(compactSkill(6, 48, 10), true);
  assert.equal(compactSkill(5000, 72, 10), true);
  assert.equal(compactSkill(5000, 60000, 10), false);
  assert.equal(compactSkill(6, 0, 10), true);
});
