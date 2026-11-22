import test from 'node:test';
import assert from 'node:assert/strict';
import { actionFailure, failureText, plainFailure, queryFailure } from './queryError.ts';

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
