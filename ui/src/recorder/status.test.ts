import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../dashboard/calendar.ts';
import { askQuestion, captureModeFixed, discardConsequence, discardDetail, followStatus, head, nextLoad, panelStage, promptTitle, recordingName, recordsAutomatically, savedText, sessionLength,
  settingsQueue, spanText, spoolText, spoolTotalText, stopPromptVisible, stopQuestion, untilNextMinute, waitingDiscardQuestion, whatToRecord } from './status.ts';
import type { RecorderSettings, Snapshot, UnsavedRecording } from './status.ts';

const CAMPAIGN = '00000000-0000-4000-8000-000000000000';
const PATH = `C:\\Users\\Example\\BattlePlan\\${CAMPAIGN}.battleplan`;
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
    detail: 'Records when you load a campaign.', action: null, campaign: false });
  assert.deepEqual(head(recording()), { glyph: false, flag: null, title: 'Germany 1936–1943', dot: 'red', label: 'Recording',
    detail: '12 Jun 1943', sizes: '1.2 GB this session · 214 MB saved', action: 'stop', campaign: true });
  assert.deepEqual(head(recording({ phase: 'notRecording', hour: hour(1938, 2, 3), spoolBytes: null })), { glyph: false, flag: null,
    title: 'Germany 1936–1943', dot: 'grey', label: null, detail: 'Not recording · 3 Feb 1938', action: 'record', campaign: true });
  assert.deepEqual(head(recording({ phase: 'saving', progress: 0.35 })), { glyph: false, flag: null, title: 'Germany 1936–1943', dot: 'spin',
    label: 'Saving recording…', detail: null, action: null, campaign: false, bar: 0.35 });
  assert.deepEqual(head(recording({ phase: 'saved', firstHour: hour(1936, 1, 1), savedBytes: 412 * MB, spoolBytes: null })), { glyph: false, flag: null,
    title: 'Germany 1936–1943', dot: 'green', label: 'Saved', detail: '1936 – Jun 1943 · 412 MB', action: 'open', campaign: false });
  assert.deepEqual(head(recording({ phase: 'discarded' })), { glyph: false, flag: null, title: 'Germany 1936–1943', dot: 'grey', label: null,
    detail: 'Session discarded', action: 'record', campaign: false });
  assert.deepEqual(head(recording({ phase: 'unsaved', recording: null, continued: false, savedBytes: null })), { glyph: false, flag: null,
    title: 'Germany 1936–1943', dot: 'grey', label: 'Not saved yet', detail: '12 Jun 1943', sizes: '1.2 GB this session', action: null, campaign: false });
});

test('a problem with no campaign to name becomes the status under a short headline, and the usual status returns once it clears', () => {
  const problem = { text: 'Another recorder is connected to this game. Close the other recorder to record here.', detail: 'claimed', failed: true };
  const next = { ...problem, text: 'Close the other recorder to record here.' };
  assert.deepEqual(head(snapshot({ problem })), { glyph: true, flag: null, title: 'Another recorder is connected to this game', dot: null, label: null, detail: null,
    action: null, campaign: false, problem: next });
  assert.deepEqual(head(snapshot({ phase: 'notRecording', problem })), { glyph: true, flag: null, title: 'Another recorder is connected to this game', dot: null,
    label: null, detail: null, action: 'record', campaign: false, problem: next });
  const caveat = { text: '2 hours couldn’t be recorded.', detail: '', failed: false };
  assert.equal(head(snapshot({ problem: caveat })).title, 'Waiting for the game', 'only a caveat keeps the idle headline');
  assert.equal(head(snapshot({ phase: 'notRecording', problem: caveat })).title, 'Couldn’t connect to the game');
  assert.equal(head(snapshot({ problem: null })).title, 'Waiting for the game');
  for (const phase of ['notRecording', 'recording', 'saving', 'saved'] as const) {
    const named = head(recording({ phase, problem }));
    assert.equal(named.title, 'Germany 1936–1943', phase);
    assert.equal(named.problem, undefined, phase);
  }
});

test('a recording is named by its file, or the name its file will get, or a placeholder until known, never by its campaign id', () => {
  assert.equal(recordingName('Germany 1936–1943'), 'Germany 1936–1943');
  assert.equal(recordingName(null), 'New recording');
  assert.equal(head(recording({ recording: null, continued: false, savedBytes: null })).title, 'Germany 1936–1943');
  assert.equal(head(recording({ recording: null, continued: false, savedBytes: null })).sizes, '1.2 GB this session');
  assert.equal(stopQuestion('Germany 1936–1943', 0, (2 * 60 + 14) * 60_000), 'Save this session of Germany 1936–1943 (2 h 14 min)?');
  assert.equal(stopQuestion(null, 0, 5 * 60_000), 'Save this session (5 min)?');
  assert.equal(askQuestion(false, false, '2 h 14 min'), 'Save this session (2 h 14 min)?');
  assert.equal(askQuestion(true, false, '2 h 14 min'), 'Save this session (2 h 14 min) before quitting?');
  assert.equal(askQuestion(false, true, '2 h 14 min'), 'Discard this session (2 h 14 min)?');
  assert.equal(askQuestion(true, true, '2 h 14 min'), 'Discard this session (2 h 14 min) and quit?');
  assert.equal(promptTitle(false, false), 'Stop recording?');
  assert.equal(promptTitle(true, false), 'Quit BattlePlan Recorder?');
  assert.equal(promptTitle(false, true), 'Discard this session?');
  assert.equal(promptTitle(true, true), 'Discard and quit?');
});

test('sizes say whether they are this session\'s or saved, and the spool total never counts the unknown as nothing', () => {
  assert.equal(spoolText(380 * MB), '380 MB this session');
  assert.equal(spoolText(1229 * MB), '1.2 GB this session');
  assert.equal(savedText(214 * MB), '214 MB saved');
  assert.equal(spoolText(null), null);
  assert.equal(savedText(null), null);
  const unsaved = (spoolBytes: number | null): UnsavedRecording => ({ session: 's', campaign: CAMPAIGN, recording: null, name: null, firstHour: null,
    lastHour: null, spoolBytes, status: 'waiting', flag: null });
  assert.equal(spoolTotalText([unsaved(1229 * MB), unsaved(2253 * MB)]), '3.4 GB on disk');
  assert.equal(spoolTotalText([unsaved(1229 * MB), unsaved(null)]), 'at least 1.2 GB on disk');
  assert.equal(spoolTotalText([unsaved(null)]), null);
  assert.equal(spoolTotalText([]), null);
});

test('loads a session left out are said in one plain line, in place of the usual status text', () => {
  const saved = recording({ phase: 'saved', firstHour: hour(1936, 1, 1), savedBytes: 412 * MB, spoolBytes: null });
  assert.deepEqual(head({ ...saved, omitted: { loads: 1, reason: 'off' } }), { glyph: false, flag: null, title: 'Germany 1936–1943',
    dot: 'green', label: 'Saved', detail: '1 load not recorded (Auto record was off)', action: 'open', campaign: false });
  assert.equal(head({ ...saved, omitted: { loads: 2, reason: 'elsewhere' } }).detail, '2 loads not recorded (they belong in another file)');
  assert.equal(head({ ...saved, omitted: { loads: 1, reason: 'notAdmitted' } }).detail, '1 load not recorded (it was not identified)');
  assert.equal(head({ ...saved, omitted: { loads: 1, reason: 'settings' } }).detail, '1 load not recorded (the recorder’s settings couldn’t be read)');
  assert.equal(head({ ...saved, omitted: { loads: 3, reason: null } }).detail, '3 loads not recorded');
  assert.equal(head(recording({ phase: 'notRecording', omitted: { loads: 1, reason: 'off' } })).detail,
    '1 load not recorded (Auto record was off)');
  assert.equal(head(snapshot({ phase: 'notRecording', omitted: { loads: 1, reason: 'off' } })).detail,
    '1 load not recorded (Auto record was off)');
  assert.equal(head({ ...saved, omitted: null }).detail, '1936 – Jun 1943 · 412 MB');
});

test('a resolved flag stands beside the campaign in every state that names it', () => {
  const flag = { width: 2, height: 1, pixels: [255, 0, 0, 255, 0, 0, 255, 255] };
  for (const phase of ['recording', 'notRecording', 'saving', 'saved', 'discarded', 'unsaved'] as const)
    assert.deepEqual(head(recording({ phase, flag })).flag, flag, phase);
  assert.equal(head(snapshot({ flag })).flag, null);
  assert.equal(head(recording({ flag: null })).flag, null);
});

test('unknown parts are left out rather than guessed', () => {
  assert.equal(head(recording({ hour: null, spoolBytes: null, savedBytes: null })).detail, null);
  assert.equal(head(recording({ hour: null, spoolBytes: null, savedBytes: null })).sizes, null);
  assert.equal(head(recording({ recording: null, name: null })).title, '');
  assert.equal(head(recording({ recording: null, name: null })).glyph, true);
  assert.equal(head(recording({ phase: 'saving', progress: null })).bar, null);
  assert.deepEqual(head(snapshot({ phase: 'notRecording' })), { glyph: true, flag: null, title: 'Not recording', dot: null, label: null,
    detail: null, action: 'record', campaign: false });
  assert.equal(head(snapshot({ automatic: false })).detail, 'Auto record is off.');
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
  assert.equal(discardConsequence(true), 'The earlier recording in this file is kept.');
  assert.equal(discardConsequence(false), 'This can’t be undone.');
  assert.equal(discardDetail('Germany 1936–1943', started, now), 'Deletes this session’s 2 h 14 min of Germany 1936–1943.');
  assert.equal(discardDetail(null, started, now), 'Deletes this session’s 2 h 14 min.');
  assert.equal(waitingDiscardQuestion(true), 'Discard these hours?');
  assert.equal(waitingDiscardQuestion(false), 'Discard this recording?');
  assert.equal(stopPromptVisible(true, recording()), true);
  assert.equal(stopPromptVisible(false, recording()), false);
  assert.equal(stopPromptVisible(true, recording({ stopping: true })), false);
  assert.equal(stopPromptVisible(true, recording({ phase: 'saving' })), false);
  assert.equal(stopPromptVisible(true, recording({ phase: 'notRecording' })), false);
});

test('every window, the compact stop prompt included, shows the connecting state until it knows the status and its kind', () => {
  for (const compact of [true, false, null]) assert.equal(panelStage(null, compact), 'pending', String(compact));
  assert.equal(panelStage(recording(), null), 'pending');
  assert.equal(panelStage(recording(), true), 'panel');
  assert.equal(panelStage(recording(), false), 'panel');
});

test('campaign choices follow the global default until set', () => {
  const settings: RecorderSettings = { recordEveryCampaign: true, autoSave: true, startWithWindows: false, startWithReplay: true, captureMode: 'full', liveCompression: 'low', autoCompress: true,
    campaigns: { [CAMPAIGN]: { record: false, next: 'newFile' } }, effectiveSaveFolder: 'C:\\r' };
  assert.equal(recordsAutomatically(settings, CAMPAIGN), false);
  assert.equal(nextLoad(settings, CAMPAIGN), 'newFile');
  assert.equal(recordsAutomatically(settings, 'other'), true);
  assert.equal(recordsAutomatically({ ...settings, recordEveryCampaign: false }, 'other'), false);
  assert.equal(nextLoad(settings, 'other'), 'continue');
  assert.equal(whatToRecord(settings), 'Full');
  assert.equal(whatToRecord({ ...settings, captureMode: 'map' }), 'Map');
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

test('the status is read once its listener is in place, and a status published during the read is newer than it', async () => {
  const settle = () => new Promise(resolve => setImmediate(resolve));
  for (const publishedDuringRead of [false, true]) {
    let listening!: (stop: () => void) => void;
    let publish!: (value: string) => void;
    let answer!: (value: string) => void;
    let reads = 0;
    const shown: string[] = [];
    const status = followStatus<string>(handler => { publish = handler; return new Promise(resolve => { listening = resolve; }); },
      () => { reads += 1; return new Promise(resolve => { answer = resolve; }); }, value => { shown.push(value); }, () => assert.fail('nothing failed'));
    await settle();
    assert.equal(reads, 0, 'nothing is read before the listener is in place');
    listening(() => {});
    await settle();
    assert.equal(reads, 1);
    if (publishedDuringRead) publish('saving');
    answer('recording');
    await status.first;
    assert.deepEqual(shown, publishedDuringRead ? ['saving'] : ['recording']);
    publish('saved');
    assert.equal(shown.at(-1), 'saved', 'later statuses keep coming');
  }
});

test('trying the status again listens again before reading, and the attempt it replaces stops showing anything', async () => {
  const settle = () => new Promise(resolve => setImmediate(resolve));
  const handlers: ((value: string) => void)[] = [];
  const failures: unknown[] = [], shown: string[] = [];
  let listens = 0, reads = 0, stopped = 0;
  const status = followStatus<string>(handler => {
    listens += 1;
    if (listens === 1) return Promise.reject(new Error('no events'));
    handlers.push(handler);
    return Promise.resolve(() => { stopped += 1; });
  }, async () => { reads += 1; if (reads === 1) throw new Error('no reply'); return `read ${reads}`; }, value => { shown.push(value); }, cause => { failures.push(cause); });
  await status.first;
  assert.deepEqual([listens, reads, failures.length], [1, 0, 1], 'a listener that couldn’t start is a failure, with nothing read');
  status.retry();
  await settle();
  assert.deepEqual([listens, reads, failures.length, stopped], [2, 1, 2, 0], 'Try again listens again, then reads');
  status.retry();
  await settle();
  assert.equal(stopped, 1, 'the listener from the attempt before is stopped');
  assert.deepEqual([listens, reads, shown], [3, 2, ['read 2']]);
  handlers[0]('stale');
  handlers[1]('saved');
  assert.deepEqual(shown, ['read 2', 'saved'], 'only the current listener shows statuses');
  status.stop();
  await settle();
  assert.equal(stopped, 2);
  handlers[1]('after closing');
  assert.equal(shown.at(-1), 'saved');
});
