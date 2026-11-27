import test from 'node:test';
import assert from 'node:assert/strict';
import { actionFailure, classifyFailure, failureText, notInRecordingSentence, plainFailure, queryFailure, RefusedError, ShownError } from './queryError.ts';
import { panelReply } from './follow/panel.ts';
import { Page } from './follow/page.ts';

test('a query the transport rejects with a sentence shows that sentence, not an invalid reply', () => {
  const lost = 'BattlePlan couldn’t reach its desktop host for dashboard_product: the IPC request failed (TypeError: Failed to fetch).';
  const failure = queryFailure(lost);
  assert.equal(failureText(failure, 'generic'), 'BattlePlan couldn’t reach its desktop host for dashboard_product.');
  assert.equal(failure.message, lost);
});

test('a failure shows its plain sentence and keeps the technical reason as detail, whatever its case', () => {
  const folder = 'Couldn’t read the recordings folder: Access is denied. (os error 5)';
  assert.deepEqual(plainFailure(folder), { text: 'Couldn’t read the recordings folder.', detail: folder });
  const settings = 'Recorder settings couldn’t be read: missing field';
  assert.deepEqual(plainFailure(settings), { text: 'Recorder settings couldn’t be read.', detail: settings });
  const lost = 'The map lost its graphics device again before it was ready, so it wasn’t started again: The map lost its graphics device: Hung';
  assert.equal(plainFailure(lost).text, 'The map lost its graphics device again before it was ready, so it wasn’t started again.');
  assert.deepEqual(plainFailure('The map’s window (handle: 3) is gone'), { text: 'The map’s window (handle: 3) is gone' }, 'a colon inside brackets is not the split');
  assert.deepEqual(plainFailure('This folder is too large. Choose a folder that holds only your recordings.'),
    { text: 'This folder is too large. Choose a folder that holds only your recordings.' });
});

test('a failure leads with the raw head only when that head is a product sentence; otherwise the action names it', () => {
  const pathLed = 'D:/Recordings/Germany 1936.battleplan: Access is denied. (os error 5)';
  assert.deepEqual(plainFailure(pathLed), { text: pathLed }, 'a path is never the sentence');
  assert.deepEqual(actionFailure('Couldn’t save the recording', pathLed), { text: 'Couldn’t save the recording.', detail: pathLed });
  assert.deepEqual(actionFailure('Couldn’t open the recording.', 'Germany 1936.battleplan: in use'), { text: 'Couldn’t open the recording.', detail: 'Germany 1936.battleplan: in use' });
  assert.deepEqual(actionFailure('Couldn’t export the map', 'The map isn’t ready yet'), { text: 'Couldn’t export the map.', detail: 'The map isn’t ready yet' },
    'a fragment without its full stop is detail');
  assert.deepEqual(actionFailure('Couldn’t export the map', 'invalid args `viewId` for command `export_map`'), { text: 'Couldn’t export the map.', detail: 'invalid args `viewId` for command `export_map`' });
  const led = 'Couldn’t save the map image: Capture map.png failed';
  assert.deepEqual(actionFailure('Couldn’t export the map', led), { text: 'Couldn’t save the map image.', detail: led });
  assert.deepEqual(actionFailure('Couldn’t rename the recording', 'This recording is in use.'), { text: 'This recording is in use.' });
  assert.deepEqual(actionFailure('Couldn’t open BattlePlan', 'Access is denied. (os error 5)'), { text: 'Couldn’t open BattlePlan.', detail: 'Access is denied. (os error 5)' });
});

test('failure classification inspects only error types, never message text, and formats all seven refusal causes', () => {
  const refused = classifyFailure(new RefusedError('Not recorded in Map mode', { kind: 'capture-mode', mode: 'Map' }));
  assert.equal(refused?.kind, 'refused');
  assert.equal(refused?.kind === 'refused' ? refused.cause?.kind : null, 'capture-mode');

  const shown = classifyFailure(new ShownError('Desktop host disconnected.', 'raw stack'));
  assert.deepEqual(shown, { kind: 'retryable', reason: 'Desktop host disconnected.', detail: 'raw stack' });

  const unclassified = classifyFailure(new Error('No table convoy_raids in the schema'));
  assert.deepEqual(unclassified, { kind: 'retryable', detail: 'No table convoy_raids in the schema' });

  assert.equal(notInRecordingSentence({ kind: 'capture-mode' }, 'factories'),
    'This is a Map recording, which holds the campaign map and battle windows only.');
  assert.equal(notInRecordingSentence({ kind: 'older-format' }, 'research'),
    'This recording was made before BattlePlan recorded research.');
  assert.equal(notInRecordingSentence({ kind: 'format-changed' }, 'division equipment'),
    'BattlePlan changed how it records division equipment after this recording was made, so it isn’t shown.');
  assert.equal(notInRecordingSentence({ kind: 'mod', modName: 'Kaiserreich', what: 'focus trees' }, 'national focus'),
    'Kaiserreich changes focus trees in a way BattlePlan can’t read yet.');
  assert.equal(notInRecordingSentence({ kind: 'dlc', dlcName: 'No Step Back' }, 'tank designs'),
    'This campaign was played without No Step Back, which tank designs needs.');
  assert.equal(notInRecordingSentence({ kind: 'game-build', version: '1.15.4' }, 'army losses'),
    'Army losses couldn’t be checked against game version 1.15.4, so it was withheld.');
  assert.equal(notInRecordingSentence({ kind: 'damage' }, 'battles'),
    'Part of this recording is damaged, so battles is withheld.');
});

test('follow panel and page refusals preserve the refusal type instead of turning into retryable failures', () => {
  const reply = panelReply({ data: undefined, refused: 'Table absent in format 141' }, value => value);
  assert.ok(reply.error instanceof RefusedError, 'panelReply keeps RefusedError for refused panels');
  assert.equal(classifyFailure(reply.error)?.kind, 'refused');

  const page = new Page();
  page.mount(1, { route: 1, lane: 0, surfaces: [] });
  page.name(1, 'refused', 'SupplyHealth is not in this playhead');
  assert.equal(page.view(1).mode, 'refused');
  assert.equal(page.view(1).reason, 'SupplyHealth is not in this playhead');
});
