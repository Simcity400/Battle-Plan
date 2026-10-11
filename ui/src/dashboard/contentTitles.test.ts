import assert from 'node:assert/strict';
import { test } from 'node:test';
import { contentTitle, contentTitleHint } from './contentTitles.ts';

test('content titles distinguish localized, empty, pending and unavailable names', () => {
  assert.equal(contentTitle('Authored mod title', 'mod_focus'), 'Authored mod title');
  assert.equal(contentTitle('', 'mod_focus'), '');
  assert.equal(contentTitle('mod_focus', 'mod_focus'), 'mod_focus', 'an authored title may equal its key');
  assert.equal(contentTitle(undefined, 'mod_focus', 'Focus name unavailable', true), 'Loading name…');
  assert.equal(contentTitle(null, 'mod_focus', 'Focus name unavailable'), 'Focus name unavailable');
  assert.equal(contentTitleHint('mod_focus', 'Missing recorded scope'), 'Key: mod_focus\nMissing recorded scope');
  assert.equal(contentTitleHint(0), 'Key: 0');
});
