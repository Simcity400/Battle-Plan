import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { engineHour } from '../dashboard/calendar.ts';
import type { PanelActions, PanelViewProps } from './RecorderPanel';
import type { Phase, RecorderSettings, Recordings, Snapshot } from './status';

const CAMPAIGN = '00000000-0000-4000-8000-000000000000';
const OTHER = '00000000-0000-4000-8000-000000000001';
const PATH = `C:\\Recordings\\Germany 1936–1943.battleplan`;
const STARTED = Date.UTC(2026, 9, 12, 12, 0);
const MB = 2 ** 20;
const settings: RecorderSettings = { recordEveryCampaign: true, autoSave: true, startWithWindows: false, startWithReplay: true, captureMode: 'full', liveCompression: 'low', autoCompress: true,
  campaigns: {}, effectiveSaveFolder: 'C:\\Recordings' };
const recording: Snapshot = { phase: 'recording', campaign: CAMPAIGN, recording: PATH, name: 'Germany 1936–1943', continued: true, hour: 59_000_000,
  firstHour: null, savedBytes: 214 * MB, spoolBytes: 1229 * MB, progress: null, startedAt: STARTED, stopping: false, problem: null, automatic: true,
  session: null, flag: null, captureMode: 'full' };
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

test('the panel draws each state, and the Stop prompt replaces everything under the head', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { PanelView } = await vite.ssrLoadModule('/ui/src/recorder/RecorderPanel.tsx') as { PanelView: (props: PanelViewProps) => unknown };
    const render = (props: Partial<PanelViewProps>) => renderToStaticMarkup(createElement(PanelView as never, {
      snapshot: recording, settings, view: 'main', asking: false, error: null, now: STARTED + (2 * 60 + 14) * 60_000, actions, ...props } as never));

    const recordingPanel = render({});
    assert.match(text(recordingPanel), /Germany 1936–1943 Recording/);
    assert.match(text(recordingPanel), /1\.2 GB this session · 214 MB saved/);
    assert.doesNotMatch(recordingPanel, /recorder-flag/, 'no flag is drawn when none resolved');
    assert.match(recordingPanel, /class="recorder-head" data-tauri-drag-region="deep"/, 'the head drags the panel');
    const flagged = render({ snapshot: { ...recording, flag } });
    assert.match(flagged, /<span class="recorder-flag"><canvas class="country-flag country-flag-art" width="2" height="1"/);
    assert.match(flagged, /recorder-flag[\s\S]*Germany 1936–1943/, 'the flag leads the head');
    assert.match(text(recordingPanel), / Stop /);
    assert.match(text(recordingPanel), /This campaign Record automatically Next time it loads Continue New file/);
    assert.doesNotMatch(text(recordingPanel), /Keep this session/);
    const unnamed = text(render({ snapshot: { ...recording, recording: null, continued: false, savedBytes: null } }));
    assert.match(unnamed, /Germany 1936–1943 Recording/);
    assert.doesNotMatch(unnamed, new RegExp(CAMPAIGN));

    const asking = render({ asking: true });
    assert.match(text(asking), /Keep this session's 2 h 14 min\? Discard Save/);
    assert.match(text(render({ asking: true, quitting: true })), /Keep 2 h 14 min before quitting\? Discard Save/);
    assert.match(asking, /aria-pressed="true"/, 'Stop stays pressed while asking');
    assert.doesNotMatch(text(asking), /This campaign/, 'the prompt is the panel’s only row under the head');
    assert.doesNotMatch(asking, /is-danger/);
    const confirming = render({ asking: true, discarding: true });
    assert.match(text(confirming), /Discard this session's 2 h 14 min\? Cancel Discard/);
    assert.match(confirming, /class="button small quiet" autofocus="">Cancel<\/button><button type="button" class="button small is-danger" data-tooltip="The earlier recording in this file is kept\.">Discard/);
    assert.match(render({ asking: true, discarding: true, snapshot: { ...recording, continued: false } }), /data-tooltip="This can&#x27;t be undone\."/);
    assert.match(text(render({ asking: true, discarding: true, quitting: true })), /Discard 2 h 14 min and quit\? Cancel Discard/);
    assert.doesNotMatch(text(render({ asking: true, snapshot: { ...recording, stopping: true } })), /Keep this session/);

    const saving = render({ snapshot: { ...recording, phase: 'saving', progress: 0.35 } });
    assert.match(text(saving), /Saving recording…/);
    assert.match(saving, /role="progressbar"[^>]*aria-valuenow="35"/);
    assert.doesNotMatch(text(saving), /This campaign/);

    const problem = render({ snapshot: { ...recording, problem: { text: '2 hours couldn\'t be recorded', detail: 'queue full' } } });
    assert.match(text(problem), /2 hours couldn't be recorded Details/);
    assert.match(problem, /data-tooltip="queue full"/);

    const waiting = render({ snapshot: { ...recording, phase: 'waiting', campaign: null, recording: null } });
    assert.match(text(waiting), /Waiting for the game Records when you load a campaign/);
    assert.doesNotMatch(text(waiting), /This campaign/);
    assert.match(text(waiting), /Open Battle Plan Recordings $/);
    assert.match(render({ snapshot: { ...recording, phase: 'waiting', campaign: null, recording: null } }), /aria-label="Settings"/);
    assert.doesNotMatch(render({}), /recordings folder/i, 'the folder opens from the Recordings view');
    const failed = render({ error: 'Recorder settings could not be saved' });
    assert.match(failed, /Recorder settings could not be saved<\/span><button type="button" class="icon-button recorder-dismiss" aria-label="Dismiss"/);
    assert.doesNotMatch(render({ snapshot: { ...recording, problem: { text: 'x', detail: '' } } }), /Dismiss/);

    const unsaved = render({ snapshot: { ...recording, phase: 'unsaved' } });
    assert.match(text(unsaved), /Germany 1936–1943 Not saved yet .* Recordings/);

    for (const phase of ['waiting', 'notRecording', 'recording', 'saving', 'saved', 'discarded', 'unsaved'] satisfies Phase[])
      assert.match(text(render({ snapshot: { ...recording, phase } })), /Open Battle Plan/, phase);

    const panelSettings = text(render({ view: 'settings' }));
    assert.match(panelSettings, /Settings Recording Auto record Records each campaign when it loads\. Auto save Saves when you load another campaign or close the game\./);
    assert.match(panelSettings, /close the game\. What to record Full Compression Low High Low: the recorder keeps pace with the game\. High: a smaller file, and the recorder can fall behind at full speed\./);
    assert.match(panelSettings, /fall behind at full speed\. Auto Compress After saving, compresses the recording to the smaller High size in the background; usable at once meanwhile\. Folders/);
    assert.match(render({ view: 'settings' }), /Auto Compress<\/span><small>[^<]*<\/small><\/span><input type="checkbox" role="switch" checked=""/);
    assert.doesNotMatch(render({ view: 'settings', settings: { ...settings, autoCompress: false } }), /Auto Compress<\/span><small>[^<]*<\/small><\/span><input type="checkbox" role="switch" checked=""/);
    assert.doesNotMatch(text(render({ view: 'settings', settings: { ...settings, liveCompression: 'high' } })), /Auto Compress/);
    assert.match(panelSettings, /Folders Save to C:\\Recordings Paradox user folder Automatic/);
    assert.doesNotMatch(panelSettings, /File size|Fast|Small/);
    assert.match(panelSettings, /Start automatically With Windows With Battle Plan Open Battle Plan $/);
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
    assert.match(text(record), /continues in a new recording file\. Open Battle Plan $/);
    assert.doesNotMatch(record, /role="switch"/);
    const map = render({ view: 'record', snapshot: idle, settings: { ...settings, captureMode: 'map' } });
    assert.match(map, /<label class="recorder-mode is-selected"><input type="radio" name="recorder-capture-mode" checked="" value="map"/);
    const fixed = render({ view: 'record' });
    assert.match(fixed, /class="recorder-modes is-fixed"/);
    assert.equal(fixed.match(/type="radio"[^>]*disabled=""/g)?.length, 2);
    assert.match(text(fixed), /Fixed while attached to the game, including at its menu\. A different mode starts a new recording file\./);
    const menu = render({ view: 'record', snapshot: { ...recording, phase: 'waiting', captureMode: 'map' } });
    assert.match(menu, /class="recorder-modes is-fixed"/);
    assert.match(menu, /checked=""[^>]*value="map"/);
    for (const view of ['main', 'settings', 'record', 'recordings'] as const)
      assert.match(text(render({ view, recordings: listed })), /Open Battle Plan/, view);
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
    assert.match(text(asked), /New recording Discard this recording\? This can't be undone\. Cancel Discard Saved/);
    assert.match(asked, /class="button small quiet recorder-item-action" autofocus="">Cancel<\/button><button type="button" class="button small is-danger recorder-item-action">Discard/);
    assert.match(text(asked), /Germany 1936–1943 Recording · 12 Jun 1941 · 1\.2 GB New recording/, 'only the asked recording changes');
    assert.match(text(view), /Saved Germany 1936–1943 1936 – Jun 1941 · 412 MB Compress/);
    assert.match(view, /data-tooltip="Open in Battle Plan · Germany 1936–1943\.battleplan"/);
    assert.match(view, /aria-label="Show in folder"/);
    assert.match(view, /class="recorder-item is-openable"/, 'a saved row opens from anywhere on it');
    assert.match(view, /aria-label="Open recordings folder"/);
    assert.match(view, /class="recorder-item-flag"><canvas/);
    assert.match(view, /class="recorder-item-flag is-empty"/, 'no flag resolved: a quiet placeholder');
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
    assert.match(text(compress({ job: { state: 'queued' } })), /Compress waiting/);
    const kept = compress({ job: { state: 'kept', reason: 'open in the desktop app or another program' } });
    assert.match(text(kept), /412 MB · Kept as recorded: open in the desktop app or another program Compress/);
    assert.match(kept, /class="recorder-item-meta is-warning" role="status"/);

    const empty = text(render({ recordings: { unsaved: [], saved: [] } }));
    assert.match(empty, /Not saved yet Nothing is waiting to be saved\. Saved No saved recordings yet\./);
    assert.doesNotMatch(empty, /Spool on disk/);
    assert.doesNotMatch(text(render({ recordings: null })), /Nothing is waiting|No saved recordings/);

    const main = render({ view: 'main', recordings: listed });
    assert.match(main, /aria-label="Recordings, 1 waiting to be saved"/);
    assert.match(main, />Recordings<span class="recorder-badge" aria-hidden="true">1</);
    assert.doesNotMatch(render({ view: 'main', recordings: { unsaved: [listed.unsaved[0]], saved: [] } }), /recorder-badge/);
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
    assert.equal(text(prompt).trim(), 'Stop recording? Keep this session\'s 2 h 14 min of Germany 1936–1943? Cancel Discard Save');
    assert.match(prompt, /role="alertdialog"/);
    assert.match(prompt, /<button type="button" class="button small primary" autofocus="">Save<\/button>/);
    assert.match(text(render({ quitting: true })), /^ ?Quit Battle Plan Recorder\? Keep this session's 2 h 14 min/);
    assert.doesNotMatch(text(prompt), /Open Battle Plan|This campaign/);
    const discarding = render({ discarding: true });
    assert.equal(text(discarding).trim(),
      'Discard this session? Deletes this session\'s 2 h 14 min of Germany 1936–1943. The earlier recording in this file is kept. Cancel Discard');
    assert.match(discarding, /<button type="button" class="button small quiet" autofocus="">Cancel<\/button><button type="button" class="button small is-danger">Discard<\/button>/);
    assert.match(text(render({ discarding: true, quitting: true })), /^ ?Discard and quit\?/);
    assert.equal(text(render({ snapshot: { ...recording, stopping: true } })).trim(), '');
  } finally { await vite.close(); }
});
