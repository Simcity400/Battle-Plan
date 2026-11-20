import assert from 'node:assert/strict';
import test from 'node:test';
import { noticeMatchesAnchor } from './anchorGeneration.ts';

test('a notice from before the current anchor is stale', () => {
  assert.equal(noticeMatchesAnchor(4, 5), false);
  assert.equal(noticeMatchesAnchor(5, 5), true);
});
