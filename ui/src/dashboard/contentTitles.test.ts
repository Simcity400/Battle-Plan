import { RefusedError } from './queryError.ts';
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { contentTitle } from './contentTitles.ts';



test('a failed title read stays distinct from an absent or empty title', () => {
  assert.equal(contentTitle(new RefusedError('Synthetic title was not recorded'), 'synthetic'), 'Synthetic title was not recorded');
  assert.equal(contentTitle(new Error('Synthetic decoder detail'), 'synthetic'), 'Couldn’t load name.');
  assert.equal(contentTitle(null, 'synthetic'), 'Name unavailable');
  assert.equal(contentTitle('', 'synthetic'), '');
});
