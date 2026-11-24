import assert from 'node:assert/strict';
import test from 'node:test';
import { acceptsLaneClick } from './marks.ts';

test('the first click expands once; repeated double-click clicks and drags do not toggle', () => {
  assert.equal(acceptsLaneClick(1, false), true);
  assert.equal(acceptsLaneClick(0, false), true);
  for (const detail of [2, 3]) assert.equal(acceptsLaneClick(detail, false), false);
  assert.equal(acceptsLaneClick(1, true), false);
});



