import assert from 'node:assert/strict';
import test from 'node:test';
import { recordingDisplayName, recordingFilename, recordingNameMatches } from './recordingNames.ts';

test("a recording's name is its file name without the extension", () => {
  const path = String.raw`C:\Recordings\Germany 1936–1941 (Multiplayer) (2).battleplan`;
  assert.equal(recordingFilename(path), 'Germany 1936–1941 (Multiplayer) (2).battleplan');
  assert.equal(recordingDisplayName(path), 'Germany 1936–1941 (Multiplayer) (2)');
  assert.equal(recordingDisplayName('D:/elsewhere/Operation.Sealion.battleplan'), 'Operation.Sealion');
});

test('search matches the file name, extension included, in any case', () => {
  const path = 'C:/Recordings/Germany 1936–1941.battleplan';
  for (const query of ['germany', ' 1936–1941 ', '.battleplan', '']) assert.equal(recordingNameMatches(path, query), true, query);
  assert.equal(recordingNameMatches(path, 'Recordings'), false, 'the folder is not the name');
  assert.equal(recordingNameMatches(path, 'italy'), false);
});
