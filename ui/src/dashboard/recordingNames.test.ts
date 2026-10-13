import assert from 'node:assert/strict';
import test from 'node:test';
import { loadCampaignSettings } from './campaignSettings.ts';
import { MAX_RECORDING_NAMES_STORAGE, loadRecordingNames, parseRecordingNames, recordingDisplayName, recordingNameChanges,
  recordingNameMatches, recordingNameTooltip, saveRecordingNames, setRecordingName, shareRecordingNames } from './recordingNames.ts';
import type { RecordingNameChanges, RecordingNames } from './recordingNames.ts';

const memory = () => { const values = new Map<string, string>(); return { values, getItem: (key: string) => values.get(key) ?? null,
  setItem: (key: string, value: string) => { values.set(key, value); } }; };
const path = 'C:/recordings/Original.battleplan';

test('display names use campaign path normalization, without changing the file name', () => {
  const names = setRecordingName({}, path, '  Campaign: 1936 / victory?  ');
  for (const equivalent of [path, 'c:\\RECORDINGS\\original.battleplan', '\\\\?\\C:\\recordings\\Original.battleplan']) {
    assert.equal(recordingDisplayName(equivalent, names), 'Campaign: 1936 / victory?');
  }
  assert.deepEqual(names, { 'c:/recordings/original.battleplan': 'Campaign: 1936 / victory?' });
  assert.equal(recordingNameTooltip(path, names), 'Campaign: 1936 / victory? · Original.battleplan');
  assert.equal(recordingDisplayName('C:/elsewhere/Original.battleplan', names), 'Original');
  const unc = setRecordingName(names, '\\\\?\\UNC\\server\\share\\Original.battleplan', 'Network campaign');
  assert.equal(recordingDisplayName('\\\\SERVER\\share\\Original.battleplan', unc), 'Network campaign');
});

test('blank and original file names clear only that recording override', () => {
  const names = setRecordingName(setRecordingName({}, path, 'Display'), 'C:/Other.battleplan', 'Other campaign');
  for (const draft of ['', '   ', 'Original', 'Original.battleplan']) {
    const cleared = setRecordingName(names, path, draft);
    assert.equal(recordingDisplayName(path, cleared), 'Original');
    assert.equal(recordingDisplayName('C:/Other.battleplan', cleared), 'Other campaign');
    assert.ok(!Object.hasOwn(cleared, 'c:/recordings/original.battleplan'));
  }
});

test('recording search matches both the display name and complete file name', () => {
  const names = setRecordingName({}, path, 'Weekend campaign');
  for (const query of ['weekEND', ' original ', '.battleplan', '']) assert.equal(recordingNameMatches(path, query, names), true);
  assert.equal(recordingNameMatches(path, 'missing', names), false);
});

test('names persist separately while old campaign preferences load unchanged', () => {
  const storage = memory();
  const old = JSON.stringify({ version: 1, campaigns: { ab: { factions: { 3: { colour: '#123456' } } } }, sets: [] });
  storage.setItem('hoi4-replay.campaign-settings.v1', old);
  assert.deepEqual(loadRecordingNames(storage), {});
  const before = loadCampaignSettings(storage);
  assert.equal(saveRecordingNames(setRecordingName({}, path, 'Weekend campaign'), storage), null);
  assert.equal(recordingDisplayName('c:\\recordings\\ORIGINAL.battleplan', loadRecordingNames(storage)), 'Weekend campaign');
  assert.equal(storage.getItem('hoi4-replay.campaign-settings.v1'), old);
  assert.deepEqual(loadCampaignSettings(storage), before);
  assert.equal(saveRecordingNames(setRecordingName(loadRecordingNames(storage), path, ''), storage), null);
  assert.equal(recordingDisplayName(path, loadRecordingNames(storage)), 'Original');
});

test('malformed saved names are dropped individually; missing or invalid settings use file names', () => {
  assert.deepEqual(parseRecordingNames({ [path]: 'Good', 'C:/Empty.battleplan': '  ', '': 'bad', 'bad\0path': 'bad',
    'C:/Other.battleplan': 4, 'C:/Null.battleplan': null, 'C:/Own.battleplan': 'Own' }), { 'c:/recordings/original.battleplan': 'Good' });
  for (const raw of [null, '{', '[]', 'null', '0', ' '.repeat(MAX_RECORDING_NAMES_STORAGE + 1)]) {
    const names = loadRecordingNames({ getItem: () => raw, setItem() {} });
    assert.deepEqual(names, {});
    assert.equal(recordingDisplayName(path, names), 'Original');
  }
});

test('5,000 recordings and long display names fit without count or name caps', () => {
  const name = 'Campaign '.repeat(1000);
  const names = parseRecordingNames(Object.fromEntries(Array.from({ length: 5000 }, (_, index) => [`C:/${index}.battleplan`, `Display ${index}`])));
  const storage = memory();
  assert.equal(saveRecordingNames(setRecordingName(names, path, name), storage), null);
  const loaded = loadRecordingNames(storage);
  assert.equal(Object.keys(loaded).length, 5001);
  assert.equal(recordingDisplayName(path, loaded), name.trim());
  assert.equal(recordingDisplayName('C:/4999.battleplan', loaded), 'Display 4999');
});

test('the complete settings storage budget is accepted; excess and device failures are reported', () => {
  const storage = memory();
  const overhead = JSON.stringify(setRecordingName({}, path, 'x')).length - 1;
  const names = setRecordingName({}, path, 'x'.repeat(MAX_RECORDING_NAMES_STORAGE - overhead));
  assert.equal(saveRecordingNames(names, storage), null);
  assert.deepEqual(loadRecordingNames(storage), names);
  assert.match(saveRecordingNames(setRecordingName(names, path, `${names['c:/recordings/original.battleplan']}x`), storage)!, /could not be saved/);
  assert.deepEqual(loadRecordingNames(storage), names, 'a refused save leaves the last persisted names intact');
  assert.match(saveRecordingNames({}, { getItem: () => null, setItem: () => { throw new Error('full'); } })!, /until the app closes/);
});

test('name changes list only the recordings whose names were set or cleared', () => {
  const before = { 'c:/a.battleplan': 'A', 'c:/b.battleplan': 'B', 'c:/c.battleplan': 'C' };
  assert.deepEqual(recordingNameChanges(before, { 'c:/a.battleplan': 'A', 'c:/b.battleplan': 'Renamed', 'c:/d.battleplan': 'D' }),
    { 'c:/b.battleplan': 'Renamed', 'c:/c.battleplan': null, 'c:/d.battleplan': 'D' });
  assert.deepEqual(recordingNameChanges(before, { ...before }), {});
});

// Last in this file: sharing changes where every later save goes.
test('desktop names move once into the shared settings, and each save there sends only what it changes', async () => {
  const key = 'battle-plan.recording-names.v1';
  const values = new Map<string, string>([[key, JSON.stringify({ 'C:/recordings/Local.battleplan': 'Local name',
    'C:/recordings/Only here.battleplan': 'Only on this device' })]]);
  const storage = { getItem: (item: string) => values.get(item) ?? null, setItem: (item: string, value: string) => { values.set(item, value); },
    removeItem: (item: string) => { values.delete(item); } };
  const saved: RecordingNameChanges[] = [];
  const save = async (changes: RecordingNameChanges) => { saved.push(changes); };
  const stored = { 'c:/recordings/shared.battleplan': 'Shared name', 'c:/recordings/local.battleplan': 'Shared wins' };
  await shareRecordingNames(stored, save, storage);
  // Only the name the shared settings lack is sent; the shared one wins where both exist.
  assert.deepEqual(saved, [{ 'c:/recordings/only here.battleplan': 'Only on this device' }]);
  assert.equal(values.has(key), false, 'this device no longer holds the names');
  const names: RecordingNames = { ...stored, 'c:/recordings/only here.battleplan': 'Only on this device' };
  const reports: (string | null)[] = [];
  // A rename sends that one recording's name, never the rest of this session's names, so
  // a name Battle Plan Recorder's settings gained meanwhile survives.
  const renamed = setRecordingName(names, 'C:/recordings/Shared.battleplan', 'Renamed');
  assert.equal(saveRecordingNames(renamed, storage, error => reports.push(error)), null);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(saved.at(-1), { 'c:/recordings/shared.battleplan': 'Renamed' });
  assert.equal(saveRecordingNames(setRecordingName(renamed, 'C:/recordings/Local.battleplan', ''), storage, error => reports.push(error)), null);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(saved.at(-1), { 'c:/recordings/local.battleplan': null });
  assert.deepEqual(reports, [null, null]);
  assert.equal(values.size, 0, 'nothing is written to this device');
  // A later start finds nothing on this device to move.
  await shareRecordingNames(stored, save, storage);
  assert.equal(saved.length, 3);
  // A failed shared save is reported; the name still applies for this session.
  await shareRecordingNames({}, async () => { throw new Error('denied'); }, storage);
  const failures: (string | null)[] = [];
  assert.equal(saveRecordingNames({ 'c:/recordings/shared.battleplan': 'Kept' }, storage, error => failures.push(error)), null);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.match(failures[0]!, /could not be saved/);
});
