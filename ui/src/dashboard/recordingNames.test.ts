import assert from 'node:assert/strict';
import test from 'node:test';
import { distinguishingFolder, distinguishingFolders, qualifiedRecordingName } from './recordingNames.ts';



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
