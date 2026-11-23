import assert from 'node:assert/strict';
import test from 'node:test';
import { FAIL, PARTIAL, PASS, UNMEASURABLE, dashboardRows, floorOf, pacedRows, proofRows, spread, wholeHarness, wholeLog } from './playbackVerdicts.mjs';

const T = 1e6 / 60;
const FROM = 1000;

function paced(n = 120, edit = () => {}, { seeks = null, exit = 0 } = {}) {
  const frames = Array.from({ length: n }, (_, k) => ({ frame: k, start_us: k * T, wall_us: 5000, thread_us: 3000, head_hour: FROM + 24 * k,
    drawn_hour: k ? FROM + 24 * k - 30 : null, lag_hours: k ? 30 : null, new_hour: k > 0, layers: { drawn: FROM + 24 * k - 10, lag_hours: 10 } }));
  edit(frames);
  const args = ['--playback-cost', 'rec', '--from-hour', String(FROM), '--hours', String(24 * n), '--refresh-hz', '60', '--paced', '1440',
    ...(seeks ? ['--seeks', String(seeks.length)] : [])];
  const report = { from_hour: FROM, game_hours: 24 * n, memory: { working_set_peak_bytes: 2 ** 30, private_peak_bytes: 2 ** 31 },
    paced: { frames: n, refresh_hz: 60, speed_hours_per_second: 1440, seconds: frames.at(-1).start_us / 1e6 + 1 / 60 }, layers: { seeks: seeks ?? [] } };
  return { run: { exit, args, playhead: true, label: 'layers' }, report, frames: { frames, cut: false } };
}

const verdictOf = (rows, bar) => rows.find(row => row.bar === bar).verdict;

test('a paced run is whole only when it exited 0, ran its span to the end and made every seek', () => {
  const run = paced();
  assert.deepEqual(wholeHarness(run.run, run.report, run.frames), { whole: true, reasons: [], accepted: null });
  const unreconciled = { ...run.report, cpu_reconciliation: { passed: false, process_minus_scopes_ns: -8e7, process_cpu_ns: 1e9 } };
  assert.match(wholeHarness({ ...run.run, exit: 1 }, unreconciled, run.frames).reasons[0], /CPU reconciliation failed \(process CPU off its scopes by -8\.0% on 1\.0 s/);
  assert.match(wholeHarness({ ...run.run, exit: 1 }, unreconciled, run.frames, { acceptReconciliation: true }).accepted, /not a result under the rule/);
  assert.equal(wholeHarness({ ...run.run, exit: 2 }, unreconciled, run.frames, { acceptReconciliation: true }).whole, false);
  assert.match(wholeHarness({ ...run.run, exit: 1 }, run.report, run.frames).reasons[0], /exit 1/);
  assert.match(wholeHarness({ ...run.run, stopped: 'stopped after 10 s, as asked', exit: 1 }, run.report, run.frames).reasons[0], /stopped by the runner/);
  assert.match(wholeHarness(run.run, run.report, { ...run.frames, cut: true }).reasons.join(), /cut line/);
  assert.match(wholeHarness({ ...run.run, memory: { least_free_mb: 900 } }, run.report, run.frames).reasons[0], /the machine was paging: 900 MB/);
  assert.equal(wholeHarness({ ...run.run, memory: { least_free_mb: 9000 } }, run.report, run.frames).whole, true);
  const short = paced(60);
  const cut = { ...short.report, game_hours: 24 * 120 };
  assert.match(wholeHarness({ ...short.run, args: short.run.args.map(value => (value === String(24 * 60) ? String(24 * 120) : value)) }, cut, short.frames).reasons.join(), /the head reached 2440, asked 3880/);
  assert.match(wholeHarness({ ...run.run, args: [...run.run.args, '--seeks', '2'] }, { ...run.report, layers: { seeks: [{ frame: 90, frames_to_shown: 2 }] } }, run.frames).reasons.join(), /1 of 2 seeks shown/);
  const back = paced(120, frames => { for (let k = 100; k < 120; k++) frames[k].head_hour = FROM + 240 + 24 * (k - 100); frames[99].head_hour = FROM + 24 * 120; });
  const sought = { ...back.run, args: [...back.run.args, '--seek-back', String(FROM + 240)] };
  const shown = { ...back.report, seek_back: FROM + 240, layers: { seeks: [{ hour: FROM + 240, frame: 100, frames_to_shown: 2 }] } };
  assert.deepEqual(wholeHarness(sought, shown, back.frames), { whole: true, reasons: [], accepted: null });
  assert.match(wholeHarness(sought, { ...shown, layers: { seeks: [] } }, back.frames).reasons.join(), /the seek back to 1240 was not shown/);
  assert.match(wholeHarness(sought, { ...shown, seek_back: null }, back.frames).reasons.join(), /asked to seek back to 1240, the report says none/);
  assert.match(wholeHarness(back.run, back.report, back.frames).reasons.join(), /the head reached 1720, asked 3880/);
  const step = (extra = []) => ({ exit: 0, args: ['--playback-cost', 'rec', '--from-hour', String(FROM), '--hours', '3', '--speed', 'max', ...extra], playhead: true, label: 'layers' });
  const stepped = n => ({ frames: Array.from({ length: n }, (_, k) => ({ frame: k })), cut: false });
  assert.equal(wholeHarness(step(['--seek-back', String(FROM)]), { from_hour: FROM, game_hours: 3, seek_back: FROM }, stepped(4)).whole, true);
  assert.match(wholeHarness(step(['--seek-back', String(FROM)]), { from_hour: FROM, game_hours: 3, seek_back: FROM }, stepped(3)).reasons.join(), /3 of 4 frames/);
});

test('a run whose last frame was long reached its end inside that frame, so it is whole', () => {
  const run = paced(100, frames => { frames[99].wall_us = 650_000; });
  run.report.paced.seconds = (run.frames.frames[99].start_us + 650_000) / 1e6;
  assert.equal(wholeHarness(run.run, run.report, run.frames).whole, true);
});

test('the map\'s bars name the worst frame, and a partial run judges nothing', () => {
  const run = paced(120, frames => {
    frames[50].thread_us = 12_000;
    frames[70].wall_us = 30_000;
    for (let k = 71; k < 120; k++) frames[k].start_us = Math.max(k * T, frames[k - 1].start_us + frames[k - 1].wall_us);
    frames[90].layers.lag_hours = 60;
  });
  const rows = pacedRows({ ...run, label: 'layers', floor: { rate: 0, late: 0, frames: 600 } });
  const thread = rows.find(row => row.bar === 'window-thread CPU a frame');
  assert.equal(thread.verdict, FAIL);
  assert.deepEqual(thread.worst, { frame: 50, hour: FROM + 24 * 50 });
  const interval = rows.find(row => row.bar === 'presented frame interval');
  assert.equal(interval.verdict, FAIL);
  assert.equal(interval.values.over, 1);
  assert.equal(interval.worst.frame, 70);
  assert.equal(verdictOf(rows, 'late frames against the floor'), FAIL);
  const layers = rows.find(row => row.bar === 'content lag, the layers');
  assert.equal(layers.verdict, FAIL, 'worst 60 h is over two steps');
  assert.deepEqual([layers.values.p99, layers.worst.frame], [10, 90]);
  assert.equal(verdictOf(rows, 'playhead step time'), UNMEASURABLE);
  const fine = pacedRows({ ...paced(), label: 'layers', floor: { rate: 0, late: 0, frames: 600 } });
  assert.deepEqual(['window-thread CPU a frame', 'presented frame interval', 'late frames against the floor', 'content lag, the layers'].map(bar => verdictOf(fine, bar)), [PASS, PASS, PASS, PASS]);
  const neverDrawn = pacedRows({ ...paced(120, frames => frames.forEach(frame => { frame.layers = {}; })), label: 'layers' })
    .find(row => row.bar === 'content lag');
  assert.deepEqual([neverDrawn.verdict, neverDrawn.measured], [FAIL, 'no picture was drawn']);
  const partial = pacedRows({ ...run, run: { ...run.run, exit: null, stopped: 'stopped after 5 s, as asked' }, label: 'layers' });
  assert.ok(partial.every(row => row.verdict === PARTIAL && row.measured === '–'));
});

test('a seek is timed from its frame to the present of the frame showing it, with no frame over budget meanwhile', () => {
  const seeks = [{ frame: 30, frames_to_shown: 2, frames_to_lag_within_a_step: 3, hour: 1500 }, { frame: 60, frames_to_shown: 9, frames_to_lag_within_a_step: 10, hour: 1600 }];
  const run = paced(120, frames => { frames[33].layers.lag_hours = 900; }, { seeks });
  const rows = pacedRows({ ...run, label: 'layers-seeks' });
  const seek = rows.find(row => row.bar === 'seek: sought hour on screen');
  assert.equal(seek.verdict, FAIL, 'the second seek takes 9 frames, 155 ms');
  assert.deepEqual([Math.round(seek.values.median), Math.round(seek.values.worst), seek.worst.frame], [38, 155, 60]);
  assert.equal(verdictOf(rows, 'content lag, the layers'), PASS, 'lag inside a seek window is the seek\'s');
});

test('the floor is paced frames at a held hour, a frame late when presented over one and a half intervals after the one before', () => {
  const frames = Array.from({ length: 11 }, (_, k) => ({ frame: k, start_us: k * T, wall_us: k === 5 ? 30_000 : 4000, head_hour: FROM }));
  const args = ['--playback-cost', 'rec', '--from-hour', String(FROM), '--hours', '11', '--refresh-hz', '60', '--paced', '1440', '--paused', '--pan', '8'];
  const report = { from_hour: FROM, game_hours: 11, paced: { frames: 11, refresh_hz: 60, speed_hours_per_second: 1440, seconds: 0.2 } };
  const floor = floorOf({ run: { exit: 0, args }, report, frames: { frames, cut: false } });
  assert.deepEqual([floor.whole, floor.late, floor.frames, floor.worst.frame], [true, 1, 10, 5]);
  assert.match(wholeHarness({ exit: 0, args }, report, { frames: frames.slice(0, 9), cut: false }).reasons.join(), /9 of 11 frames at the held hour/);
});

test("each layer is judged against its own bar on its own CPU, and the stepper's step, stalls and held bytes are read", () => {
  const run = paced(120, frames => frames.forEach((frame, k) => Object.assign(frame.layers, { steps: 1, step_ms: k === 40 ? 12 : 2, cpu_ms: [[0.05, 0.05, k === 60 ? 2 : 0.1], [0.4, 0, 0]] })));
  const scope = { wall_ms: { p99: 0.2 }, cpu_ms: { p99: 0.1 } };
  run.report.layers.layers = [{ name: 'political', cpu_ms: { p50: 0.2, p99: 0.25, max: 2.1 }, wall_ms: { p99: 0.3, max: 2.2 }, apply: scope, settle: scope, upload: scope },
    { name: 'vision', cpu_ms: { p50: 0.4, p99: 0.4, max: 0.4 }, wall_ms: { p99: 0.4, max: 0.4 } }, { name: 'naval', cpu_ms: { p50: 0, p99: 0, max: 0 }, wall_ms: {} }];
  run.report.layers.step_ms = { p50: 2, p99: 2, max: 12 };
  run.report.stepper = { steps: 120, stalls: 0, stalled_steps: 0, playhead_bytes: { peak: 30e6 }, host_bytes: { total: 40e6 } };
  const rows = pacedRows({ ...run, label: 'layers' });
  const political = rows.find(row => row.bar === 'layer political: apply, settle and upload');
  assert.deepEqual([political.verdict, political.worst.frame], [FAIL, 60], 'worst 2.1 ms over its 1 ms');
  assert.equal(verdictOf(rows, 'layer vision: apply, settle and upload'), PASS);
  assert.equal(verdictOf(rows, 'layer naval: apply, settle and upload'), 'reported');
  assert.equal(rows.find(row => row.bar === 'window-thread CPU a frame, less the vision layer').values.p99, 2.6);
  const step = rows.find(row => row.bar === 'playhead step time');
  assert.deepEqual([step.verdict, step.worst.frame], [FAIL, 40]);
  assert.deepEqual(['playhead stalls', 'playhead held()'].map(bar => verdictOf(rows, bar)), [PASS, PASS]);
});

test('a run cut short says what it covered', () => {
  const run = paced(60);
  assert.match(wholeHarness({ ...run.run, exit: 1, stopped: 'stopped after 9 s, as asked' }, null, run.frames).reasons.join('; '), /covered 60 frames, the head to 2416 of 2440/);
  assert.equal(wholeHarness(run.run, run.report, run.frames).whole, true);
});

test('the proof run judges stalls and the playhead\'s held bytes', () => {
  const summary = { last: 2000, flat: { last: 2000 }, resets: [{}, {}], frames: { count: 10 }, timing: true, stalls: 1, steps: 40, playhead_held_peak: 250e6,
    held_series: [{ hour: 1100, playhead: 100e6 }, { hour: 1900, playhead: 250e6 }], seeks: [{ shown_ms: 20, target: 1500 }, { shown_ms: 40, target: 1600 }] };
  const rows = proofRows({ label: 'proof', run: { exit: 0, args: ['--playhead-proof', 'rec', '--seeks', '2'] }, summary, frames: { frames: [{}], cut: false } });
  assert.deepEqual(['playhead stalls', 'playhead held()'].map(bar => verdictOf(rows, bar)), [FAIL, FAIL]);
  assert.equal(rows.find(row => row.bar === 'playhead held()').worst.hour, 1900);
  assert.ok(proofRows({ label: 'proof', run: { exit: 0, args: ['--playhead-proof', 'rec', '--seeks', '3'] }, summary, frames: { frames: [{}], cut: false } })
    .every(row => row.verdict === PARTIAL));
});

test('repeats give each figure\'s spread over whole runs', () => {
  const rows = [1, 2, 3].map(repeat => ({ page: 'map', run: 'layers', bar: 'b', repeat, verdict: PASS, values: { p99: repeat * 2 } }));
  assert.deepEqual(spread([...rows, { ...rows[0], repeat: 4, verdict: PARTIAL, values: { p99: 99 } }])[0].values, { p99: [2, 6] });
});

function syntheticLog() {
  const records = [{ t: 'page', at: 0, loaf: true }, { t: 'widget', at: 0, id: 'w1', title: 'Political power' }, { t: 'widget', at: 0, id: 'w2', title: 'Stability' },
    { t: 'follow', at: 1, id: 1, status: 'pending', product: 'series-state' }, { t: 'follow_widget', at: 1, widget: 'w1', id: 1 }, { t: 'follow', at: 5, id: 1, status: 'followed' },
    { t: 'follow', at: 1, id: 2, status: 'pending', product: 'series-state' }, { t: 'follow_widget', at: 1, widget: 'w2', id: 2 },
    { t: 'follow', at: 6, id: 2, status: 'unavailable', reason: 'CountryStability is derived and not in the unit files' }];
  const frame = 1000 / 60;
  let at = 0, hour = 5000, seek = 0;
  const late = 2500;
  for (let index = 0; index < 240; index++) {
    const playing = index >= 60;
    const dt = Math.abs(at + frame - late) < frame / 2 ? 40 : frame;
    at += dt;
    if (playing) hour += 24 * dt / frame;
    records.push({ t: 'tick', at: at + 1, hour, playing, speed: 1440, sequence: index, seek });
    const commit = (playing && index === 61) || index === 10 ? 1 : 0;
    records.push({ t: 'frame', at, dt, playing, late: 0.1, commits: commit, main: 2, ...(commit && { zones: { 'widget:w1': 1 } }) });
  }
  const planted = records.find(record => record.t === 'frame' && record.at > 3000);
  Object.assign(planted, { commits: 1, zones: { 'widget:w1': 1 } });
  records.push({ t: 'q', at: planted.at + 2, cmd: 'dashboard_product', product: 'series-state', outcome: 'ok', ms: 1 });
  records.push({ t: 'q', at: 1005, cmd: 'dashboard_product', product: 'series-state', outcome: 'ok', ms: 1 },
    { t: 'q', at: 500, cmd: 'dashboard_product', product: 'army-size', outcome: 'ok', ms: 1 },
    { t: 'q', at: 3500, product: 'series-state', cache: 'hit' }, { t: 'q', at: 3600, cmd: 'dashboard_product', product: 'follow-clock', outcome: 'ok', ms: 1 },
    { t: 'q', at: 3700, cmd: 'dashboard_product', product: 'series-tiles', outcome: 'ok', ms: 1, tile: { key: 'k', bins: 6, index: 4 } });
  return { records: records.sort((a, b) => a.at - b.at), cut: false };
}

test('the dashboard analyser judges frames and head reads, leaving commit attribution to the dated audit', () => {
  const log = syntheticLog();
  log.records.push({ t: 'q', at: 3400, cmd: 'dashboard_product', product: 'sprite-artwork', hour: null, outcome: 'ok' });
  log.records.find(r => r.t === 'frame' && r.at > 3400).work = { w2: { read: 1 } };
  const { whole, rows, follow } = dashboardRows(log, { page: 'Politics', refreshHz: 60 });
  assert.equal(whole.whole, true);
  const frames = rows.find(row => row.bar === 'webview frame interval' && row.page === 'Politics');
  assert.equal(frames.verdict, FAIL);
  assert.equal(frames.values.over, 1);
  const lateFrame = log.records.filter(record => record.t === 'frame').findIndex(record => record.dt === 40);
  assert.equal(frames.worst.frame, lateFrame);
  const commits = rows.filter(row => row.bar === 'React commits caused by a value');
  assert.equal(commits.length, 0);
  const requests = rows.filter(row => row.bar === 'requests for values at the head');
  assert.deepEqual(requests.map(row => [row.page, row.values.count, row.verdict]), [['Politics', 1, FAIL]]);
  assert.equal(rows.find(row => row.bar === 'chart tile requests').verdict, PASS);
  assert.equal(rows.find(row => row.bar === 'long animation frames').verdict, PASS);
  assert.deepEqual(follow.widgets.map(widget => [widget.title, widget.followed, widget.fellBack]),
    [['Political power', ['series-state'], []], ['Stability', [], ['series-state: CountryStability is derived and not in the unit files']]]);
});

test('a log that lost records judges nothing', () => {
  const log = syntheticLog();
  const lost = { ...log, records: [...log.records, { t: 'lost', at: 9000, source: 'ui', count: 3 }] };
  assert.ok(dashboardRows(lost, { refreshHz: 60 }).rows.every(row => row.verdict === PARTIAL));
  assert.match(wholeLog(log, log.records, { playingSeconds: 10 }).reasons[0], /played 3\.\d s, asked 10/);
});

test('a chart that asks a tile twice, or more than the head crossed, fails', () => {
  const log = syntheticLog();
  log.records.push({ t: 'q', at: 3800, cmd: 'dashboard_product', product: 'series-tiles', outcome: 'ok', ms: 1, tile: { key: 'k', bins: 6, index: 4 } });
  const tiles = dashboardRows(log, { refreshHz: 60 }).rows.find(row => row.bar === 'chart tile requests');
  assert.equal(tiles.verdict, FAIL);
  assert.match(tiles.note, /1 asked twice/);
});

test('a list\'s request for a block wholly ahead of the head passes once per block; at or behind the head, or a block asked twice, fails', () => {
  const log = syntheticLog();
  const ticks = log.records.filter(record => record.t === 'tick' && record.playing);
  const headAt = time => Math.floor(ticks.filter(tick => tick.at <= time).at(-1).hour);
  const page = (at, from, lane = 'battles') => ({ t: 'q', at, cmd: 'dashboard_product', product: 'land-battles', outcome: 'ok', ms: 1, lane, from, head: headAt(at) });
  log.records.push(page(3100, headAt(3100) + 400), page(3300, headAt(3300) + 400), page(3500, headAt(3500) + 400));
  const judged = () => dashboardRows({ ...log, records: [...log.records].sort((a, b) => a.at - b.at) }, { refreshHz: 60 }).rows;
  const ahead = judged().find(row => row.bar === 'list requests ahead of the head');
  assert.equal(ahead.verdict, PASS, ahead.note);
  assert.equal(ahead.values.count, 3);
  assert.deepEqual(judged().filter(row => row.bar === 'requests for values at the head').map(row => row.values.count), [1], 'only the planted request is at the head');

  const again = { ...page(3550, 0), from: log.records.at(-1).from };
  log.records.push(again);
  assert.match(judged().find(row => row.bar === 'list requests ahead of the head').note, /1 asked twice/);
  log.records.pop();

  log.records.push(page(3600, headAt(3600)), page(3650, headAt(3650) - 50));
  const head = judged().find(row => row.bar === 'requests for values at the head');
  assert.equal(head.values.count, 3, 'a block from the head\'s own hour, or behind it, is a request at the head');
  assert.equal(judged().find(row => row.bar === 'list requests ahead of the head').verdict, PASS);
  log.records.splice(-2);

  for (let at = 3700; at < 3760; at += 10) log.records.push(page(at, headAt(at) + 2000 + at));
  const many = judged().find(row => row.bar === 'list requests ahead of the head');
  assert.equal(many.verdict, FAIL, 'blocks far past what the head crossed are asked too soon');
  assert.match(many.note, /land-battles: 9 asked, \d+ allowed/);
});

test('a window of the log judges only the play inside it, and only losses overlapping it mark it partial', () => {
  const log = syntheticLog();
  const { rows } = dashboardRows(log, { refreshHz: 60, window: { from: 1001, to: 2400 } });
  assert.equal(rows.find(row => row.bar === 'webview frame interval').verdict, PASS, 'the late frame at 2.5 s is outside');
  assert.equal(rows.find(row => row.bar === 'requests for values at the head').verdict, PASS);
  const outsideLost = { ...log, records: [...log.records, { t: 'lost', at: 5000, from: 4000, to: 4500, source: 'ui', count: 10 }] };
  assert.equal(dashboardRows(outsideLost, { refreshHz: 60, window: { from: 1001, to: 2400 } }).rows.find(r => r.bar === 'webview frame interval').verdict, PASS);
  const insideLost = { ...log, records: [...log.records, { t: 'lost', at: 2600, from: 2200, to: 2500, source: 'ui', count: 10 }] };
  assert.ok(dashboardRows(insideLost, { refreshHz: 60, window: { from: 1001, to: 2400 } }).rows.every(r => r.verdict === PARTIAL));
});
