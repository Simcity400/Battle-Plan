import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { engineHour } from '../dashboard/calendar.ts';
import type { UpdateState } from '../update/api';
import type { PanelActions, PanelUpdate, PanelViewProps } from './RecorderPanel';
import type { Phase, RecorderSettings, Recordings, Snapshot } from './status';

const CAMPAIGN = '00000000-0000-4000-8000-000000000000';
const OTHER = '00000000-0000-4000-8000-000000000001';
const PATH = `C:\\Recordings\\Germany 1936–1943.battleplan`;
const STARTED = Date.UTC(2026, 9, 12, 12, 0);
const MB = 2 ** 20;
const settings: RecorderSettings = { recordEveryCampaign: true, autoSave: true, startWithWindows: false, startWithReplay: true, captureMode: 'full', liveCompression: 'low', autoCompress: true,
  campaigns: {}, effectiveSaveFolder: 'C:\\Recordings' };
const recording: Snapshot = { phase: 'recording', campaign: CAMPAIGN, recording: PATH, name: 'Germany 1936–1943', continued: true, hour: 59_000_000,
  firstHour: null, savedBytes: 214 * MB, spoolBytes: 1229 * MB, startedAt: STARTED, stopping: false, problem: null, automatic: true,
  session: null, flag: null, captureMode: 'full', revision: 0 };
const flag = { width: 2, height: 1, pixels: [255, 0, 0, 255, 0, 0, 255, 255] };
const START = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
const NOW = engineHour({ year: 1941, month: 6, day: 12, hour: 1 })!;
const listed: Recordings = {
  unsaved: [
    { session: 'C:\\Recordings\\sessions\\2-2', campaign: CAMPAIGN, recording: PATH, name: 'Germany 1936–1943', firstHour: null, lastHour: NOW,
      spoolBytes: 1229 * MB, status: 'recording', flag },
    { session: 'C:\\Recordings\\sessions\\1-1', campaign: OTHER, recording: null, name: null, firstHour: START, lastHour: NOW, spoolBytes: 2253 * MB,
      status: 'waiting', flag: null },
  ],
  saved: [{ path: PATH, firstHour: START, lastHour: NOW, bytes: 412 * MB, flag, level: 7, job: null }],
};
const actions = new Proxy({}, { get: () => () => {} }) as PanelActions;
const text = (html: string) => html.replace(/<[^>]*>/g, ' ').replace(/&#x27;/g, '\'').replace(/\s+/g, ' ');

test('the panel draws each state, and the Stop prompt covers everything under the head', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { PanelView } = await vite.ssrLoadModule('/ui/src/recorder/RecorderPanel.tsx') as { PanelView: (props: PanelViewProps) => unknown };
    const render = (props: Partial<PanelViewProps>) => renderToStaticMarkup(createElement(PanelView as never, {
      snapshot: recording, settings, view: 'main', asking: false, error: null, now: STARTED + (2 * 60 + 14) * 60_000, actions, ...props } as never));

    const recordingPanel = render({});
    assert.match(text(recordingPanel), /Germany 1936–1943 Recording/);
    assert.match(text(recordingPanel), /1\.2 GB this session · 214 MB saved/);
    assert.match(recordingPanel, /<span class="recorder-flag"><span class="country-flag country-flag-unavailable" role="img" aria-label="Germany 1936–1943 flag unavailable"><svg[^>]*class="country-flag-glyph"/,
      'the shared placeholder, named for the campaign, stands in for a flag that did not resolve');
    assert.match(recordingPanel, /class="recorder-head" data-tauri-drag-region="deep"/, 'the head drags the panel');
    assert.match(recordingPanel, /<strong data-tooltip-overflow="Germany 1936–1943">Germany 1936–1943<\/strong>/, 'a clipped campaign title shows in full on hover');
    const flagged = render({ snapshot: { ...recording, flag } });
    assert.match(flagged, /<span class="recorder-flag"><canvas class="country-flag country-flag-art" width="2" height="1"/);
    assert.doesNotMatch(recordingPanel, /data-tooltip="[^"]*\.battleplan/, 'no file extension in the head');
    assert.match(flagged, /recorder-flag[\s\S]*Germany 1936–1943/, 'the flag leads the head');
    assert.match(text(recordingPanel), / Stop /);
    assert.match(text(recordingPanel), /This campaign Auto record Next time it loads Continue New file/);
    assert.doesNotMatch(text(recordingPanel), /Save this session/);
    const unnamed = text(render({ snapshot: { ...recording, recording: null, continued: false, savedBytes: null } }));
    assert.match(unnamed, /Germany 1936–1943 Recording/);
    assert.doesNotMatch(unnamed, new RegExp(CAMPAIGN));

    const asking = render({ asking: true });
    assert.match(text(asking), /Save this session \(2 h 14 min\)\? Cancel Discard Save/);
    assert.match(asking, /role="alertdialog"/);
    assert.match(asking, /<button type="button" class="button small primary" autofocus="">Save<\/button>/);
    assert.match(text(render({ asking: true, quitting: true })), /Save this session \(2 h 14 min\) before quitting\? Cancel Discard Save/);
    assert.match(asking, /aria-pressed="true"/, 'Stop stays pressed while asking');
    assert.match(asking, /<div class="recorder-under is-covered" inert="">[\s\S]*This campaign/,
      'the prompt covers everything under the head and out of reach, and the panel keeps its height');
    assert.doesNotMatch(recordingPanel, /inert/);
    const unshown = { text: 'Couldn’t show the recorder panel.', detail: 'the window couldn’t be sized', retry() {} };
    const failing = render({ asking: true, windowFailures: [unshown], snapshot: { ...recording, problem: { text: 'The game stopped answering.', detail: '', failed: true } } });
    const [reachable] = failing.split('<div class="recorder-under is-covered" inert="">');
    assert.match(text(reachable!), /Couldn’t show the recorder panel\. .*Try again/, 'a failure line stays in reach while the prompt covers the rest');
    assert.match(text(reachable!), /The game stopped answering\./);
    assert.doesNotMatch(asking, /is-danger/);
    const confirming = render({ asking: true, discarding: true });
    assert.match(text(confirming), /Discard this session \(2 h 14 min\)\? The earlier recording in this file is kept\. Cancel Discard/);
    assert.match(confirming, /class="button small quiet" autofocus="">Cancel<\/button><button type="button" class="button small is-danger">Discard/);
    assert.match(text(render({ asking: true, discarding: true, snapshot: { ...recording, continued: false } })), /This can’t be undone\. Cancel Discard/);
    assert.match(asking, /data-focus="discard"/, 'Esc on the Discard step returns focus to the Discard that opened it');
    assert.match(text(render({ asking: true, discarding: true, quitting: true })), /Discard this session \(2 h 14 min\) and quit\? The earlier/);
    assert.doesNotMatch(text(render({ asking: true, snapshot: { ...recording, stopping: true } })), /Save this session/);

    const saving = render({ snapshot: { ...recording, phase: 'saving' } });
    assert.match(text(saving), /Saving recording…/);
    assert.match(saving, /role="progressbar"/);
    assert.doesNotMatch(saving, /aria-valuenow/);
    assert.doesNotMatch(text(saving), /This campaign/);

    const problem = render({ snapshot: { ...recording, problem: { text: '2 hours couldn’t be recorded: the recorder fell behind.', detail: 'queue full', failed: false } } });
    assert.match(text(problem), /2 hours couldn’t be recorded: the recorder fell behind\. Details/);
    assert.match(problem, /<button type="button" class="recorder-text-button" aria-expanded="false">Details<\/button>/, 'Details discloses the reason');
    assert.doesNotMatch(problem, /queue full|Show files/, 'the reason stays folded away, and a caveat offers no files');

    const conflict = 'Another recorder is connected to this game. Close the other recorder to record here.';
    const unattached = { ...recording, phase: 'waiting', campaign: null, recording: null, name: null, problem: { text: conflict, detail: 'claimed', failed: true } } as const;
    const leftOver = render({ snapshot: unattached });
    assert.match(text(leftOver), /^ ?Another recorder is connected to this game Close the other recorder to record here\. Details Open BattlePlan/,
      'a failure is the headline, its next step the line under it, never a notice and never “Waiting for the game”');
    assert.match(leftOver, /recorder-glyph is-failed/);
    assert.doesNotMatch(leftOver, /class="notice|Waiting for the game/);
    const attachFailed = render({ snapshot: { ...unattached, phase: 'notRecording' } });
    assert.match(text(attachFailed), /^ ?Another recorder is connected to this game Close the other recorder to record here\. Details Record/);
    assert.match(attachFailed, /<strong class="is-problem">Another recorder is connected to this game<\/strong><span class="recorder-state is-problem">/);
    assert.match(text(render({ snapshot: { ...unattached, problem: { ...unattached.problem, text: 'Recording couldn’t start.' } } })),
      /^ ?Recording couldn’t start Details Open BattlePlan/, 'a one-sentence failure leaves only Details under it');
    assert.match(text(render({ snapshot: { ...unattached, phase: 'notRecording', name: 'Germany 1936–1943' } })),
      /Germany 1936–1943 Not recording .* Record Another recorder is connected to this game\. Close the other recorder to record here\. Details/,
      'a named campaign keeps its status, the problem beneath');
    assert.match(leftOver, /class="recorder-text-button"/, 'Details keeps its own 24px hit area');
    assert.doesNotMatch(leftOver, /Show files/, 'no session, no files to show');
    assert.match(text(render({ snapshot: { ...unattached, phase: 'notRecording', session: 'C:\\Recordings\\sessions\\2-2' } })), /record here\. Details Show files Record/,
      'a failed session offers its files beside Details');
    assert.match(text(render({ snapshot: { ...unattached, problem: { ...unattached.problem, text: 'Recording couldn’t start.', detail: '' } } })),
      /^ ?Recording couldn’t start Open BattlePlan/, 'no detail, no Details');
    assert.doesNotMatch(render({ snapshot: { ...recording, buildWarning: 'Unrecognised game build' } as Snapshot }), /Unrecognised game build/,
      'build identity stays in Diagnostics');

    const waiting = render({ snapshot: { ...recording, phase: 'waiting', campaign: null, recording: null } });
    assert.match(text(waiting), /Waiting for the game Records when you load a campaign\./);
    assert.doesNotMatch(waiting, /is-problem|is-failed/);
    assert.doesNotMatch(text(waiting), /This campaign/);
    assert.match(text(waiting), /Open BattlePlan Recordings $/);
    assert.match(render({ snapshot: { ...recording, phase: 'waiting', campaign: null, recording: null } }), /aria-label="Settings"/);
    assert.doesNotMatch(render({}), /recordings folder/i, 'the folder opens from the Recordings view');
    const failed = render({ error: 'Recorder settings couldn’t be saved' });
    assert.match(failed, /Recorder settings couldn’t be saved<\/span><button type="button" class="icon-button small dismiss-button" aria-label="Dismiss" data-tooltip="Dismiss"/,
      'the app’s shared dismiss button');
    assert.match(failed, /class="notice error" role="alert"/, 'a failed action uses the shared red banner');
    assert.match(problem, /class="notice warning" role="status"/, 'a recording that carries on with a caveat stays amber');
    const stopped = render({ snapshot: { ...recording, phase: 'notRecording', problem: { text: 'Recording stopped because of an error.', detail: 'synthetic', failed: true } } });
    assert.match(stopped, /class="notice error" role="alert"><svg[\s\S]*?Recording stopped because of an error\./, 'a failed recording is red even under a named campaign');
    assert.match(render({ snapshot: { ...unattached, problem: { ...unattached.problem, failed: false } } }), /recorder-glyph is-warning/, 'a caveat in the head is amber');
    assert.doesNotMatch(render({ snapshot: { ...recording, problem: { text: 'x', detail: '', failed: false } } }), /Dismiss/);
    const updateFailed = render({ error: 'Couldn’t download the update.', errorDetail: 'Offline (os error 10051)' });
    assert.match(updateFailed, /<span data-detail="Offline \(os error 10051\)">Couldn’t download the update\.<\/span>/, 'the raw reason stays out of the line');
    const caveat = render({ error: 'Saved. Country names couldn’t be read, so countries show by tag.', errorCaveat: true });
    assert.match(caveat, /class="notice warning" role="status"/, 'a save that worked with a caveat is amber, not red');

    const unsaved = render({ snapshot: { ...recording, phase: 'unsaved' } });
    assert.match(text(unsaved), /Germany 1936–1943 Not saved yet /);
    assert.equal(unsaved.match(/data-opens="recordings"/g)?.length, 1, 'only the footer opens Recordings');

    for (const phase of ['waiting', 'notRecording', 'recording', 'saving', 'saved', 'discarded', 'unsaved'] satisfies Phase[])
      assert.match(text(render({ snapshot: { ...recording, phase } })), /Open BattlePlan/, phase);

    const panelSettings = text(render({ view: 'settings' }));
    assert.match(panelSettings, /Settings Recording Auto record Records each campaign when it loads\. Auto save Saves when you load another campaign or close the game\./);
    assert.match(panelSettings, /close the game\. What to record Full Compression Low High Low: the recorder keeps pace with the game\. High: a smaller file, and the recorder can fall behind at full speed\./);
    assert.match(panelSettings, /fall behind at full speed\. Auto compress After saving, compresses the recording to High \(smaller file\) in the background\. Folders/);
    assert.match(render({ view: 'settings' }), /Auto compress<\/span><small>[^<]*<\/small><\/span><input type="checkbox" role="switch" checked=""/);
    assert.doesNotMatch(render({ view: 'settings', settings: { ...settings, autoCompress: false } }), /Auto compress<\/span><small>[^<]*<\/small><\/span><input type="checkbox" role="switch" checked=""/);
    const highSettings = render({ view: 'settings', settings: { ...settings, liveCompression: 'high' } });
    assert.match(text(highSettings), /Auto compress Only with Low compression\. High recordings are already compressed as they’re made\. Low only Folders/);
    assert.doesNotMatch(highSettings, /Auto compress<\/span><small>[^<]*<\/small><\/span><input/);
    assert.match(panelSettings, /Folders Recordings folder Recordings Paradox user folder Automatic/);
    assert.match(render({ view: 'settings' }), /aria-label="Recordings folder"[^>]*data-tooltip="C:\\Recordings"/, 'both folders are the same select, the full path on hover');
    assert.equal((render({ view: 'settings' }).match(/class="select-trigger"/g) ?? []).length, 2);
    assert.match(render({ view: 'settings' }), /<div class="group-label recorder-label">Recording<\/div>/, 'one group-label style');
    assert.doesNotMatch(panelSettings, /File size|Fast|Small/);
    assert.match(panelSettings, /Start automatically With Windows With BattlePlan Open BattlePlan $/);
    assert.match(render({ view: 'settings' }), /role="group" aria-label="Compression"><button type="button" aria-pressed="true">Low<\/button><button type="button" aria-pressed="false">High/);
    assert.match(render({ view: 'settings', settings: { ...settings, liveCompression: 'high' } }), /aria-pressed="false">Low<\/button><button type="button" aria-pressed="true">High/);
    assert.match(text(render({ view: 'settings', settings: { ...settings, captureMode: 'map' } })), /What to record Map/,
      'the summary names the mode');
    assert.match(render({ view: 'settings' }), /role="switch" checked=""[^>]*\/>[\s\S]*role="switch" checked=""/, 'both are on by default');
    assert.doesNotMatch(render({ view: 'settings', settings: { ...settings, autoSave: false } }),
      /Auto save<\/span><small>[^<]*<\/small><\/span><input type="checkbox" role="switch" checked=""/);
    const idle = { ...recording, phase: 'notRecording', captureMode: null } as const;
    const record = render({ view: 'record', snapshot: idle });
    assert.match(text(record), /What to record Capture mode Full Everything: the map, battle windows, division details and every dashboard\. Map Smaller files Campaign map, counters and battle windows; no dashboards or division details\./);
    assert.match(record, /role="radiogroup" aria-label="Capture mode"/);
    assert.match(record, /<label class="recorder-mode is-selected"><input type="radio" name="recorder-capture-mode" checked="" value="full"/);
    assert.doesNotMatch(record, /type="radio"[^>]*disabled/, 'the modes can be chosen between sessions');
    assert.match(text(record), /continues in a new recording file\. Open BattlePlan $/);
    assert.doesNotMatch(record, /role="switch"/);
    const map = render({ view: 'record', snapshot: idle, settings: { ...settings, captureMode: 'map' } });
    assert.match(map, /<label class="recorder-mode is-selected"><input type="radio" name="recorder-capture-mode" checked="" value="map"/);
    const fixed = render({ view: 'record' });
    assert.match(fixed, /class="recorder-modes is-fixed"/);
    assert.equal(fixed.match(/type="radio"[^>]*disabled=""/g)?.length, 2);
    assert.match(text(fixed), /Fixed while connected to the game, including at its menu\. A different mode starts a new recording file\./);
    const menu = render({ view: 'record', snapshot: { ...recording, phase: 'waiting', captureMode: 'map' } });
    assert.match(menu, /class="recorder-modes is-fixed"/);
    assert.match(menu, /checked=""[^>]*value="map"/);
    for (const view of ['main', 'settings', 'record', 'recordings'] as const)
      assert.match(text(render({ view, recordings: listed })), /Open BattlePlan/, view);
  } finally { await vite.close(); }
});

test('the recordings view lists what is not saved yet and what is saved, each with a quiet empty state', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { PanelView } = await vite.ssrLoadModule('/ui/src/recorder/RecorderPanel.tsx') as { PanelView: (props: PanelViewProps) => unknown };
    const render = (props: Partial<PanelViewProps>) => renderToStaticMarkup(createElement(PanelView as never, {
      snapshot: recording, settings, view: 'recordings', asking: false, error: null, now: STARTED, actions, ...props } as never));

    const view = render({ recordings: listed });
    assert.match(text(view), /Recordings Not saved yet 3\.4 GB on disk/);
    assert.match(text(view), /Germany 1936–1943 Recording · 12 Jun 1941 · 1\.2 GB New recording Waiting · 1936 – Jun 1941 · 2\.2 GB Discard Save/);
    assert.doesNotMatch(view, /is-danger/, 'Discard asks before it deletes');
    assert.equal((view.match(/>Save</g) ?? []).length, 1, 'only a waiting recording has Save');
    const asked = render({ recordings: listed, confirmingWaiting: `${listed.unsaved[1].session}/${listed.unsaved[1].campaign}` });
    assert.match(text(asked), /New recording Discard this recording\? This can’t be undone\. Cancel Discard Saved/);
    assert.match(asked, /class="button small quiet recorder-item-action" autofocus="">Cancel<\/button><button type="button" class="button small is-danger recorder-item-action">Discard/);
    assert.match(text(asked), /Germany 1936–1943 Recording · 12 Jun 1941 · 1\.2 GB New recording/, 'only the asked recording changes');
    assert.match(asked, /class="recorder-item is-confirming" data-waiting="[^"]+" data-tooltip-side="above"/, 'the name wraps while Discard asks');
    assert.match(asked, /class="recorder-item-meta is-asking" role="status"><span class="recorder-item-question">Discard this recording\?<\/span> This can’t be undone\./,
      'a neutral question with its consequence, like every other confirm');
    assert.match(view, /<span class="recorder-item-name" data-tooltip-overflow="New recording">/, 'a clipped waiting name shows in full on hover');
    assert.match(text(view), /Saved Germany 1936–1943 1936 – Jun 1941 · 412 MB Compress/);
    assert.match(view, /class="recorder-item is-openable" data-tooltip-side="above"/, 'hints open above the row, never over its own date and size');
    assert.match(view, /data-tooltip-overflow="Germany 1936–1943" aria-description="Open in BattlePlan"/, 'a clipped saved name shows in full on hover');
    assert.doesNotMatch(view, /data-tooltip="Open in BattlePlan"/);
    assert.doesNotMatch(view, /\.battleplan/, 'no file extension in the list or its tooltips');
    assert.match(view, /class="button small primary recorder-item-action">Save</, 'Save has the same emphasis as in the Stop prompt');
    assert.match(view, /aria-label="Show in folder"/);
    assert.match(view, /class="recorder-item is-openable"/, 'a saved row opens from anywhere on it');
    assert.match(view, /aria-label="Open recordings folder"/);
    assert.match(view, /class="recorder-item-flag"><canvas/);
    assert.match(view, /class="recorder-item-flag"><span class="country-flag country-flag-unavailable" role="img" aria-label="New recording flag unavailable">/,
      'no flag resolved: the shared placeholder, named for the recording');
    assert.doesNotMatch(view, /is-missing/);
    assert.match(view, /aria-description="Compress in the background"/);
    assert.doesNotMatch(view, /data-tooltip="Compress in the background"/, "the Compress label speaks for itself; no tooltip covers the row");
    assert.match(view, /class="recorder-list themed-scrollbar"/, 'long lists scroll inside the view');
    assert.match(view, /class="recorder-top" data-tauri-drag-region="deep"/);

    const saved = listed.saved[0];
    const compress = (row: Partial<typeof saved>, low = true) => render({ recordings: { unsaved: [], saved: [{ ...saved, ...row }] },
      settings: { ...settings, liveCompression: low ? 'low' : 'high' } });
    assert.equal((view.match(/>Compress</g) ?? []).length, 1);
    assert.doesNotMatch(compress({}, false), />Compress</, 'hidden while Compression is High');
    assert.doesNotMatch(compress({ level: 19 }), />Compress</, 'not on a recording at level 19');
    assert.match(compress({ level: null }), />Compress</, 'a file recording no level is offered it');
    const running = compress({ job: { state: 'compressing', units: 1, total: 4 } });
    assert.match(text(running), /412 MB · Compressing 25%/);
    assert.doesNotMatch(running, />Compress</);
    assert.match(text(compress({ job: { state: 'queued' } })), /Waiting to compress/);
    const kept = compress({ job: { state: 'kept', reason: 'open in the desktop app or another program' } });
    assert.match(text(kept), /412 MB · Kept as recorded: open in the desktop app or another program Compress/);
    assert.match(kept, /class="recorder-item-meta is-warning" role="status"/);
    const broken = compress({ job: { state: 'kept', reason: 'compression couldn’t finish: the compressor stopped early (exit code: 1)' } });
    assert.match(text(broken), /Kept as recorded: compression couldn’t finish Compress/);
    assert.match(broken, /data-detail="compression couldn’t finish: the compressor stopped early \(exit code: 1\)"/, 'the exit code stays in the detail');
    const unconfirmed = compress({ level: 19, job: { state: 'unconfirmed', reason: 'the disk is full' } });
    assert.match(text(unconfirmed), /Compression not confirmed: the disk is full/);
    assert.doesNotMatch(text(unconfirmed), /Kept as recorded/);
    assert.match(unconfirmed, /class="recorder-item-meta is-warning" role="status"/);

    const empty = text(render({ recordings: { unsaved: [], saved: [] } }));
    assert.match(empty, /Not saved yet Nothing is waiting to be saved\. Saved No saved recordings yet\./);
    assert.doesNotMatch(empty, /Spool on disk/);
    const unreadable = render({ recordings: { unsaved: [], saved: [], unreadable: { whole: true, detail: 'C:\\Recordings: Access is denied. (os error 5)' } } });
    assert.match(text(unreadable), /Saved Couldn’t read the recordings folder\. Try again Open BattlePlan/, 'an unreadable folder is not an empty one');
    assert.match(unreadable, /class="failure-line recorder-failure" role="alert" data-detail="C:\\Recordings: Access is denied\. \(os error 5\)"/);
    assert.match(render({ recordings: { unsaved: [], saved: [], unreadable: { whole: true, detail: 'x' } }, relisting: true }), /disabled="">Try again/);
    const partly = text(render({ recordings: { unsaved: [], saved: [saved], unreadable: { whole: false, detail: 'x' } } }));
    assert.match(partly, /Saved Couldn’t read some folders in the recordings folder\. Try again Germany 1936–1943 1936 – Jun 1941/, 'readable rows stay listed');
    const twins = render({ recordings: { unsaved: [], saved: [saved, { ...saved, path: 'C:\\Recordings\\Old\\Germany 1936–1943.battleplan' }] } });
    assert.match(twins, /data-tooltip-overflow="Germany 1936–1943 in Recordings" aria-label="Germany 1936–1943 in Recordings"/);
    assert.match(twins, /data-tooltip-overflow="Germany 1936–1943 in Old" aria-label="Germany 1936–1943 in Old"/);
    assert.match(text(twins), /Germany 1936–1943 Old · 1936 – Jun 1941/, 'the folder that tells them apart leads the second line');
    assert.doesNotMatch(view, /recorder-item-folder/, 'a unique name carries no folder');
    const loading = render({ recordings: null });
    assert.equal(text(loading).trim(), 'Recordings Loading recordings… Open BattlePlan', 'one loading line, no bare headings');
    assert.match(loading, /class="recorder-loading" role="status"/);
    assert.doesNotMatch(text(render({ recordings: null, error: 'The list could not be read' })), /Loading/);

    const main = render({ view: 'main', recordings: listed });
    assert.match(main, /aria-label="Recordings, 1 waiting to be saved"/);
    assert.match(main, />Recordings<span class="recorder-badge" aria-hidden="true">1</);
    assert.doesNotMatch(render({ view: 'main', recordings: { unsaved: [listed.unsaved[0]], saved: [] } }), /recorder-badge/);
  } finally { await vite.close(); }
});

test('a downloaded update says so in words above the footer, and the footer glyph shows only while there is news', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { PanelView } = await vite.ssrLoadModule('/ui/src/recorder/RecorderPanel.tsx') as { PanelView: (props: PanelViewProps) => unknown };
    const state: UpdateState = { enabled: true, status: 'up-to-date', channel: 'stable', build: 'stable', currentVersion: '0.9.4', availableVersion: null,
      downloadedVersion: null, releases: [], unlisted: false, downloadPercent: null, checkedAt: null, message: null, errorContext: null, installBlocked: null, channelWarning: null, downgrade: false };
    const update = (change: Partial<UpdateState>, ask: PanelUpdate['ask'] = null): PanelUpdate =>
      ({ state: { ...state, ...change }, failure: null, ask, pending: false, press() {}, restart() {}, retry() {}, dismiss() {}, channel() {} });
    const render = (props: Partial<PanelViewProps>) => renderToStaticMarkup(createElement(PanelView as never, {
      snapshot: { ...recording, phase: 'waiting', campaign: null, recording: null }, settings, view: 'main', asking: false, error: null, now: STARTED,
      actions, ...props } as never));
    const { blockedReason } = await vite.ssrLoadModule('/ui/src/update/logic.ts') as typeof import('../update/logic');
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
