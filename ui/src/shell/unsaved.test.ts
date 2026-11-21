import assert from 'node:assert/strict';
import test from 'node:test';
import { leavePrompt, unsavedDetail } from './unsaved.ts';

test('a restart to update goes ahead at once only when nothing is unsaved', () => {
  assert.equal(leavePrompt('update', unsavedDetail([], null)), null);
  assert.deepEqual(leavePrompt('update', unsavedDetail(['Overview'], 'Carbon copy')), {
    title: 'Restart to update without saving?',
    detail: 'Layout changes on Overview and the theme “Carbon copy” aren’t saved.',
    discard: 'Discard and restart',
    failed: 'Couldn’t restart to update.',
  });
  assert.deepEqual(leavePrompt('close', unsavedDetail([], '')), {
    title: 'Close BattlePlan without saving?', detail: 'Your new theme isn’t saved.', discard: 'Discard and close', failed: 'Couldn’t close BattlePlan.',
  });
});

test('the unsaved detail names every page and the theme', () => {
  assert.equal(unsavedDetail(['Overview', 'Economy', 'Air war'], null), 'Layout changes on Overview, Economy and Air war aren’t saved.');
  assert.equal(unsavedDetail([], 'Carbon copy'), 'The theme “Carbon copy” isn’t saved.');
});
