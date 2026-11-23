import assert from 'node:assert/strict';
import test from 'node:test';
import { takesShot } from './session.mjs';
import { shownHours, catalogueHour, classifyPerfFile, classifyWidgetSnapshot, costSessionConfinement, droppedFrames, failedReplies, firstContent, framesLate, frameTimes, freshness, freshnessByKind, item, losses, mapDrew, mapFrames, mapSeek, pageShown, presentedAt, profileLines, quantiles, sessionConfinement, UNPRESENTED, windowLosses } from './verdicts.mjs';
import { asked, machineLine, noVisibleChange, phaseSeconds, settingDrawn, stepOver, stillInFlight } from './verdicts.mjs';

test('a setting that changed nothing is known once its widget reports drawing it, never while it loads or before it took it', () => {
  const inPage = new Function(`return (${settingDrawn.toString()});`)();
  for (const drawn of [settingDrawn, inPage]) {
    assert.equal(drawn({ settings: 'b', drawn: 'b', state: 'content' }, 'a'), true);
    assert.equal(drawn({ settings: 'b', drawn: 'b', state: 'empty' }, 'a'), true, 'an empty notice is what it drew');
    assert.equal(drawn({ settings: 'b', drawn: 'b', state: 'loading' }, 'a'), false, 'still loading what the setting asked for');
    assert.equal(drawn({ settings: 'b', drawn: 'a', state: 'content' }, 'a'), false, 'committed, not yet painted');
    assert.equal(drawn({ settings: 'a', drawn: 'a', state: 'content' }, 'a'), false, 'never given the setting: only the timeout tells');
    assert.equal(drawn({ settings: 'b', drawn: null, state: 'content' }, 'a'), false);
  }
  assert.equal(noVisibleChange('a', 'a'), 'no visible change after the setting: the widget was never given it');
  assert.equal(noVisibleChange('a', 'b'), 'no visible change after the setting: the widget took it, and nothing it shows changed');
  assert.equal(noVisibleChange('a', null), 'no visible change after the setting: the widget took it, and nothing it shows changed');
});

test('the report header gives each phase its seconds, in the order the sessions ran them', () => {
  const sessions = [{ phases: ['pages'], pagesSeconds: 68.8 }, { phases: ['visual', 'map', 'playback'], visualSeconds: 129, mapSeconds: 44.9 }];
  assert.deepEqual(phaseSeconds(sessions), [['pages', 68.8], ['visual', 129], ['map', 44.9]]);
  assert.deepEqual(phaseSeconds(undefined), []);
});

test('the report header says the load and the apps running beside a run, and what was not recorded', () => {
  const start = { cpuPercent: 23.5, memoryPercent: 61, apps: { thisRun: 0, otherRuns: 1, machine: 3 } };
  const end = { cpuPercent: null, memoryPercent: 64, apps: { thisRun: 1, otherRuns: 1, machine: null } };
  assert.equal(machineLine({ machine: { logicalCpus: 16, start, end } }),
    'Machine (16 logical CPUs): before the first app CPU 23.5%, memory 61%, apps 0 of this run, 1 of other suite runs, 3 on the machine; '
    + 'before the last app closed CPU not recorded, memory 64%, apps 1 of this run, 1 of other suite runs, not recorded on the machine.');
  assert.equal(machineLine({ machine: { logicalCpus: 16, start, end: null } }).endsWith('before the last app closed not recorded.'), true);
  assert.equal(machineLine({}), 'Machine: not recorded.');
});

test('a phase the run did not ask for is not judged; one asked for is, whether or not its session started', () => {
  const quick = { tier: 'quick', requested: ['glance', 'playback', 'catalogue', 'controls'] };
  assert.equal(asked(quick, 'playback'), true);
  assert.equal(asked(quick, 'seeks'), false);
  assert.equal(asked(quick, 'mapfps'), false);
  assert.equal(asked({}, 'mapfps'), true);
  assert.equal(asked({ requested: ['mapfps'] }, 'mapfps'), true, 'asked for: its failure to start fails it');
});

test('a page pass step saves its screenshot whether it passed, ran over time or failed; elsewhere only a failure does', () => {
  assert.equal(takesShot({ pass: true, screenshot: true }), true);
  assert.equal(takesShot({ pass: false, screenshot: true }), true);
  assert.equal(takesShot({ pass: false, screenshot: undefined }), true);
  assert.equal(takesShot({ pass: false, screenshot: false }), false);
  assert.equal(takesShot({ pass: true, screenshot: undefined }), false);
});

test('a page switch is done only once the page it clicked is the one shown, its grid laid out', () => {
  const shown = (current, grid) => new Function('document', `return ${pageShown('Overview')};`)({
    querySelector: selector => selector.includes('aria-current') ? (current === null ? null : { textContent: ` ${current} ` })
      : selector === '.dash-grid-scroll' ? (grid === null ? null : { getClientRects: () => grid ? [{}] : [] }) : null,
  });
  assert.equal(shown(null, null), false, 'the Campaign map shown: no page row current, no grid');
  assert.equal(shown('Overview', false), false, 'the row current, its grid not laid out yet');
  assert.equal(shown('Politics', true), false, 'another page shown');
  assert.equal(shown('Overview', true), true);
});

test('a saved map setting is drawn once the map reports every setting the menu saved', () => {
  const state = { applied: { graphics: { shadows: false, samples: 2, natoCounters: null }, audio: { music: 15 },
    counters: { tags: ['B', 'A'], kind: 'countries' }, visualMods: ['workshop/2', 'workshop/1'] } };
  const expected = [['graphics.shadows', 'false'], ['graphics.samples', '2'], ['graphics.natoCounters', 'null'], ['audio.music', '15'],
    ['counters', '{"kind":"countries","tags":["B","A"]}'], ['visualMods', '["workshop/1","workshop/2"]']];
  assert.equal(mapDrew(state, expected), null);
  assert.equal(mapDrew(state, [['graphics.shadows', 'true']]), 'it drew graphics.shadows as false, the menu saved true');
  assert.equal(mapDrew(state, [['counters', '{"kind":"countries","tags":["A","B"]}']]), 'it drew counters as {"tags":["B","A"],"kind":"countries"}, the menu saved {"kind":"countries","tags":["A","B"]}');
  assert.equal(mapDrew(state, [['visualMods', '[]']]), 'it drew visualMods as ["workshop/2","workshop/1"], the menu saved []');
  assert.equal(mapDrew({ hour: 5 }, expected), 'its map reports no settings it drew with');
});

test('quantiles are nearest-rank, as the host summary computes them', () => {
  assert.deepEqual(quantiles([5, 1, 4, 2, 3]), { n: 5, p50: 3, p95: 5, max: 5 });
  assert.deepEqual(quantiles([]), { n: 0, p50: null, p95: null, max: null });
});

test('an item fails on its maximum, on a failed sample, and when it has no samples', () => {
  assert.equal(item('fast', [{ ms: 20, pass: true, reasons: [] }, { ms: 90, pass: true, reasons: [] }]).pass, true);
  const slow = item('slow', [{ ms: 20, pass: true, reasons: [] }, { ms: 140, pass: true, reasons: [] }]);
  assert.equal(slow.pass, false);
  assert.match(slow.reasons[0], /max 140 ms, over 100 ms/);
  assert.deepEqual(item('broken', [{ ms: 10, pass: false, label: 'open', reasons: ['never finished'] }]).reasons, ['open: never finished']);
  assert.deepEqual(item('none', []).reasons, ['no samples: the case never ran']);
  assert.equal(item('tooltip', [{ ms: 560, pass: true, reasons: [] }], { budget: 600 }).pass, true);
});

test('a widget seen in several steps of a visit counts once, where it first showed content', () => {
  const step = (label, widgets) => ({ area: 'pages', item: 'cold/Overview', label, reasons: [], widgets });
  const placements = firstContent([
    step('switch', [{ id: 1, kind: 'map', title: 'Map', first: 30, ready: null, end: 'content', errors: [] },
      { id: 2, kind: 'feed', title: 'Feed', first: null, ready: null, end: 'loading', errors: [] }]),
    step('scroll 1', [{ id: 2, kind: 'feed', title: 'Feed', first: 40, ready: 60, end: 'content', errors: [] }]),
  ]);
  assert.equal(placements.length, 2);
  assert.deepEqual(placements.map(p => [p.kind, p.visit, p.ms, p.pass]), [['map', 'cold', 30, true], ['feed', 'cold', 60, false]]);
});

test('a map seek ends at the first acknowledgement presenting the moved position', () => {
  const acks = [
    { at: 100, hour: 10, fraction: 0, expected: 10, loading: false },
    { at: 210, hour: 10, fraction: 0, expected: 500, loading: true },
    { at: 260, hour: 500, fraction: 0, expected: 500, loading: false },
  ];
  const frames = [{ t: 'map_open', pid: 1, role: 'main' }, { t: 'map', pid: 1, at: 258, work_ms: 4, painted_at: null }, { t: 'map', pid: 1, at: 266, work_ms: 4, painted_at: 261 }];
  const drawn = acks.map(ack => ({ ...ack, drawn_at: ack.at - 3 }));
  assert.deepEqual(mapSeek(drawn, 200, frames), { ms: 61, reason: null });
  assert.equal(mapSeek(drawn.slice(0, 2), 200, frames).reason, 'never presented the sought hour within 10 s');
  assert.equal(mapSeek(acks, 200, frames).reason, 'its acknowledgement carries no drawn time');
  assert.equal(mapSeek(drawn, 200, frames.slice(0, 2)).reason, UNPRESENTED);
});

test('freshness counts hours behind the clock per playing frame, and frames with no values as never current', () => {
  const records = [
    { t: 'widget', id: 'w', title: 'Widget' },
    { t: 'tick', at: 0, hour: 100, speed: 24, playing: true },
    { t: 'tick', at: 250, hour: 106, speed: 24, playing: true },
    { t: 'shown', at: 0, widget: 'w', hour: 100 },
    { t: 'shown', at: 300, widget: 'w', hour: 106 },
    ...[10, 260, 310, 320].map(at => ({ t: 'frame', at, playing: true })),
  ];
  const result = freshness(records, 0, 1000);
  assert.deepEqual(result.widgets.map(w => [w.title, w.frames, w.p95, w.pass]), [['Widget', 4, 6, true]]);
  const future = freshness([...records, { t: 'shown', at: 315, widget: 'w', hour: 108 }], 0, 1000);
  assert.deepEqual(future.widgets.map(w => [w.ahead, w.pass]), [[1, false]]);
  const stale = freshness(records.map(r => r.t === 'shown' ? { ...r, hour: 12 } : r), 0, 1000);
  assert.equal(stale.widgets[0].pass, false);
});

test('a dropped frame is an interval over 1.5 refreshes, and three in a row fail as stutter', () => {
  const frames = dts => dts.map((dt, index) => ({ t: 'frame', at: index * 10, dt, playing: true }));
  const scattered = droppedFrames(frames([10.6, 22, 10.6, 22, 10.6, 22, 10.6]), 94);
  assert.deepEqual([scattered.dropped, scattered.longest, scattered.pass], [3, 1, true]);
  assert.equal(scattered.rate, 3 / 7);
  const sustained = droppedFrames(frames([10.6, 22, 21.3, 32, 10.6]), 94);
  assert.deepEqual([sustained.longest, sustained.pass], [3, false]);
  assert.equal(droppedFrames([], 94).pass, false);
});

test('page frame times end with timed playback, before the scripted inputs and idle measurement', () => {
  const frame = (at, main, dt, playing = true) => ({ t: 'frame', at, main, dt, playing });
  const measured = frameTimes([frame(0, 2, 10), frame(1, 90, 100, false), frame(2, 4, 12), frame(3, 120, 80)], 0, 2);
  assert.equal(measured.main.p95, 4);
  assert.equal(measured.interval.p95, 12);
  assert.equal(measured.main.n, 2);
});

test('the map is judged at 120 Hz on its main process frames, and a highlight after a whole later frame is late', () => {
  const records = [
    { t: 'map_open', pid: 1, role: 'main' }, { t: 'map_open', pid: 2, role: 'popout' },
    { t: 'map', pid: 1, frame: 1, at: 110, work_ms: 4, dt: 10.5 }, { t: 'map', pid: 1, frame: 2, at: 121, work_ms: 6.5, dt: 10.5, acquire_ms: 0, painted_at: 110.25 },
    { t: 'map', pid: 2, frame: 1, at: 115, work_ms: 30, dt: 40 },
    { t: 'map', pid: 1, frame: 3, at: 140, work_ms: 4, dt: 10.5, acquire_ms: 0, painted_at: 121.25, gpu_frame: 1, gpu_ms: 3 },
    { t: 'map', pid: 1, frame: 4, at: 151, work_ms: 4, dt: 10.5, acquire_ms: 0, painted_at: 140.25, gpu_frame: 2, gpu_ms: 3.5 },
  ];
  const fine = mapFrames(records, 100, 130);
  assert.deepEqual([fine.frames, fine.dropped, fine.pass], [2, 0, true]);
  assert.equal(fine.gpu.max, 3.5);
  const slow = mapFrames([...records.map(r => (r.frame === 3 && r.pid === 1 ? { ...r, painted_at: 125.5 } : r)),
    { t: 'map', pid: 1, frame: 9, at: 125, work_ms: 9, dt: 10.5, acquire_ms: 0, painted_at: 121.25 }], 100, 130);
  assert.deepEqual([slow.dropped, slow.pass], [1, false]);
  assert.match(slow.reasons.join('; '), /work max 9.5 ms, over 8.33 ms/);
  assert.match(slow.reasons.join('; '), /1 of 3 frames have no GPU time and no reason/);
  assert.match(mapFrames(records.map(({ gpu_ms, gpu_frame, ...r }) => r), 100, 130).reasons.join('; '), /2 of 2 frames have no GPU time and no reason/);
  assert.match(mapFrames(records.map(r => (r.gpu_frame === 1 ? { ...r, gpu_ms: null, gpu_error: 'the query was lost' } : r)), 100, 130).reasons.join('; '), /1 of 2 frames have no GPU time: the query was lost/);
  const unsupported = records.map(r => (r.t === 'map_open' && r.pid === 1 ? { ...r, gpu_timer: 'unsupported', gpu_timer_reason: 'no timestamp queries' } : r));
  assert.match(mapFrames(unsupported, 100, 130).reasons.join('; '), /cannot time the GPU on this adapter: no timestamp queries/);
  assert.equal(framesLate(records, 105, 109), 0);
  assert.equal(framesLate(records, 105, 120), 1);
});

test('a widget kind fails by name when a placement never reports the hour it shows', () => {
  const records = [
    { t: 'tick', at: 0, hour: 10, speed: 24 },
    { t: 'shown', at: 0, widget: 'a', hour: 10 },
    { t: 'frame', at: 5, playing: true },
  ];
  const kinds = freshnessByKind(records, [{ page: 'Overview', speed: 24, from: 0, to: 10,
    widgets: [{ id: 'a', kind: 'scoreboard', title: 'Scoreboard', visible: true }, { id: 'b', kind: 'map', title: 'Map', visible: true },
      { id: 'c', kind: 'feed', title: 'Feed', visible: false }] }],
    [{ kind: 'feed', page: 'Overview', title: 'Feed', widget: 'c', at: 50, hour: 10 }]);
  assert.deepEqual(kinds.map(kind => [kind.kind, kind.pass]), [['feed', false], ['scoreboard', true], ['map', false]]);
  assert.match(kinds[2].reasons[0], /does not report the hour it shows \(Overview: Map\)/);
  assert.match(kinds[0].reasons[0], /does not report the hour it shows \(Overview: Feed\)/);
});

test('the worker presentation gate owns dated widgets in the measured window', () => {
  const widget = { id: 'w', kind: 'research', title: 'Research history', visible: true };
  const window = { page: 'Research', speed: 1440, from: 10, to: 50, widgets: [widget] };
  const dated = { from: 10, widget: 'w', p95: 105, eligibleP95: 37, future: 0, exact: true, pass: true };
  const judge = placements => freshnessByKind([], [window], [], [], placements)[0];
  assert.equal(judge([dated]).pass, true);
  assert.equal(judge([dated]).worst, 105);
  assert.equal(judge([dated]).eligible, 37);
  assert.equal(judge([{ ...dated, pass: false }]).pass, false);
  assert.equal(judge([{ ...dated, from: 0 }]).pass, false);
});

test('a reply cut off by the suite closing its session is not a failure, and nothing else is excused', () => {
  const records = [
    { t: 'q', product: 'a', at: 100, ms: 5, outcome: 'error', failure: 'refused: no' },
    { t: 'q', product: 'b', at: 190, ms: 50, outcome: 'error', failure: 'failed: The dashboard data connection closed' },
    { t: 'q', product: 'c', at: 190, ms: 50, outcome: 'error', failure: 'failed: the reader crashed' },
    { t: 'q', product: 'd', at: 120, ms: 5, outcome: 'error', failure: 'failed: The dashboard data connection closed' },
  ];
  const closing = { t: 'q', product: 'art', ms: 2, outcome: 'error', failure: 'failed: The dashboard is closing' };
  const log = [...records, { ...closing, at: 199 }, { ...closing, at: 204 }, { ...closing, at: 180 },
    { ...closing, at: 210, failure: 'failed: a real read error' }].map(r => ({ ...r, session: 'quick' }));
  log.push({ ...closing, session: 'other', at: 220 });
  const { failures, shutdown } = failedReplies(log, new Map([['quick', 200]]));
  assert.equal(shutdown, 3);
  assert.deepEqual(failures.map(r => [r.product, r.count]), [['a', 1], ['c', 1], ['d', 1], ['art', 3]]);
  assert.equal(failedReplies(log).shutdown, 0);
});

test('a map frame is presented when the paint the next frame record carries ended', () => {
  const records = [{ t: 'map_open', pid: 1, role: 'main' }, { t: 'map_open', pid: 2, role: 'popout' },
    { t: 'map', pid: 2, at: 108, work_ms: 8, painted_at: 101 }, { t: 'map', pid: 2, at: 120, work_ms: 2, painted_at: 111 },
    { t: 'map', pid: 1, at: 110, work_ms: 6, painted_at: 103 }, { t: 'map', pid: 1, at: 118, work_ms: 3, painted_at: 114.5 }];
  assert.equal(presentedAt(records, 107), 114.5);
  assert.equal(presentedAt(records, 112), null);
  assert.equal(presentedAt(records, 116), null);
});

test('an added catalogue view must report showing the clock hour', () => {
  const records = [{ t: 'shown', widget: 'w', at: 20, hour: 7 }];
  assert.equal(catalogueHour(records, { widget: 'w', input: 10, hour: 7 }).pass, true);
  assert.match(catalogueHour(records, { widget: 'w', input: 10, hour: 8 }).reason, /the clock is at 8/);
  assert.equal(catalogueHour(records, { widget: 'x', input: 10, hour: 7 }).reason, 'does not report the hour it shows');
});

test('a map frame counts its work through tessellation to the end of its paint, never the wait for a swapchain image', () => {
  const records = [
    { t: 'map_open', pid: 1, role: 'main' },
    { t: 'map', pid: 1, frame: 1, at: 110, work_ms: 4, gpu_frame: 1, gpu_ms: 2 },
    { t: 'map', pid: 1, frame: 2, at: 150, work_ms: 1, paint_ms: 6, acquire_ms: 3, painted_at: 118, gpu_frame: 2, gpu_ms: 2 },
  ];
  assert.equal(mapFrames(records, 100, 120).work.max, 9);
  assert.match(mapFrames(records.slice(0, 2), 100, 120).reasons[0], /no paint for any frame/);
});

test('a map reports the hour it shows through its acknowledged frames', () => {
  const records = [{ t: 'map_state', view: 'm', at: 5, hour: 3, loading: false }, { t: 'map_state', view: 'm', at: 6, hour: 4, loading: true }, { t: 'shown', widget: 'w', at: 7, hour: 3 }];
  assert.deepEqual(shownHours(records).map(record => [record.widget, record.hour]), [['w', 3], ['m', 3]]);
});

test('every record a perf log dropped is a loss by its source', () => {
  const found = losses([{ t: 'lost', source: 'ui', count: 3, from: 10, to: 20 }, { t: 'lost', source: 'ui', count: 2, at: 30 }, { t: 'q' }]);
  assert.deepEqual(found, [{ source: 'ui', count: 5, from: 10, to: 30 }]);
});

test('with the app\'s count of what it has in flight, a step is over one frame after it reaches zero; without it, after its quiet window', () => {
  const inPage = new Function(`return (${stepOver.toString()});`)();
  for (const over of [stepOver, inPage]) {
    assert.equal(over({ settled: true, busy: 0, idle: 1, heldMs: 0, wait: 200 }), false, 'zero for one sample: a request may start on the next');
    assert.equal(over({ settled: true, busy: 0, idle: 2, heldMs: 17, wait: 200 }), true, 'one frame later, without the 200 ms window');
    assert.equal(over({ settled: true, busy: 1, idle: 0, heldMs: 900, wait: 200 }), false, 'a late request in flight holds the step however long it looked settled');
    assert.equal(over({ settled: false, busy: 0, idle: 0, heldMs: 0, wait: 200 }), false, 'a widget still loading, or its change not shown');
    assert.equal(over({ settled: true, busy: null, idle: 0, heldMs: 199, wait: 200 }), false);
    assert.equal(over({ settled: true, busy: null, idle: 0, heldMs: 200, wait: 200 }), true);
  }
  assert.equal(stillInFlight({ requests: 2, updates: 0, commits: 0, draws: 1, follow: 0, waits: 0 }, 6000), 'still in flight after 6 s: requests 2, draws 1');
});

test('a seek holds its captured pin through the next input, even after a correct first report', async () => {
  const { sought } = await import('./verdicts.mjs');
  const seek = { hour: 99, input: 100, pass: true, reasons: [], widgetIds: { p: 'Panel' } };
  const records = [{ at: 95, hour: 27, fixedHour: 27 }, { at: 105, hour: 27, fixedHour: 27 },
    { at: 115, hour: 99, fixedHour: 99 }, { at: 120, hour: 27, fixedHour: 27 }];
  assert.equal(sought(seek, new Map([['p', records]]), [], 110).pass, true);
  assert.equal(sought(seek, new Map([['p', records]]), [], 125).pass, false);
  const latePin = [{ at: 105, hour: 40, fixedHour: 40 }, { at: 110, hour: 99 }, { at: 112, hour: 40, fixedHour: 40 }];
  assert.equal(sought(seek, new Map([['p', latePin]]), [], 120).pass, false);
  assert.equal(sought(seek, new Map([['p', [{ at: 105, hour: 99 }, { at: 110, hour: 98 }]]]), [], 120).pass, false);
});

test('a visible placeholder fails freshnessByKind with reason not migrated while staying distinct from a real failure', () => {
  const widget = { id: 'p', kind: 'legacy', title: 'Legacy', visible: true, notMigrated: true };
  const window = { page: 'Overview', speed: 1440, from: 10, to: 50, widgets: [widget] };
  const dated = [{ from: 10, widget: 'p', status: 'not migrated', pass: false }];
  const [entry] = freshnessByKind([], [window], [], [], dated);
  assert.equal(entry.pass, false);
  assert.equal(entry.notMigrated, 1);
  assert.equal(entry.status, 'not migrated');
  assert.deepEqual(entry.reasons, ['not migrated']);
});

test('windowLosses counts only perf log losses overlapping a window, and losses names affected windows', () => {
  const records = [
    { t: 'lost', source: 'ui', count: 5, from: 100, to: 200 },
    { t: 'lost', source: 'ui', count: 7, from: 500, to: 600 },
  ];
  assert.equal(windowLosses(records, 150, 300).count, 5);
  assert.equal(windowLosses(records, 250, 450).count, 0);
  const summary = losses(records, [{ label: 'Overview 1440 h/s', from: 150, to: 300 }]);
  assert.deepEqual(summary[0].windows, ['Overview 1440 h/s']);
});

test('classifyWidgetSnapshot classifies strictly from explicit DOM state and keeps text as diagnostic detail (both direct and serialized)', () => {
  const serialized = new Function(`return (${classifyWidgetSnapshot.toString()});`)();
  for (const fn of [snap => classifyWidgetSnapshot(snap), snap => serialized(snap)]) {
    assert.deepEqual(fn({ domState: 'error', errorDetail: 'Changed rows arrived without the table they change.' }),
      { outcome: 'error', detail: 'Changed rows arrived without the table they change.', notMigrated: false });
    assert.deepEqual(fn({ domState: 'error', errorDetail: 'No table 44 in the schema' }),
      { outcome: 'error', detail: 'No table 44 in the schema', notMigrated: false },
      'an explicit error state is never reclassified as a refusal by message text');
    assert.deepEqual(fn({ domState: 'unavailable', unavailableText: 'No table 44 in the schema' }),
      { outcome: 'refusal', detail: 'No table 44 in the schema', notMigrated: false });
    assert.deepEqual(fn({ domState: 'outside', outsideText: 'This time is outside the recording.' }),
      { outcome: 'outside', detail: 'This time is outside the recording.', notMigrated: false });
    assert.deepEqual(fn({ domState: 'not-recorded', notRecordedText: 'Not recorded at this time.' }),
      { outcome: 'not-recorded', detail: 'Not recorded at this time.', notMigrated: false });
    assert.deepEqual(fn({ domState: 'empty', emptyText: 'No battles yet.' }),
      { outcome: 'content', detail: 'No battles yet.', notMigrated: false });
    assert.deepEqual(fn({ probeState: 'loading' }), { outcome: 'error', detail: 'still loading', notMigrated: false });
    assert.deepEqual(fn({ probeState: 'loading', probeDrawing: true }), { outcome: 'content', detail: null, notMigrated: false });
    assert.deepEqual(fn({ probeState: 'blank' }), { outcome: 'error', detail: 'blank widget body', notMigrated: false });
    assert.deepEqual(fn({ domState: 'placeholder', placeholderText: 'Scoreboard paused' }),
      { outcome: 'placeholder', detail: 'Scoreboard paused', notMigrated: true },
      'explicit placeholder DOM state classifies without English text matching');
    assert.deepEqual(fn({ placeholderText: "Scoreboard isn't playing back yet" }),
      { outcome: 'error', detail: "Scoreboard isn't playing back yet", notMigrated: false },
      'plain text without explicit placeholder marker is not matched by regex');
  }
});

test('classifyPerfFile classifies viewer map logs once with their owning session and never as page logs', () => {
  assert.deepEqual(classifyPerfFile('perf-playback.jsonl'), { kind: 'page', session: 'playback' });
  assert.deepEqual(classifyPerfFile('perf-playback.jsonl.map.1.jsonl'), { kind: 'viewer', session: 'playback' });
  assert.deepEqual(classifyPerfFile('perf-widget-costs.jsonl.map.42.jsonl'), { kind: 'viewer', session: 'widget-costs' });
  assert.equal(classifyPerfFile('other.jsonl'), null);
});

test('costSessionConfinement exports missing observations as null without borrowing from another session', () => {
  const run = {
    confinement: { jobProcesses: 4, confinedProcesses: 4, allConfined: true },
    sessions: [
      { name: 'playback', confinement: { jobProcesses: 4, confinedProcesses: 4, allConfined: true } },
      { name: 'widget costs' },
    ],
  };
  const sweep = {
    meta: {
      sessions: [
        { session: 'playback', confinement: { hardwareConcurrency: 4, lanes: 2 } },
        { session: 'widget-costs' },
      ],
    },
  };
  assert.deepEqual(costSessionConfinement(run, sweep, 'widget-costs'), { job: null, page: null });
  assert.deepEqual(costSessionConfinement(run, sweep, 'playback'), {
    job: { jobProcesses: 4, confinedProcesses: 4, allConfined: true },
    page: { hardwareConcurrency: 4, lanes: 2 },
  });
});

test('sessionConfinement checks each session Job affinity, hardwareConcurrency, and lane count without cross-pairing', () => {
  const run = {
    profile: 'weak',
    cpuThrottle: 4,
    cores: { count: 4, mask: '0x55', cpus: [0, 2, 4, 6] },
    sessions: [
      { name: 'pages', phases: ['pages'], confinement: { jobProcesses: 4, confinedProcesses: 4, allConfined: true } },
      { name: 'playback', phases: ['playback'], confinement: { jobProcesses: 4, confinedProcesses: 3, allConfined: false } },
    ],
  };
  const sweep = {
    meta: {
      cpuThrottle: 4,
      sessions: [
        { session: 'pages', phases: ['pages'], confinement: { hardwareConcurrency: 4, lanes: null } },
        { session: 'playback', phases: ['playback'], confinement: { hardwareConcurrency: 16, lanes: 4 } },
      ],
    },
  };
  const pagesConf = sessionConfinement(run, run.sessions[0], sweep.meta.sessions[0]);
  assert.equal(pagesConf.pass, true);
  const playbackConf = sessionConfinement(run, run.sessions[1], sweep.meta.sessions[1]);
  assert.equal(playbackConf.pass, false);
  assert.ok(playbackConf.reasons.some(r => r.includes('3/4')));
  assert.ok(playbackConf.reasons.some(r => r.includes('hardwareConcurrency = 16')));
  assert.ok(playbackConf.reasons.some(r => r.includes('4 drawing lanes, expected 2')));
  const header = profileLines(run, sweep);
  assert.ok(header[0].includes('pages: Job 4/4, hardwareConcurrency=4'));
  assert.ok(header[0].includes('playback: Job 3/4 (NOT ALL CONFINED), hardwareConcurrency=16, 4 lanes'));
});
