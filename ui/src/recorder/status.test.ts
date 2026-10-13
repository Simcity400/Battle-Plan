import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../dashboard/calendar.ts';
import { discardTooltip, head, nextLoad, recordsAutomatically, sessionLength, spanText, stopPromptVisible, whatToRecord } from './status.ts';
import type { RecorderSettings, Snapshot } from './status.ts';

const CAMPAIGN = '00000000-0000-4000-8000-000000000000';
const PATH = `C:\\Users\\Example\\Battle Plan\\${CAMPAIGN}.battleplan`;
const names = { [`c:/users/Example/Battle Plan/${CAMPAIGN}.battleplan`]: 'Operation Barbarossa' };
const hour = (year: number, month: number, day: number) => engineHour({ year, month, day, hour: 1 })!;
const MB = 2 ** 20;

function snapshot(change: Partial<Snapshot>): Snapshot {
  return { phase: 'waiting', campaign: null, recording: null, continued: false, hour: null, firstHour: null, bytes: null, progress: null,
    startedAt: null, stopping: false, problem: null, automatic: true, session: null, flag: null, ...change };
}
const recording = (change: Partial<Snapshot> = {}) => snapshot({ phase: 'recording', campaign: CAMPAIGN, recording: PATH, hour: hour(1943, 6, 12), bytes: 380 * MB, ...change });

test('every panel state reads as the approved design', () => {
  assert.deepEqual(head(snapshot({}), names), { glyph: true, flag: null, title: 'Waiting for the game', dot: null, label: null,
    detail: 'Records when you load a campaign', action: null, campaign: false, footer: true });
  assert.deepEqual(head(recording(), names), { glyph: false, flag: null, title: 'Operation Barbarossa', dot: 'red', label: 'Recording',
    detail: '12 Jun 1943 · 380 MB', action: 'stop', campaign: true, footer: true });
  assert.deepEqual(head(recording({ phase: 'notRecording', hour: hour(1938, 2, 3), bytes: null }), names), { glyph: false, flag: null, title: 'Operation Barbarossa',
    dot: 'grey', label: null, detail: 'Not recording · 3 Feb 1938', action: 'record', campaign: true, footer: true });
  assert.deepEqual(head(recording({ phase: 'saving', progress: 0.35 }), names), { glyph: false, flag: null, title: 'Operation Barbarossa', dot: 'spin',
    label: 'Saving recording…', detail: null, action: null, campaign: false, footer: false, bar: 0.35 });
  assert.deepEqual(head(recording({ phase: 'saved', firstHour: hour(1936, 1, 1), bytes: 412 * MB }), names), { glyph: false, flag: null,
    title: 'Operation Barbarossa', dot: 'green', label: 'Saved', detail: '1936 – Jun 1943 · 412 MB', action: 'open', campaign: false, footer: true });
  assert.deepEqual(head(recording({ phase: 'discarded' }), names), { glyph: false, flag: null, title: 'Operation Barbarossa', dot: 'grey', label: null,
    detail: 'Session discarded', action: 'record', campaign: false, footer: false });
});

test('loads a session left out are said in one plain line, in place of the usual status text', () => {
  const saved = recording({ phase: 'saved', firstHour: hour(1936, 1, 1), bytes: 412 * MB });
  assert.deepEqual(head({ ...saved, omitted: { loads: 1, reason: 'off' } }, names), { glyph: false, flag: null, title: 'Operation Barbarossa',
    dot: 'green', label: 'Saved', detail: '1 load not recorded (Record automatically was off)', action: 'open', campaign: false, footer: true });
  assert.equal(head({ ...saved, omitted: { loads: 2, reason: 'elsewhere' } }, names).detail, '2 loads not recorded (they belong in another file)');
  assert.equal(head({ ...saved, omitted: { loads: 1, reason: 'notAdmitted' } }, names).detail, '1 load not recorded (it was not identified)');
  assert.equal(head({ ...saved, omitted: { loads: 1, reason: 'settings' } }, names).detail, "1 load not recorded (the recorder's settings couldn't be read)");
  assert.equal(head({ ...saved, omitted: { loads: 3, reason: null } }, names).detail, '3 loads not recorded');
  // Nothing saved because every load was left out.
  assert.equal(head(recording({ phase: 'notRecording', omitted: { loads: 1, reason: 'off' } }), names).detail,
    '1 load not recorded (Record automatically was off)');
  assert.equal(head(snapshot({ phase: 'notRecording', omitted: { loads: 1, reason: 'off' } }), names).detail,
    '1 load not recorded (Record automatically was off)');
  // Nothing left out: the usual text.
  assert.equal(head({ ...saved, omitted: null }, names).detail, '1936 – Jun 1943 · 412 MB');
});

test('a resolved flag stands beside the campaign in every state that names it', () => {
  const flag = { width: 2, height: 1, pixels: [255, 0, 0, 255, 0, 0, 255, 255] };
  for (const phase of ['recording', 'notRecording', 'saving', 'saved', 'discarded'] as const)
    assert.deepEqual(head(recording({ phase, flag }), names).flag, flag, phase);
  // Waiting names no campaign, so no flag; none resolved means none drawn.
  assert.equal(head(snapshot({ flag }), names).flag, null);
  assert.equal(head(recording({ flag: null }), names).flag, null);
});

test('unknown parts are left out rather than guessed', () => {
  // Before the first recorded hour there is no date or size.
  assert.equal(head(recording({ hour: null, bytes: null }), names).detail, null);
  // Without a display name the recording's file name names it.
  assert.equal(head(recording(), {}).title, CAMPAIGN);
  // Saving with no measure of the remaining work.
  assert.equal(head(recording({ phase: 'saving', progress: null }), names).bar, null);
  // A game whose campaign is unknown.
  assert.deepEqual(head(snapshot({ phase: 'notRecording' }), names), { glyph: true, flag: null, title: 'Not recording', dot: null, label: null,
    detail: null, action: 'record', campaign: false, footer: true });
  // With automatic recording off, Waiting does not promise a recording.
  assert.equal(head(snapshot({ automatic: false }), names).detail, null);
  // A stop in progress takes the Stop button away.
  assert.equal(head(recording({ stopping: true }), names).action, null);
  assert.equal(spanText(null, hour(1943, 6, 12)), '12 Jun 1943');
  assert.equal(spanText(null, null), null);
});

test('the Stop prompt asks about this session and says what Discard deletes', () => {
  const started = Date.UTC(2026, 9, 12, 12, 0);
  assert.equal(sessionLength(started, started + (2 * 60 + 14) * 60_000 + 59_000), '2 h 14 min');
  assert.equal(sessionLength(started, started + 3 * 3_600_000), '3 h');
  assert.equal(sessionLength(started, started + 20_000), '1 min');
  assert.equal(sessionLength(null, started), '1 min');
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
  const settings: RecorderSettings = { recordEveryCampaign: true, startWithWindows: false, startWithReplay: true, gameScore: true,
    operationOutcomes: true, politics: true, campaigns: { [CAMPAIGN]: { record: false, next: 'newFile' } }, recordingNames: {}, effectiveSaveFolder: 'C:\\r' };
  assert.equal(recordsAutomatically(settings, CAMPAIGN), false);
  assert.equal(nextLoad(settings, CAMPAIGN), 'newFile');
  assert.equal(recordsAutomatically(settings, 'other'), true);
  assert.equal(recordsAutomatically({ ...settings, recordEveryCampaign: false }, 'other'), false);
  assert.equal(nextLoad(settings, 'other'), 'continue');
  assert.equal(whatToRecord(settings), 'Everything');
  assert.equal(whatToRecord({ ...settings, politics: false }), 'Custom');
});
