import assert from 'node:assert/strict';
import test from 'node:test';
import { distinguishingFolder, distinguishingFolders, qualifiedRecordingName, recordingDisplayName, recordingFilename, recordingNameMatches } from './recordingNames.ts';

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

test('recordings that share a name are told apart by the shortest folder that differs, and others carry no folder', () => {
  const paths = [String.raw`C:\Recordings\runs\first\Italy 1936.battleplan`, 'C:/Recordings/runs/second/italy 1936.battleplan',
    String.raw`C:\Recordings\Italy 1936.battleplan`, String.raw`C:\Recordings\Germany.battleplan`,
    String.raw`D:\Old\first\Japan.battleplan`, String.raw`E:\New\first\Japan.battleplan`];
  const folders = distinguishingFolders(paths);
  assert.deepEqual(paths.map(path => distinguishingFolder(folders, path)), ['first', 'second', 'Recordings', null, String.raw`Old\first`, String.raw`New\first`]);
  assert.equal(distinguishingFolders([paths[0], paths[0].replaceAll('\\', '/')]).size, 0, 'one recording listed twice is not a collision');
  assert.equal(qualifiedRecordingName(folders, paths[0]), 'Italy 1936 in first');
  assert.equal(qualifiedRecordingName(folders, paths[3]), 'Germany');
});
