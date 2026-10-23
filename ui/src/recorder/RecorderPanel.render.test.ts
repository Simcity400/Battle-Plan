import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { PanelActions, PanelViewProps } from './RecorderPanel';
import type { RecorderSettings, Snapshot } from './status';

const CAMPAIGN = '00000000-0000-4000-8000-000000000000';
const PATH = `C:\\Recordings\\Germany 1936–1943.battleplan`;
const STARTED = Date.UTC(2026, 9, 12, 12, 0);
const settings: RecorderSettings = { recordEveryCampaign: true, startWithWindows: false, startWithReplay: true, gameScore: true,
  operationOutcomes: true, politics: true, captureMode: 'full', campaigns: {}, effectiveSaveFolder: 'C:\\Recordings' };
const recording: Snapshot = { phase: 'recording', campaign: CAMPAIGN, recording: PATH, name: 'Germany 1936–1943', continued: true, hour: 59_000_000, firstHour: null,
  bytes: 380 * 2 ** 20, progress: null, startedAt: STARTED, stopping: false, problem: null, automatic: true, session: null, flag: null, captureMode: 'full' };
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
    assert.doesNotMatch(recordingPanel, /recorder-flag/, 'no flag is drawn when none resolved');
    const flagged = render({ snapshot: { ...recording, flag: { width: 2, height: 1, pixels: [255, 0, 0, 255, 0, 0, 255, 255] } } });
    assert.match(flagged, /<span class="recorder-flag"><canvas class="country-flag country-flag-art" width="2" height="1"/);
    assert.match(flagged, /recorder-flag[\s\S]*Germany 1936–1943/, 'the flag leads the head');
    assert.match(text(recordingPanel), / Stop /);
    assert.match(text(recordingPanel), /This campaign Record automatically Next time it loads Continue New file/);
    assert.match(text(recordingPanel), /Open Battle Plan/);
    assert.doesNotMatch(text(recordingPanel), /Keep this session/);

    const asking = render({ asking: true });
    assert.match(text(asking), /Keep this session's 2 h 14 min\? Discard Save/);
    assert.match(asking, /aria-pressed="true"/, 'Stop stays pressed while asking');
    assert.match(asking, /data-tooltip="Deletes this session only\. Earlier recording in this file is kept\."/);
    assert.doesNotMatch(text(asking), /This campaign|Open Battle Plan/, 'the prompt is the panel’s only row under the head');
    assert.match(render({ asking: true, snapshot: { ...recording, continued: false } }), /data-tooltip="Deletes this recording\."/);
    // A stop being carried out, or saving, closes the prompt.
    assert.doesNotMatch(text(render({ asking: true, snapshot: { ...recording, stopping: true } })), /Keep this session/);

    const saving = render({ snapshot: { ...recording, phase: 'saving', progress: 0.35 } });
    assert.match(text(saving), /Saving recording…/);
    assert.match(saving, /role="progressbar"[^>]*aria-valuenow="35"/);
    assert.doesNotMatch(text(saving), /Open Battle Plan|This campaign/);

    const problem = render({ snapshot: { ...recording, problem: { text: '2 hours couldn\'t be recorded', detail: 'queue full' } } });
    assert.match(text(problem), /2 hours couldn't be recorded Details/);
    assert.match(problem, /data-tooltip="queue full"/);

    const waiting = render({ snapshot: { ...recording, phase: 'waiting', campaign: null, recording: null } });
    assert.match(text(waiting), /Waiting for the game Records when you load a campaign/);
    assert.doesNotMatch(text(waiting), /This campaign/);

    const panelSettings = text(render({ view: 'settings' }));
    assert.match(panelSettings, /Settings Record every campaign Save to C:\\Recordings Paradox user folder Automatic/);
    assert.doesNotMatch(panelSettings, /File size|Fast|Small/);
    assert.match(panelSettings, /Start With Windows With Battle Plan What to record Full/);
    assert.match(text(render({ view: 'settings', settings: { ...settings, captureMode: 'map' } })), /What to record Map/,
      'the summary names the mode');

    // What to record: the two modes as radio cards, each saying what it keeps and leaves out.
    const idle = { ...recording, phase: 'notRecording', captureMode: null } as const;
    const record = render({ view: 'record', snapshot: idle });
    assert.match(text(record), /What to record Capture mode Full Everything: the map, battle windows, division details and every dashboard\. Map Smaller files Campaign map, counters and battle windows; no dashboards or division details\./);
    assert.match(record, /role="radiogroup" aria-label="Capture mode"/);
    assert.match(record, /<label class="recorder-mode is-selected"><input type="radio" name="recorder-capture-mode" checked="" value="full"/);
    assert.doesNotMatch(record, /type="radio"[^>]*disabled/, 'the modes can be chosen between sessions');
    assert.match(text(record), /continues in a new recording file\. Game score Operation outcomes Politics/);
    assert.doesNotMatch(record, /Full only/);
    const map = render({ view: 'record', snapshot: idle, settings: { ...settings, captureMode: 'map' } });
    assert.match(map, /<label class="recorder-mode is-selected"><input type="radio" name="recorder-capture-mode" checked="" value="map"/);
    assert.match(text(map), /Game score Operation outcomes Full only Politics Full only/);
    const switches = (map.match(/<input[^>]*role="switch"[^>]*>/g) ?? []).map(input => [/checked=""/.test(input), /disabled=""/.test(input)]);
    assert.deepEqual(switches, [[true, false], [false, true], [false, true]], 'Map shows operation outcomes and politics off and disabled');
    // While recording the mode is shown fixed, and the note says why.
    const fixed = render({ view: 'record' });
    assert.match(fixed, /class="recorder-modes is-fixed"/);
    assert.equal(fixed.match(/type="radio"[^>]*disabled=""/g)?.length, 2);
    assert.match(text(fixed), /Fixed while attached to the game, including at its menu\. A different mode starts a new recording file\./);
    // Attached at the game's menu in Map while the settings now say Full: the session's mode shows, fixed.
    const menu = render({ view: 'record', snapshot: { ...recording, phase: 'waiting', captureMode: 'map' } });
    assert.match(menu, /class="recorder-modes is-fixed"/);
    assert.match(menu, /checked=""[^>]*value="map"/);
  } finally { await vite.close(); }
});
