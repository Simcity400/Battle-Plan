import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { UpdateState } from '../update/api';
import type { PanelActions, PanelUpdate, PanelViewProps } from './RecorderPanel';
import type { RecorderSettings, Snapshot } from './status';

const CAMPAIGN = '00000000-0000-4000-8000-000000000000';
const PATH = `C:\\Recordings\\Germany 1936–1943.battleplan`;
const STARTED = Date.UTC(2026, 9, 12, 12, 0);
const MB = 2 ** 20;
const settings: RecorderSettings = { recordEveryCampaign: true, autoSave: true, startWithWindows: false, startWithReplay: true, captureMode: 'full', liveCompression: 'low', autoCompress: true,
  campaigns: {}, effectiveSaveFolder: 'C:\\Recordings' };
const recording: Snapshot = { phase: 'recording', campaign: CAMPAIGN, recording: PATH, name: 'Germany 1936–1943', continued: true, hour: 59_000_000,
  firstHour: null, savedBytes: 214 * MB, spoolBytes: 1229 * MB, startedAt: STARTED, stopping: false, problem: null, automatic: true,
  session: null, flag: null, captureMode: 'full', revision: 0 };
const actions = new Proxy({}, { get: () => () => {} }) as PanelActions;
const text = (html: string) => html.replace(/<[^>]*>/g, ' ').replace(/&#x27;/g, '\'').replace(/\s+/g, ' ');


test('a downloaded update says so in words above the footer, and the footer glyph shows only while there is news', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { PanelView } = await vite.ssrLoadModule('/ui/src/recorder/RecorderPanel.tsx') as { PanelView: (props: PanelViewProps) => unknown };
    const state: UpdateState = { enabled: true, status: 'up-to-date', channel: 'stable', build: 'stable', currentVersion: '0.9.4', availableVersion: null,
      downloadedVersion: null, releases: [], unlisted: false, downloadPercent: null, checkedAt: null, message: null, errorContext: null, installBlocked: null, channelWarning: null, downgrade: false, revision: 1 };
    const update = (change: Partial<UpdateState>, ask: PanelUpdate['ask'] = null): PanelUpdate =>
      ({ state: { ...state, ...change }, failure: null, ask, pending: false, press() {}, restart() {}, retry() {}, dismiss() {}, channel() {} });
    const render = (props: Partial<PanelViewProps>) => renderToStaticMarkup(createElement(PanelView as never, {
      snapshot: { ...recording, phase: 'waiting', campaign: null, recording: null }, settings, view: 'main', asking: false, error: null, now: STARTED,
      actions, ...props } as never));
    const { blockedReason, latestUpdateState, peekTitle } = await vite.ssrLoadModule('/ui/src/update/logic.ts') as typeof import('../update/logic');
    const { UpdateNotes } = await vite.ssrLoadModule('/ui/src/update/UpdateControl.tsx') as typeof import('../update/UpdateControl');
    const { actionFailure } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts') as typeof import('../dashboard/queryError');
    assert.deepEqual(actionFailure('Couldn’t show the recording in its folder.', 'Couldn’t show Germany 1936 in its folder: C:\\Rec\\Germany 1936.battleplan: denied'),
      { text: 'Couldn’t show Germany 1936 in its folder.', detail: 'Couldn’t show Germany 1936 in its folder: C:\\Rec\\Germany 1936.battleplan: denied' });
    assert.deepEqual(actionFailure('Couldn’t open BattlePlan.', 'Access is denied. (os error 5)'), { text: 'Couldn’t open BattlePlan.', detail: 'Access is denied. (os error 5)' },
      'a bare OS error is named by the action that failed');
    assert.deepEqual(actionFailure('Couldn’t start recording.', 'BattlePlan Recorder is closing for an update.'), { text: 'BattlePlan Recorder is closing for an update.' },
      'a plain sentence is shown as it is');
    assert.deepEqual(actionFailure('Couldn’t save the recording.', 'C:\\Rec\\sessions\\1-1\\Germany 1936: Access is denied. (os error 5)'),
      { text: 'Couldn’t save the recording.', detail: 'C:\\Rec\\sessions\\1-1\\Germany 1936: Access is denied. (os error 5)' }, 'a path is never the message');
    assert.deepEqual(actionFailure('Couldn’t discard the recording.', 'the map closed'), { text: 'Couldn’t discard the recording.', detail: 'the map closed' },
      'a raw fragment is never the message');
    assert.equal(blockedReason({ ...state, installBlocked: 'BattlePlan isn’t responding, so the update can’t start yet. Try again in a moment: The pipe is busy. (os error 231)' }),
      'BattlePlan isn’t responding, so the update can’t start yet. Try again in a moment.');

    const idle = render({ update: update({}) });
    assert.doesNotMatch(idle, /update-button|update-dot|recorder-update-ready/, 'nothing to say while up to date');
    assert.match(render({ update: update({ status: 'available', availableVersion: '0.9.5' }) }), /class="update-badge update-dot"/);
    assert.match(render({ update: update({ status: 'downloading', availableVersion: '0.9.5', downloadPercent: 40 }) }), /class="update-button"/);
    const offline = 'The stable update feed couldn’t be read: it couldn’t be reached: error sending request for url (https://example.invalid/feed).';
    const checkFailed = render({ update: update({ status: 'error', errorContext: 'check', message: offline }) });
    assert.match(checkFailed, /class="update-badge is-alert"/);
    assert.match(checkFailed, /aria-label="Couldn’t check for updates · Try again" data-tooltip="Couldn’t check for updates · Try again"/,
      'the button names the failure plainly, never the network error');
    assert.doesNotMatch(checkFailed, /example\.invalid|update feed/);
    const failedSettings = render({ view: 'settings', update: update({ status: 'error', errorContext: 'check', message: offline }) });
    assert.match(text(failedSettings), /Updates BattlePlan Recorder 0\.9\.4 Couldn’t check for updates\. Try again Channel .*Stable: tested releases/,
      'one failure line with Try again in place of the button');
    assert.match(failedSettings, /class="failure-line recorder-update-failure" role="alert" data-detail="The stable update feed/);
    assert.doesNotMatch(text(failedSettings), /example\.invalid|Check for updates/);
    const downloadFailed = update({ status: 'error', errorContext: 'download', availableVersion: '0.9.5', message: 'Update 0.9.5 couldn’t be downloaded: offline.' });
    assert.match(render({ update: downloadFailed }), /data-tooltip="Couldn’t download update 0\.9\.5 · Try again"/);
    assert.match(text(render({ view: 'settings', update: downloadFailed })), /Couldn’t download update 0\.9\.5\. Try again/);
    const laterCheck = update({ status: 'downloaded', errorContext: 'check', availableVersion: '0.9.5', downloadedVersion: '0.9.5', message: offline });
    assert.match(text(render({ view: 'settings', update: laterCheck })), /Update 0\.9\.5 is downloaded\./, 'a later failed check never hides a downloaded update');
    const summarized = { version: '0.9.6', since: '0.9.5', date: null, changes: 4, more: 2, pulls: [],
      highlights: [{ label: 'Fog of war', detail: 'The map hides what a country cannot see.' }] };
    const listed = { version: '0.9.5', since: '0.9.3', date: null, changes: 3, highlights: null, more: null,
      pulls: [{ number: 10, title: 'Recorder: a spool survives a crash' }] };
    const behind = text(render({ view: 'settings', update: update({ status: 'available', availableVersion: '0.9.6', releases: [summarized, listed], unlisted: true }) }));
    assert.match(behind, /is available\. Fog of war\. The map hides what a country cannot see\. Plus 2 more fixes and improvements\. 0\.9\.5 Recorder: a spool survives a crash And 2 more changes\. Earlier releases aren’t listed here\./,
      'each release the update brings, newest first; one without a summary lists its pull requests, and a gap in the history is named');
    assert.match(text(render({ view: 'settings', update: update({ status: 'available', availableVersion: '0.9.6', unlisted: true }) })), /This update’s changes aren’t listed here\./,
      'an update whose notes are missing says so');
    const direct = { version: '0.9.6', since: '0.9.5', date: null, changes: 1, highlights: [], more: 0, pulls: [] };
    assert.match(text(render({ view: 'settings', update: update({ status: 'available', availableVersion: '0.9.6', releases: [direct] }) })), /1 change made without a pull request\./,
      'a change no summary read is never called internal');
    assert.equal(peekTitle({ ...state, status: 'error', errorContext: 'download', availableVersion: '0.9.6' }), 'Couldn’t download update 0.9.6 · Try again',
      'hovering a failed update names the failure, never a ready update');
    assert.match(renderToStaticMarkup(createElement(UpdateNotes, { state: { ...state, status: 'available', availableVersion: '0.9.6', unlisted: true },
      onOpenRelease() {} })), /Release on GitHub/, 'an update without notes still links to its release');
    const installFailed = render({ update: update({ status: 'error', errorContext: 'install', availableVersion: '0.9.5', downloadedVersion: '0.9.5', message: 'The installer couldn’t be started: denied.' }) });
    assert.match(text(installFailed), /Couldn’t install update 0\.9\.5\. Restart to update/, 'a failed install says so where the restart is offered');
    assert.doesNotMatch(text(installFailed), /denied/);
    assert.match(installFailed, /data-detail="The installer couldn’t be started: denied\."/);
    const ready = render({ update: update({ status: 'downloaded', availableVersion: '0.9.5', downloadedVersion: '0.9.5' }) });
    assert.match(text(ready), /Update 0\.9\.5 is ready Restart to update Open BattlePlan/);
    assert.doesNotMatch(ready, /class="update-button"/, 'one control for one action');
    const asked = text(render({ update: update({ status: 'downloaded', availableVersion: '0.9.5', downloadedVersion: '0.9.5' }, 'confirm') }));
    assert.match(asked, /Restart to update to 0\.9\.5\? BattlePlan and BattlePlan Recorder close, update to 0\.9\.5 and reopen\. Cancel Restart to update/);
    assert.doesNotMatch(asked, /is ready/);
    assert.match(render({ update: update({ status: 'downloaded', availableVersion: '0.9.5', downloadedVersion: '0.9.5' }, 'downloaded') }),
      /Restart to update to 0\.9\.5\?[\s\S]*class="button small quiet" autofocus="">Later<\/button><button type="button" class="button small primary">Restart to update</, 'one question and verb, the safe choice focused');
    assert.match(ready, /data-focus="update"><svg[^>]*>[\s\S]*?<\/svg>Restart to update/, 'dismissing the prompt returns focus here; the update glyph leads it');
    assert.match(render({ view: 'settings', update: update({}) }), /data-focus="update-settings"><svg[^>]*>[\s\S]*?<\/svg>Check again/, 'the same glyph as the app’s update button');
    const fromSettings = render({ view: 'settings', update: update({ status: 'downloaded', availableVersion: '0.9.5', downloadedVersion: '0.9.5' }, 'confirm') });
    assert.match(text(fromSettings), /Stable: tested releases for BattlePlan and the recorder\. Stable Nightly Restart to update to 0\.9\.5\? .* Cancel Restart to update Open BattlePlan/,
      'a restart asked from Settings is asked there');
    assert.match(fromSettings, /data-focus="update-settings"/, 'dismissing it returns focus to the Settings button');
    const switching = { status: 'downloaded', channel: 'stable', build: 'nightly', currentVersion: '0.9.5-nightly.20261123.7', availableVersion: '0.9.4',
      downloadedVersion: '0.9.4', downgrade: true } as const;
    assert.match(text(render({ update: update(switching) })), /Version 0\.9\.4 is ready Restart to switch Open BattlePlan/);
    assert.match(text(render({ update: update(switching, 'confirm') })),
      /Restart to switch to 0\.9\.4\? BattlePlan and BattlePlan Recorder close, switch to 0\.9\.4, an older version, and reopen\. Recordings made with this build may not open in it\. Cancel Restart to switch/,
      'a channel switch to an older release says so before the restart');
    assert.match(text(render({ view: 'settings', update: update({ ...switching, status: 'available', downloadedVersion: null }) })),
      /Version 0\.9\.4 is available on this channel\. It’s older than 0\.9\.5-nightly\.20261123\.7\. Download Channel Stable: tested releases/);
    const updates = text(render({ view: 'settings', update: update({ checkedAt: Date.UTC(2026, 10, 21, 18, 0) }) }));
    assert.match(updates, /BattlePlan Recorder 0\.9\.4 Up to date \(checked \d\d:\d\d\)\. Check again Channel Stable: tested releases for BattlePlan and the recorder\. Stable Nightly/,
      'the check result sits with the version and its button, the channel note with the channel');
    const reading = text(render({ view: 'settings', update: { ...update({}), state: null } }));
    assert.match(reading, /Updates BattlePlan Recorder Reading the update state… Check for updates Channel —/);
    const unread = render({ view: 'settings', update: { ...update({}), state: null, failure: 'The updater is not running.' } });
    assert.match(text(unread), /Updates BattlePlan Recorder Couldn’t check for updates\. Try again Channel Unavailable Open BattlePlan/,
      'a failed read says so plainly and offers Try again, never a read that goes on forever');
    assert.doesNotMatch(text(unread), /Reading the update state/);
    assert.match(unread, /class="failure-line recorder-update-failure" role="alert" data-detail="The updater is not running\."><svg/, 'the raw reason is kept out of the line');
    const channel = (change: Partial<UpdateState>, pending = false) => render({ view: 'settings', update: { ...update(change), pending } })
      .match(/role="group" aria-label="Update channel">.*?<\/div>/)![0];
    assert.doesNotMatch(channel({}), /disabled/);
    assert.equal(channel({ status: 'downloading', availableVersion: '0.9.5', downloadPercent: 40 }).match(/disabled=""/g)?.length, 2, 'no channel change during a download');
    assert.equal(channel({}, true).match(/disabled=""/g)?.length, 2, 'nor while an update action runs');
    assert.doesNotMatch(channel({ status: 'checking' }), /disabled/, 'a background check leaves the channel free to change');
    const newer = { ...state, revision: 2, status: 'available' as const, availableVersion: '0.9.5' };
    assert.equal(latestUpdateState(newer, state), newer, 'an older state arriving after a newer one is ignored');
    assert.equal(latestUpdateState(state, newer), newer);
    assert.equal(latestUpdateState(null, state), state);
  } finally { await vite.close(); }
});

test('the Stop prompt beside the tray icon asks one question with Save focused, and nothing else', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { PanelView } = await vite.ssrLoadModule('/ui/src/recorder/RecorderPanel.tsx') as { PanelView: (props: PanelViewProps) => unknown };
    const render = (props: Partial<PanelViewProps>) => renderToStaticMarkup(createElement(PanelView as never, {
      snapshot: recording, settings, view: 'main', asking: true, error: null, now: STARTED + (2 * 60 + 14) * 60_000, actions, compact: true, ...props } as never));
    const prompt = render({});
    assert.equal(text(prompt).trim(), 'Stop recording? Save this session of Germany 1936–1943 (2 h 14 min)? Cancel Discard Save');
    assert.match(prompt, /role="alertdialog"/);
    assert.match(prompt, /<button type="button" class="button small primary" autofocus="">Save<\/button>/);
    assert.match(text(render({ quitting: true })), /^ ?Quit BattlePlan Recorder\? Save this session of Germany 1936–1943 \(2 h 14 min\)\? Cancel Discard Save/);
    assert.doesNotMatch(text(prompt), /Open BattlePlan|This campaign/);
    const discarding = render({ discarding: true });
    assert.equal(text(discarding).trim(),
      'Discard this session? Deletes this session’s 2 h 14 min of Germany 1936–1943. The earlier recording in this file is kept. Cancel Discard');
    assert.match(discarding, /<button type="button" class="button small quiet" autofocus="">Cancel<\/button><button type="button" class="button small is-danger">Discard<\/button>/);
    assert.match(discarding, /<p id="recorder-prompt-question" class="recorder-prompt-lead">[^<]*<\/p><p id="recorder-prompt-consequence">The earlier recording in this file is kept\.<\/p>/,
      'the consequence has its own line, so a long name never pushes it out of sight');
    assert.match(discarding, /aria-describedby="recorder-prompt-question recorder-prompt-consequence"/);
    assert.match(text(render({ discarding: true, quitting: true })), /^ ?Discard and quit\?/);
    assert.equal(text(render({ snapshot: { ...recording, stopping: true } })).trim(), '');
    const lost = { text: 'Couldn’t save the recording.', detail: 'the reply was lost', retry() {} };
    assert.match(text(render({ snapshot: { ...recording, stopping: true }, promptFailure: lost })), /Couldn’t save the recording\. .*Try again/,
      'a failed reply outlives the prompt the status already retired, so the panel it keeps open isn’t blank');
    const unclosed = { text: 'Couldn’t close the recorder panel.', detail: 'the request failed', retry() {} };
    assert.match(text(render({ asking: false, windowFailures: [unclosed] })), /Couldn’t close the recorder panel\. .*Try again/,
      'a panel that couldn’t close itself says so, with Try again, instead of standing blank');
  } finally { await vite.close(); }
});

test('before the first status the panel says it is connecting, and a failed read offers Try again', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { PanelPending } = await vite.ssrLoadModule('/ui/src/recorder/RecorderPanel.tsx') as { PanelPending: (props: { failure: string | null; retry(): void }) => unknown };
    const render = (failure: string | null) => renderToStaticMarkup(createElement(PanelPending as never, { failure, retry() {} } as never));
    const connecting = render(null);
    assert.equal(text(connecting).trim(), 'Connecting to the recorder…');
    assert.match(connecting, /role="status"/);
    const failed = render('The recorder is not running.');
    assert.equal(text(failed).trim(), 'Couldn’t reach the recorder. Try again');
    assert.match(failed, /<strong data-detail="The recorder is not running\.">/, 'the raw reason stays out of the line');
    assert.match(failed, /role="alert"/);
  } finally { await vite.close(); }
});
