import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../dashboard/calendar.ts';
import { captureModeFixed, discardTooltip, head, nextLoad, recordingName, recordsAutomatically, savedText, sessionLength, settingsQueue, spanText,
  spoolText, spoolTotalText, stopPromptVisible, stopQuestion, untilNextMinute, whatToRecord } from './status.ts';
import type { RecorderSettings, Snapshot, UnsavedRecording } from './status.ts';

const CAMPAIGN = '00000000-0000-4000-8000-000000000000';
const PATH = `C:\\Users\\Example\\Battle Plan\\${CAMPAIGN}.battleplan`;
const hour = (year: number, month: number, day: number) => engineHour({ year, month, day, hour: 1 })!;
const MB = 2 ** 20;

function snapshot(change: Partial<Snapshot>): Snapshot {
  return { phase: 'waiting', campaign: null, recording: null, name: null, continued: false, hour: null, firstHour: null, savedBytes: null, spoolBytes: null,
    progress: null, startedAt: null, stopping: false, problem: null, automatic: true, session: null, flag: null, captureMode: null, ...change };
}
const recording = (change: Partial<Snapshot> = {}) => snapshot({ phase: 'recording', campaign: CAMPAIGN, recording: PATH, name: 'Germany 1936–1943',
  continued: true, hour: hour(1943, 6, 12), spoolBytes: 1229 * MB, savedBytes: 214 * MB, ...change });

test('every panel state reads as the approved design', () => {
  assert.deepEqual(head(snapshot({})), { glyph: true, flag: null, title: 'Waiting for the game', dot: null, label: null,
    detail: 'Records when you load a campaign', action: null, campaign: false });
  // The spool and the saved part it continues are told apart, never added up.
  assert.deepEqual(head(recording()), { glyph: false, flag: null, title: 'Germany 1936–1943', dot: 'red', label: 'Recording',
    detail: '12 Jun 1943', sizes: 'spool 1.2 GB · saved 214 MB', action: 'stop', campaign: true });
  assert.deepEqual(head(recording({ phase: 'notRecording', hour: hour(1938, 2, 3), spoolBytes: null })), { glyph: false, flag: null,
    title: 'Germany 1936–1943', dot: 'grey', label: null, detail: 'Not recording · 3 Feb 1938', action: 'record', campaign: true });
  assert.deepEqual(head(recording({ phase: 'saving', progress: 0.35 })), { glyph: false, flag: null, title: 'Germany 1936–1943', dot: 'spin',
    label: 'Saving recording…', detail: null, action: null, campaign: false, bar: 0.35 });
  assert.deepEqual(head(recording({ phase: 'saved', firstHour: hour(1936, 1, 1), savedBytes: 412 * MB, spoolBytes: null })), { glyph: false, flag: null,
    title: 'Germany 1936–1943', dot: 'green', label: 'Saved', detail: '1936 – Jun 1943 · 412 MB', action: 'open', campaign: false });
  assert.deepEqual(head(recording({ phase: 'discarded' })), { glyph: false, flag: null, title: 'Germany 1936–1943', dot: 'grey', label: null,
    detail: 'Session discarded', action: 'record', campaign: false });
  // Auto save off and the session over: it waits, under the name its file will get.
  assert.deepEqual(head(recording({ phase: 'unsaved', recording: null, continued: false, savedBytes: null })), { glyph: false, flag: null,
    title: 'Germany 1936–1943', dot: 'grey', label: 'Not saved yet', detail: '12 Jun 1943', sizes: 'spool 1.2 GB', action: 'recordings', campaign: false });
});

test('a recording is named by its file, or the name its file will get, or a placeholder until known, never by its campaign id', () => {
  assert.equal(recordingName('Germany 1936–1943'), 'Germany 1936–1943');
  assert.equal(recordingName(null), 'New recording');
  // A new campaign records into a file that does not exist yet.
  assert.equal(head(recording({ recording: null, continued: false, savedBytes: null })).title, 'Germany 1936–1943');
  assert.equal(head(recording({ recording: null, continued: false, savedBytes: null })).sizes, 'spool 1.2 GB');
  assert.equal(stopQuestion('Germany 1936–1943', 0, (2 * 60 + 14) * 60_000), 'Stop recording Germany 1936–1943 after 2 h 14 min?');
});

test('sizes say whether they are spool or saved, and the spool total never counts the unknown as nothing', () => {
  assert.equal(spoolText(380 * MB), 'spool 380 MB');
  assert.equal(spoolText(1229 * MB), 'spool 1.2 GB');
  assert.equal(savedText(214 * MB), 'saved 214 MB');
  assert.equal(spoolText(null), null);
  assert.equal(savedText(null), null);
  const unsaved = (spoolBytes: number | null): UnsavedRecording => ({ session: 's', campaign: CAMPAIGN, recording: null, name: null, firstHour: null,
    lastHour: null, spoolBytes, status: 'waiting', flag: null });
  assert.equal(spoolTotalText([unsaved(1229 * MB), unsaved(2253 * MB)]), 'Spool on disk: 3.4 GB');
  assert.equal(spoolTotalText([unsaved(1229 * MB), unsaved(null)]), 'Spool on disk: at least 1.2 GB');
  assert.equal(spoolTotalText([unsaved(null)]), null);
  assert.equal(spoolTotalText([]), null);
});

test('loads a session left out are said in one plain line, in place of the usual status text', () => {
  const saved = recording({ phase: 'saved', firstHour: hour(1936, 1, 1), savedBytes: 412 * MB, spoolBytes: null });
  assert.deepEqual(head({ ...saved, omitted: { loads: 1, reason: 'off' } }), { glyph: false, flag: null, title: 'Germany 1936–1943',
    dot: 'green', label: 'Saved', detail: '1 load not recorded (Record automatically was off)', action: 'open', campaign: false });
  assert.equal(head({ ...saved, omitted: { loads: 2, reason: 'elsewhere' } }).detail, '2 loads not recorded (they belong in another file)');
  assert.equal(head({ ...saved, omitted: { loads: 1, reason: 'notAdmitted' } }).detail, '1 load not recorded (it was not identified)');
  assert.equal(head({ ...saved, omitted: { loads: 1, reason: 'settings' } }).detail, "1 load not recorded (the recorder's settings couldn't be read)");
  assert.equal(head({ ...saved, omitted: { loads: 3, reason: null } }).detail, '3 loads not recorded');
  // Nothing saved because every load was left out.
  assert.equal(head(recording({ phase: 'notRecording', omitted: { loads: 1, reason: 'off' } })).detail,
    '1 load not recorded (Record automatically was off)');
  assert.equal(head(snapshot({ phase: 'notRecording', omitted: { loads: 1, reason: 'off' } })).detail,
    '1 load not recorded (Record automatically was off)');
  // Nothing left out: the usual text.
  assert.equal(head({ ...saved, omitted: null }).detail, '1936 – Jun 1943 · 412 MB');
});

test('a resolved flag stands beside the campaign in every state that names it', () => {
  const flag = { width: 2, height: 1, pixels: [255, 0, 0, 255, 0, 0, 255, 255] };
  for (const phase of ['recording', 'notRecording', 'saving', 'saved', 'discarded', 'unsaved'] as const)
    assert.deepEqual(head(recording({ phase, flag })).flag, flag, phase);
  // Waiting names no campaign, so no flag; none resolved means none drawn.
  assert.equal(head(snapshot({ flag })).flag, null);
  assert.equal(head(recording({ flag: null })).flag, null);
});

test('unknown parts are left out rather than guessed', () => {
  // Before the first recorded hour there is no date or size.
  assert.equal(head(recording({ hour: null, spoolBytes: null, savedBytes: null })).detail, null);
  assert.equal(head(recording({ hour: null, spoolBytes: null, savedBytes: null })).sizes, null);
  // A recording not named yet shows no name, never the campaign's id.
  assert.equal(head(recording({ recording: null, name: null })).title, '');
  assert.equal(head(recording({ recording: null, name: null })).glyph, true);
  // Saving with no measure of the remaining work.
  assert.equal(head(recording({ phase: 'saving', progress: null })).bar, null);
  // A game whose campaign is unknown.
  assert.deepEqual(head(snapshot({ phase: 'notRecording' })), { glyph: true, flag: null, title: 'Not recording', dot: null, label: null,
    detail: null, action: 'record', campaign: false });
  // With automatic recording off, Waiting does not promise a recording.
  assert.equal(head(snapshot({ automatic: false })).detail, null);
  // A stop in progress takes the Stop button away.
  assert.equal(head(recording({ stopping: true })).action, null);
  assert.equal(spanText(null, hour(1943, 6, 12)), '12 Jun 1943');
  assert.equal(spanText(null, null), null);
});

test('the Stop prompt asks about this session and says what Discard deletes', () => {
  const started = Date.UTC(2026, 9, 12, 12, 0);
  assert.equal(sessionLength(started, started + (2 * 60 + 14) * 60_000 + 59_000), '2 h 14 min');
  assert.equal(sessionLength(started, started + 3 * 3_600_000), '3 h');
  assert.equal(sessionLength(started, started + 20_000), '1 min');
  assert.equal(sessionLength(null, started), '1 min');
  const now = started + (2 * 60 + 14) * 60_000 + 59_000;
  assert.equal(untilNextMinute(started, now), 1_000);
  assert.equal(sessionLength(started, now + untilNextMinute(started, now)), '2 h 15 min');
  assert.equal(untilNextMinute(started, started + 3 * 3_600_000), 60_000);
  assert.equal(untilNextMinute(null, started), 60_000);
  assert.equal(discardTooltip(true), 'Deletes this session only. Earlier recording in this file is kept.');
  assert.equal(discardTooltip(false), 'Deletes this recording.');
  assert.equal(stopPromptVisible(true, recording()), true);
  assert.equal(stopPromptVisible(false, recording()), false);
  // Saving, a stop in progress, or the recording ending closes the prompt.
  assert.equal(stopPromptVisible(true, recording({ stopping: true })), false);
  assert.equal(stopPromptVisible(true, recording({ phase: 'saving' })), false);
  assert.equal(stopPromptVisible(true, recording({ phase: 'notRecording' })), false);
});

test('campaign choices follow the global default until set', () => {
  const settings: RecorderSettings = { recordEveryCampaign: true, autoSave: true, startWithWindows: false, startWithReplay: true, gameScore: true,
    operationOutcomes: true, politics: true, captureMode: 'full', campaigns: { [CAMPAIGN]: { record: false, next: 'newFile' } }, effectiveSaveFolder: 'C:\\r' };
  assert.equal(recordsAutomatically(settings, CAMPAIGN), false);
  assert.equal(nextLoad(settings, CAMPAIGN), 'newFile');
  assert.equal(recordsAutomatically(settings, 'other'), true);
  assert.equal(recordsAutomatically({ ...settings, recordEveryCampaign: false }, 'other'), false);
  assert.equal(nextLoad(settings, 'other'), 'continue');
  assert.equal(whatToRecord(settings), 'Full');
  assert.equal(whatToRecord({ ...settings, politics: false }), 'Full, custom');
  // Map leaves operation outcomes and politics out whatever their switches say.
  assert.equal(whatToRecord({ ...settings, captureMode: 'map', politics: false }), 'Map');
  assert.equal(whatToRecord({ ...settings, captureMode: 'map', gameScore: false }), 'Map, custom');
});

test('the capture mode is fixed while a session is attached, even at the game menu', () => {
  for (const phase of ['waiting', 'recording', 'saving'] as const)
    assert.equal(captureModeFixed(recording({ phase, captureMode: 'map' })), true, phase);
  for (const phase of ['waiting', 'notRecording', 'saved', 'discarded'] as const)
    assert.equal(captureModeFixed(recording({ phase, captureMode: null })), false, phase);
});

test('settings reads and saves run one at a time, and a failed save is followed by a read', async () => {
  let stored = 'initial';
  const shown: string[] = [];
  const queue = settingsQueue(async () => stored, value => { shown.push(value); });
  let finishFirst!: () => void;
  const firstStored = new Promise<void>(resolve => { finishFirst = resolve; });
  const first = queue.save(() => firstStored.then(() => { stored = 'game score off'; return stored; }));
  let secondSaw: string | undefined;
  const second = queue.save(async () => { secondSaw = stored; stored = 'game score and politics off'; return stored; });
  const read = queue.refresh();
  finishFirst();
  await Promise.all([first, second, read]);
  assert.equal(secondSaw, 'game score off');
  assert.deepEqual(shown, ['game score off', 'game score and politics off', 'game score and politics off']);
  stored = 'stored before the flush failed';
  await assert.rejects(queue.save(async () => { throw new Error('flush failed'); }), /flush failed/);
  await queue.refresh();
  assert.deepEqual(shown.slice(-2), ['stored before the flush failed', 'stored before the flush failed']);
  await queue.save(async () => null);
  assert.equal(shown.length, 5);
});
