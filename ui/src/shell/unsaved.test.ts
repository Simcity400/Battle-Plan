import assert from 'node:assert/strict';
import test from 'node:test';
import { leavePrompt, unsavedDetail } from './unsaved.ts';

test('a restart to update goes ahead at once only when nothing is unsaved', () => {
  assert.equal(leavePrompt('update', unsavedDetail([], null)), null);
  assert.deepEqual(leavePrompt('update', unsavedDetail(['Overview'], 'Carbon copy')), {
    title: 'Restart to update without saving?',
    detail: 'Layout changes on Overview and the theme “Carbon copy” aren’t saved.',
    discard: 'Discard and restart',
  });
  assert.deepEqual(leavePrompt('close', unsavedDetail([], '')), {
    title: 'Close BattlePlan without saving?', detail: 'Your new theme isn’t saved.', discard: 'Discard and close',
  });
});

test('the unsaved detail names every page and the theme', () => {
  assert.equal(unsavedDetail(['Overview', 'Economy', 'Air war'], null), 'Layout changes on Overview, Economy and Air war aren’t saved.');
  assert.equal(unsavedDetail([], 'Carbon copy'), 'The theme “Carbon copy” isn’t saved.');
});

test('a settings change still on its way to BattlePlan keeps a close from going ahead unasked', () => {
  assert.deepEqual(leavePrompt('close', unsavedDetail([], null, 'saving')), {
    title: 'Close BattlePlan without saving?', detail: 'A settings change is still being saved.', discard: 'Discard and close',
  });
  assert.equal(unsavedDetail(['Overview'], null, 'saving'), 'Layout changes on Overview aren’t saved. A settings change is still being saved.');
  assert.equal(unsavedDetail([], null, 'failed'), 'A settings change isn’t saved.', 'a change that failed to save, or a folder typed but not applied, is still unsaved work');
});
