import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../dashboard/calendar.ts';
import { askedPrompt, askQuestion, discardConsequence, discardDetail, escapeStep, followStatus, head, nextLoad, PANEL_UNOPENED, panelStage, pendingFailure, promptDiscards, promptTitle, stopConfirmed, recordingName, recordsAutomatically, savedText, sessionLength, settingsQueue, spanText, spoolText, spoolTotalText, stopPromptVisible, stopQuestion, untilNextMinute, waitingDiscardQuestion, whatToRecord } from './status.ts';
import type { RecorderSettings, Snapshot, UnsavedRecording } from './status.ts';

const CAMPAIGN = '00000000-0000-4000-8000-000000000000';
const PATH = `C:\\Users\\Example\\BattlePlan\\${CAMPAIGN}.battleplan`;
const hour = (year: number, month: number, day: number) => engineHour({ year, month, day, hour: 1 })!;
const MB = 2 ** 20;

function snapshot(change: Partial<Snapshot>): Snapshot {
  return { phase: 'waiting', campaign: null, recording: null, name: null, continued: false, hour: null, firstHour: null, savedBytes: null, spoolBytes: null,
    startedAt: null, stopping: false, problem: null, automatic: true, session: null, flag: null, captureMode: null, revision: 0, ...change };
}
const recording = (change: Partial<Snapshot> = {}) => snapshot({ phase: 'recording', campaign: CAMPAIGN, recording: PATH, name: 'Germany 1936–1943',
  continued: true, hour: hour(1943, 6, 12), spoolBytes: 1229 * MB, savedBytes: 214 * MB, ...change });


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

test('Escape never seems to cancel a Save or Discard already sent', () => {
  const screen = { updateAsked: false, confirmingWaiting: false, prompting: true, discarding: true, promptBusy: false, up: false };
  assert.equal(escapeStep(screen), 'discard', 'Escape steps back from the Discard confirmation');
  assert.equal(escapeStep({ ...screen, promptBusy: true }), 'none', 'not once the Discard is on its way');
  assert.equal(escapeStep({ ...screen, discarding: false, promptBusy: true }), 'none');
  assert.equal(escapeStep({ ...screen, discarding: false }), 'prompt');
  assert.equal(escapeStep({ ...screen, prompting: false, promptBusy: true, up: true }), 'up');
  assert.equal(escapeStep({ ...screen, prompting: false }), 'close');
  assert.equal(escapeStep({ ...screen, updateAsked: true, promptBusy: true }), 'update');
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



test('unknown parts are left out rather than guessed', () => {
  assert.equal(head(recording({ hour: null, spoolBytes: null, savedBytes: null })).detail, null);
  assert.equal(head(recording({ hour: null, spoolBytes: null, savedBytes: null })).sizes, null);
  assert.equal(head(recording({ recording: null, name: null })).title, '');
  assert.equal(head(recording({ recording: null, name: null })).glyph, true);
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

test('a panel still connecting shows a window it couldn’t fit, with that failure’s Try again', () => {
  const retry = () => {};
  const unshown = { text: 'Couldn’t show the recorder panel.', detail: 'the request failed', retry };
  assert.deepEqual(pendingFailure(null, unshown), { title: unshown.text, detail: unshown.detail, retry });
  const unopened = { title: PANEL_UNOPENED, detail: 'the recorder is not running', retry() {} };
  assert.equal(pendingFailure(unopened, unshown), unopened, 'a panel that can’t reach the recorder says so first');
  assert.equal(pendingFailure(null, null), null);
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


test('a settings save is sent at once, overlapping saves end in one read of what is stored, and each warning is shown', async () => {
  type Stored = { value: string; warning?: string | null };
  let stored: Stored = { value: 'initial' };
  const shown: string[] = [], warned: string[] = [];
  const queue = settingsQueue<Stored>(async () => stored, value => { shown.push(value.value); }, warning => { warned.push(warning); });
  let finishFirst!: () => void;
  const firstStored = new Promise<void>(resolve => { finishFirst = resolve; });
  let secondSent = false;
  const first = queue.save(() => firstStored.then(() => { stored = { value: 'game score and politics off' }; return { value: 'game score off', warning: 'the disk didn’t answer' }; }));
  const second = queue.save(async () => { secondSent = true; stored = { value: 'politics off' }; return stored; });
  assert.equal(secondSent, true, 'nothing waits in the page behind an earlier save, so a tray Quit can’t overtake it');
  await second;
  assert.deepEqual(shown, [], 'a reply while another save is out shows nothing yet');
  finishFirst();
  await first;
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(shown, ['game score and politics off'], 'once both are answered the panel reads what is stored, whichever committed first');
  assert.deepEqual(warned, ['the disk didn’t answer'], 'and the first save’s warning is shown though its snapshot isn’t');
  await queue.save(async () => ({ value: 'alone' }));
  assert.deepEqual(shown.at(-1), 'alone', 'a save on its own shows its reply');
  stored = { value: 'stored before the flush failed' };
  await assert.rejects(queue.save(async () => { throw new Error('flush failed'); }), /flush failed/);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(shown.at(-1), 'stored before the flush failed', 'a failed save is followed by a read of what is stored');

  let finishRead!: () => void;
  const slowRead = new Promise<void>(resolve => { finishRead = resolve; });
  const unread: unknown[] = [], laterShown: string[] = [];
  const later = settingsQueue<Stored>(async () => { await slowRead; return { value: 'read before the later save' }; }, value => { laterShown.push(value.value); }, () => {}, cause => { unread.push(cause); });
  await assert.rejects(later.save(async () => { throw new Error('refused'); }));
  await later.save(async () => ({ value: 'the later save' }));
  finishRead();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(laterShown.at(-1), 'the later save', 'a read that started before a later save never replaces what that save showed');
  const failing = settingsQueue<Stored>(async () => { throw new Error('locked'); }, () => {}, () => {}, cause => { unread.push(cause); });
  await assert.rejects(failing.save(async () => { throw new Error('refused'); }));
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(unread.length, 1, 'and a read that fails after the saves says so');

  let failRead!: () => void;
  const failingLater = new Promise<never>((_, reject) => { failRead = () => reject(new Error('locked')); });
  const superseded = settingsQueue<Stored>(() => failingLater, () => {}, () => {}, cause => { unread.push(cause); });
  await assert.rejects(superseded.save(async () => { throw new Error('refused'); }));
  await superseded.save(async () => ({ value: 'newer', warning: 'the disk didn’t answer' }));
  failRead();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(unread.length, 1, 'a read a later save superseded reports no failure over that save');
});

interface Numbered { revision: number; phase: string }
const numbered = (revision: number, phase: string): Numbered => ({ revision, phase });
const phases = (shown: Numbered[]) => shown.map(({ phase }) => phase);

test('the status is read once its listener is in place, and a status published during the read is newer than it', async () => {
  const settle = () => new Promise(resolve => setImmediate(resolve));
  for (const publishedDuringRead of [false, true]) {
    let listening!: (stop: () => void) => void;
    let publish!: (value: Numbered) => void;
    let answer!: (value: Numbered) => void;
    let reads = 0;
    const shown: Numbered[] = [];
    const status = followStatus<Numbered>(handler => { publish = handler; return new Promise(resolve => { listening = resolve; }); },
      () => { reads += 1; return new Promise(resolve => { answer = resolve; }); }, value => { shown.push(value); }, () => assert.fail('nothing failed'),
      value => value.revision);
    await settle();
    assert.equal(reads, 0, 'nothing is read before the listener is in place');
    listening(() => {});
    await settle();
    assert.equal(reads, 1);
    if (publishedDuringRead) publish(numbered(5, 'saving'));
    answer(numbered(4, 'recording'));
    await status.first;
    assert.deepEqual(phases(shown), publishedDuringRead ? ['saving'] : ['recording']);
    publish(numbered(6, 'saved'));
    assert.equal(shown.at(-1)?.phase, 'saved', 'later statuses keep coming');
  }
});

test('a status the recorder published before the one shown never replaces it', async () => {
  let publish!: (value: Numbered) => void;
  const shown: Numbered[] = [];
  const status = followStatus<Numbered>(handler => { publish = handler; return Promise.resolve(() => {}); }, async () => numbered(1, 'recording'),
    value => { shown.push(value); }, () => assert.fail('nothing failed'), value => value.revision);
  await status.first;
  publish(numbered(3, 'asked to quit'));
  publish(numbered(2, 'recording'));
  assert.deepEqual(phases(shown), ['recording', 'asked to quit'], 'an older status sent after a newer one is ignored, so the newer prompt stays');
  publish(numbered(3, 'asked to quit'));
  assert.equal(shown.length, 2, 'nor is the same status shown twice');
});

test('trying the status again listens again before reading, and the attempt it replaces stops showing anything', async () => {
  const settle = () => new Promise(resolve => setImmediate(resolve));
  const handlers: ((value: Numbered) => void)[] = [];
  const failures: unknown[] = [], shown: Numbered[] = [];
  let listens = 0, reads = 0, stopped = 0;
  const status = followStatus<Numbered>(handler => {
    listens += 1;
    if (listens === 1) return Promise.reject(new Error('no events'));
    handlers.push(handler);
    return Promise.resolve(() => { stopped += 1; });
  }, async () => { reads += 1; if (reads === 1) throw new Error('no reply'); return numbered(reads, `read ${reads}`); }, value => { shown.push(value); },
  cause => { failures.push(cause); }, value => value.revision);
  await status.first;
  assert.deepEqual([listens, reads, failures.length], [1, 0, 1], 'a listener that couldn’t start is a failure, with nothing read');
  status.retry();
  await settle();
  assert.deepEqual([listens, reads, failures.length, stopped], [2, 1, 2, 0], 'Try again listens again, then reads');
  status.retry();
  await settle();
  assert.equal(stopped, 1, 'the listener from the attempt before is stopped');
  assert.deepEqual([listens, reads, phases(shown)], [3, 2, ['read 2']]);
  handlers[0](numbered(8, 'stale'));
  handlers[1](numbered(9, 'saved'));
  assert.deepEqual(phases(shown), ['read 2', 'saved'], 'only the current listener shows statuses');
  status.stop();
  await settle();
  assert.equal(stopped, 2);
  handlers[1](numbered(10, 'after closing'));
  assert.equal(shown.at(-1)?.phase, 'saved');
});

test('the stop prompt asks what the recorder asked about the live session, or what the panel’s own Stop asked, and nothing once it stops', () => {
  const live = recording({ sessionKey: 3, prompt: null });
  assert.equal(askedPrompt(live, null), null);
  assert.deepEqual(askedPrompt(live, 3), { quitting: false, discarding: false, sessionKey: 3 }, 'the panel’s own Stop asks about the session it shows');
  const asked = { id: 7, quitting: true, discarding: false, sessionKey: 3 };
  assert.deepEqual(askedPrompt({ ...live, prompt: asked }, null), asked, 'the tray’s question stays until the recorder retires it');
  assert.equal(askedPrompt({ ...live, prompt: asked, stopping: true }, 3), null, 'a stopping session asks nothing');
  assert.equal(askedPrompt({ ...live, sessionKey: null }, 3), null, 'without a live session there is nothing to stop');
  assert.equal(askedPrompt(null, 3), null);
});

test('a Stop and its Discard step confirm only the session they were opened for, so a new session’s status never inherits them', () => {
  const first = recording({ sessionKey: 3, prompt: null });
  const next = recording({ sessionKey: 4, prompt: null });
  assert.equal(promptDiscards(askedPrompt(first, 3), 3), true, 'the Discard step stands for the session it was taken in');
  assert.equal(askedPrompt(next, 3), null, 'a Stop opened for one session asks nothing about the next');
  const quit = { id: 8, quitting: true, discarding: false, sessionKey: 4 };
  const carried = askedPrompt({ ...next, prompt: quit }, 3);
  assert.deepEqual(carried, quit);
  assert.equal(promptDiscards(carried, 3), false, 'the Quit asked again about the new session asks to save, not to discard');
  assert.equal(promptDiscards({ ...quit, discarding: true }, null), true, 'the tray’s Discard is its own confirmation step');
  assert.equal(promptDiscards(null, 3), false);
});

test('a Save or Discard, and its Try again, acts only while the screen still asks it about that session', () => {
  const live = recording({ sessionKey: 3, prompt: null });
  const asked = askedPrompt(live, 3);
  assert.equal(stopConfirmed(asked, 3, true, 3), true, 'the confirmed Discard step discards');
  assert.equal(stopConfirmed(asked, null, true, 3), false, 'a Discard whose confirmation was cancelled back to Save or Discard is retired, Try again included');
  assert.equal(stopConfirmed(asked, null, false, 3), true, 'the Save on that screen still saves');
  assert.equal(stopConfirmed(asked, 3, false, 3), false, 'the confirmation step offers no Save');
  assert.equal(stopConfirmed(askedPrompt(recording({ sessionKey: 4, prompt: null }), 4), 4, true, 3), false, 'nor does it act on another session');
  assert.equal(stopConfirmed(askedPrompt(live, null), 3, true, 3), false, 'nor once the prompt has closed');
});

