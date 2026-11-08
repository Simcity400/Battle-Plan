import assert from 'node:assert/strict';
import test from 'node:test';
import { shownHours, catalogueHour, droppedFrames, failedReplies, firstContent, framesLate, freshness, freshnessByKind, item, losses, mapFrames, mapSeek, preparation, presentedAt, quantiles, UNPRESENTED } from './verdicts.mjs';

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
    { t: 'tick', at: 0, hour: 100, speed: 24 },
    { t: 'tick', at: 250, hour: 106, speed: 24 },
    { t: 'shown', at: 0, widget: 'w', hour: 100 },
    { t: 'shown', at: 300, widget: 'w', hour: 106 },
    ...[10, 260, 310, 320].map(at => ({ t: 'frame', at, playing: true })),
  ];
  const result = freshness(records, 0, 1000);
  assert.equal(result.step, 6);
  assert.deepEqual(result.widgets.map(w => [w.title, w.frames, w.p95, w.pass]), [['Widget', 4, 6, true]]);
});

test('preparation is judged against the median of the latest passing runs, beyond each margin', () => {
  const prep = { state: 'ready', error: null, failedSections: [], exitCode: 0, wallSeconds: 100, cpuSeconds: 1000, peakCommitGb: 8 };
  const first = preparation(prep, []);
  assert.equal(first.pass, true);
  assert.equal(first.metrics[0].baseline, null);
  const history = [90, 400, 80, 100, 95, 85].map((wallSeconds, index) => ({ wallSeconds, cpuSeconds: 1000, peakCommitGb: 8, pass: index !== 1 }));
  assert.equal(preparation(prep, history).metrics[0].baseline, 90);
  const slow = preparation({ ...prep, wallSeconds: 120 }, history);
  assert.equal(slow.pass, false);
  assert.match(slow.reasons[0], /wallSeconds 120.00 over 112.50/);
  assert.equal(preparation({ ...prep, state: 'failed', error: 'disk' }, history).reasons[0], 'ended failed: disk');
});

test('a dropped frame is an interval over 1.5 refreshes, and three in a row fail as stutter', () => {
  const frames = dts => dts.map((dt, index) => ({ t: 'frame', at: index * 10, dt, playing: true }));
  const scattered = droppedFrames(frames([10.6, 22, 10.6, 22, 10.6, 22, 10.6]), 0, 1000, 94);
  assert.deepEqual([scattered.dropped, scattered.longest, scattered.pass], [3, 1, true]);
  assert.equal(scattered.rate, 3 / 7);
  const sustained = droppedFrames(frames([10.6, 22, 21.3, 32, 10.6]), 0, 1000, 94);
  assert.deepEqual([sustained.longest, sustained.pass], [3, false]);
  assert.equal(droppedFrames([], 0, 1000, 94).pass, false);
});

test('the map is judged at 120 Hz on its main process frames, and a highlight after a whole later frame is late', () => {
  const records = [
    { t: 'map_open', pid: 1, role: 'main' }, { t: 'map_open', pid: 2, role: 'popout' },
    { t: 'map', pid: 1, frame: 1, at: 110, work_ms: 4, dt: 10.5 }, { t: 'map', pid: 1, frame: 2, at: 121, work_ms: 6.5, dt: 10.5, acquire_ms: 0, painted_at: 110.25 },
    { t: 'map', pid: 2, frame: 1, at: 115, work_ms: 30, dt: 40 },
    // GPU times land a few frames later, naming the frame they measured.
    { t: 'map', pid: 1, frame: 3, at: 140, work_ms: 4, dt: 10.5, acquire_ms: 0, painted_at: 121.25, gpu_frame: 1, gpu_ms: 3 },
    { t: 'map', pid: 1, frame: 4, at: 151, work_ms: 4, dt: 10.5, acquire_ms: 0, painted_at: 140.25, gpu_frame: 2, gpu_ms: 3.5 },
  ];
  const fine = mapFrames(records, 100, 130);
  assert.deepEqual([fine.frames, fine.dropped, fine.pass], [2, 0, true]);
  assert.equal(fine.gpu.max, 3.5);
  // A slower frame between them: its paint ended half a millisecond after its interface pass.
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
  // Out of view while playing it is not judged; paused in view it must report.
  assert.match(kinds[0].reasons[0], /does not report the hour it shows \(Overview: Feed\)/);
});

test('a reply cut off by the suite closing its session is not a failure, and nothing else is excused', () => {
  const records = [
    { t: 'q', product: 'a', at: 100, ms: 5, outcome: 'error', failure: 'refused: no' },
    { t: 'q', product: 'b', at: 190, ms: 50, outcome: 'error', failure: 'failed: The dashboard data connection closed' },
    { t: 'q', product: 'c', at: 190, ms: 50, outcome: 'error', failure: 'failed: the reader crashed' },
    { t: 'q', product: 'd', at: 120, ms: 5, outcome: 'error', failure: 'failed: The dashboard data connection closed' },
  ];
  assert.deepEqual(failedReplies(records, [{ from: 0, to: 200 }]).map(reply => reply.product), ['a', 'c', 'd']);
  assert.equal(failedReplies(records).length, 4);
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
    // The next record carries frame 1's paint: it ended 8 ms after the interface pass, 3 of them waiting for an image.
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
