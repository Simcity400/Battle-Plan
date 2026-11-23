import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { buildWidgetClassifierExpr, createOutcomeWatcher, measureIdleCost, runThrottled } from './sweepHelpers.mjs';
import { classifyWidgetSnapshot } from './verdicts.mjs';

const source = readFileSync(new URL('./sweep.mjs', import.meta.url), 'utf8');

test('scrub waits for the released timeline target and records a failure without stopping the phase', () => {
  const release = source.indexOf("await app.mouse('mouseReleased', x(.31), line.y);");
  assert.ok(release >= 0, 'the scripted scrub release is present');
  const wait = source.indexOf('scrub target', release);
  assert.ok(wait > release, 'the release is followed by a named target wait');
  assert.match(source.slice(release, wait + 300), /recordFailure\('scrub'/,
    'a failed scrub target wait records a failure instead of throwing out of the phase');
});

test('scroll tries each scrollable area and records a failure without stopping the phase when none moves', () => {
  const wheel = source.indexOf("type: 'mouseWheel'");
  const need = source.lastIndexOf('const point = await s.locate(target);', wheel);
  const before = source.indexOf('const [before, wheel, at] = await app.evaluate', need);
  assert.ok(need >= 0 && before > need, 'the baseline is captured after revealing the target');
  assert.ok(wheel >= 0, 'the scripted wheel is present');
  const wait = source.indexOf('scrolled = await s.until(', wheel), fail = source.indexOf("if (!scrolled) await failInput('scroll'", wait);
  assert.ok(wait > wheel, 'the wheel is followed by a wait on the target');
  assert.ok(fail > wait && /moved under the wheel/.test(source.slice(fail, fail + 200)), 'a scroll no candidate makes records a failure instead of stopping the phase');
});

test('pause verifies that the playback clock stopped after the click', () => {
  const pause = source.indexOf("await mark('pause');");
  assert.ok(pause >= 0, 'the pause action is present');
  const end = source.indexOf("const inputs = await app.evaluate('__e2e.endInputs()')", pause);
  assert.ok(end > pause, 'pause verification precedes input recording');
  assert.match(source.slice(pause, end), /if \(!await s\.until\([^\n]+(?:playing|Pause playback|Play recording|header-play)/i,
    'the pause action waits for a stopped playback state');
});

test('settlement probing records a bounded failure instead of throwing after timeout', () => {
  const settle = source.indexOf('const settleInputs = async () =>');
  const end = source.indexOf("await mark('rewind');", settle);
  assert.ok(settle >= 0 && end > settle, 'the shared settlement helper is present');
  const helper = source.slice(settle, end);
  assert.match(helper, /try\s*\{[\s\S]*app\.evaluate\(unsettled\)[\s\S]*\}\s*catch/,
    'a timed-out settlement probe is converted into evidence');
  assert.match(helper, /settlement probe failed/, 'the probe failure is named');
});

test('pin restore reopens the widget that was pinned and waits for its time controls', () => {
  const pin = source.indexOf("__e2e.setPinWidget(");
  const restore = source.indexOf("if (pinApplied)", pin);
  assert.ok(pin >= 0 && restore > pin, 'the pin action records the widget and later restores it');
  const body = source.slice(restore, source.indexOf('return {', restore));
  assert.match(body, /data-widget[^;]*pinWidget|pinnedConfig/, 'restore targets the widget recorded by the pin action');
  assert.match(body, /await s\.until\([\s\S]*?time[\s\S]*?\)/, 'restore waits for the time controls after opening the menu');
});

test('picture audits are consumed through bounded evaluate chunks', () => {
  const audit = source.indexOf("window.__perf.auditPicturesBatch(64)");
  assert.ok(audit >= 0, 'the playback phase runs the picture audit');
  const start = source.lastIndexOf('while (!auditDone', audit);
  const end = source.indexOf("const summary = await app.evaluate('window.__perf.stop()'", audit);
  assert.ok(start >= 0, 'the picture audit has a bounded loop');
  assert.ok(end > audit, 'the picture audit completes before perf stop');
  assert.match(source.slice(start, end), /while|for\s*\(/, 'the audit result is collected in more than one evaluate when needed');
  assert.match(source.slice(start, end), /auditPicturesBatch\([^)]*\d+/, 'each audit evaluate has a bounded picture count');
});

test('runThrottled keeps the requested rate active during the measurement and restores rate 1 on return or throw', async () => {
  const calls = [];
  let activeRate = 1;
  const throttle = async value => { calls.push(value); activeRate = value; };

  let seenDuringOk = null;
  const result = await runThrottled(throttle, 4, async () => { seenDuringOk = activeRate; return 'ok'; });
  assert.equal(result, 'ok');
  assert.equal(seenDuringOk, 4);
  assert.equal(activeRate, 1);
  assert.deepEqual(calls, [4, 1]);

  calls.length = 0;
  let seenDuringFail = null;
  await assert.rejects(() => runThrottled(throttle, 8, async () => { seenDuringFail = activeRate; throw new Error('boom'); }), /boom/);
  assert.equal(seenDuringFail, 8);
  assert.equal(activeRate, 1);
  assert.deepEqual(calls, [8, 1]);

  calls.length = 0;
  const partialThrottle = async value => {
    calls.push(value);
    activeRate = value;
    if (value !== 1) throw new Error('page eval failed after CDP throttle');
  };
  await assert.rejects(() => runThrottled(partialThrottle, 4, async () => 'never'), /page eval failed/);
  assert.equal(activeRate, 1, 'throttle(1) runs in finally even when throttle(rate) itself rejects');
  assert.deepEqual(calls, [4, 1]);
});

test('createOutcomeWatcher scans once on playing transition, stops polling, and classifies only mutation-touched widgets', () => {
  const widgetA = { nodeType: 1, dataset: { widget: 'a' }, closest: sel => (sel === 'article.dash-widget' ? widgetA : null) };
  const widgetB = { nodeType: 1, dataset: { widget: 'b' }, closest: sel => (sel === 'article.dash-widget' ? widgetB : null) };
  const childOfA = { nodeType: 1, closest: sel => (sel === 'article.dash-widget' ? widgetA : null) };
  const grid = { querySelectorAll: () => [widgetA, widgetB] };
  const playHost = {};
  let playing = false, gridCb = null, playCb = null, playDisconnected = false, rafCalls = 0, pendingRaf = null;
  const classified = [];
  const outcomes = { a: { outcome: 'content', detail: null, notMigrated: false }, b: { outcome: 'content', detail: null, notMigrated: false } };
  const watcher = createOutcomeWatcher({
    grid,
    playHost,
    isPlaying: () => playing,
    classify: a => { classified.push(a.dataset.widget); return outcomes[a.dataset.widget]; },
    observe: (node, _opts, cb) => {
      if (node === grid) { gridCb = cb; return () => { gridCb = null; }; }
      playCb = cb; return () => { playDisconnected = true; playCb = null; };
    },
    requestFrame: cb => { rafCalls++; pendingRaf = cb; return rafCalls; },
    cancelFrame: () => { pendingRaf = null; },
    waitForPlaying: true,
  });
  assert.deepEqual(classified, [], 'no classification before playback starts');
  playing = true;
  playCb();
  assert.deepEqual(classified, ['a', 'b'], 'scans all widgets once on playing transition');
  assert.equal(playDisconnected, true, 'activation observer disconnects once playing');
  assert.equal(pendingRaf, null, 'activation frame polling stops once playing');

  classified.length = 0;
  outcomes.a = { outcome: 'error', detail: 'boom', notMigrated: false };
  gridCb([{ target: childOfA, addedNodes: [] }]);
  assert.deepEqual(classified, ['a'], 'grid mutation classifies only the touched widget, never untouched siblings');

  const result = watcher.end();
  assert.equal(result.a.outcome, 'error');
  assert.equal(result.b.outcome, 'content');
});

test('measureIdleCost returns bounds on success and records a failure without throwing when pause or speed fails', async () => {
  const events = [];
  let clock = 1000;
  const ok = await measureIdleCost({
    pausePlayback: async () => { events.push('pause'); },
    waitPlayButton: async () => { events.push('waitPlay'); },
    setSpeed: async speed => { events.push(`speed:${speed}`); },
    clickPlay: async () => { events.push('play'); },
    now: async () => (clock += 500),
    frames: async n => { events.push(`frames:${n}`); },
    onFailure: err => events.push(`fail:${err.message}`),
  });
  assert.deepEqual(ok, { from: 1500, to: 2000 });
  assert.deepEqual(events, ['pause', 'waitPlay', 'speed:1', 'play', 'frames:120', 'pause']);

  const failures = [];
  let cleanupPauses = 0;
  const failed = await measureIdleCost({
    pausePlayback: async () => { cleanupPauses++; },
    waitPlayButton: async () => {},
    setSpeed: async () => { throw new Error('clock refused 1 h/s'); },
    clickPlay: async () => {},
    now: async () => 0,
    frames: async () => {},
    onFailure: err => failures.push(err.message),
  });
  assert.equal(failed, undefined);
  assert.deepEqual(failures, ['clock refused 1 h/s']);
  assert.equal(cleanupPauses, 2, 'cleanup pause runs after idle failure');
});

test('browser widget classifier expression delegates directly to classifyWidgetSnapshot using explicit data-state', () => {
  const expr = buildWidgetClassifierExpr(classifyWidgetSnapshot);
  const runInPage = new Function('window', `return (${expr});`);
  const nodeWith = (state, text, detail = null) => ({
    getAttribute: name => name === 'data-state' ? state : name === 'data-detail' ? detail : null,
    textContent: text,
  });
  const makeNode = (map, probeState = null) => ({
    querySelector: sel => map[sel] ?? null,
    __probe: probeState,
  });
  const win = { __e2e: { stateOf: a => a.__probe } };
  const classify = runInPage(win);
  const errNode = nodeWith('error', 'Couldn’t load.', 'No table 44 in the schema');
  assert.deepEqual(
    classify(makeNode({ '.widget-state[data-state]:not(.is-section)': errNode, '[data-state="error"], [role="alert"]': errNode })),
    { outcome: 'error', detail: 'No table 44 in the schema', notMigrated: false },
  );
  const unavailNode = nodeWith('unavailable', 'Not in this recording.', 'No table 44 in the schema');
  assert.deepEqual(
    classify(makeNode({ '.widget-state[data-state]:not(.is-section)': unavailNode, '[data-state="unavailable"]': unavailNode })),
    { outcome: 'refusal', detail: 'No table 44 in the schema', notMigrated: false },
  );
  const outsideNode = nodeWith('outside', 'This time is outside the recording.');
  assert.deepEqual(
    classify(makeNode({ '.widget-state[data-state]:not(.is-section)': outsideNode, '[data-state="outside"]': outsideNode })),
    { outcome: 'outside', detail: 'This time is outside the recording.', notMigrated: false },
  );
  const gapNode = nodeWith('not-recorded', 'Not recorded at this time.');
  assert.deepEqual(
    classify(makeNode({ '.widget-state[data-state]:not(.is-section)': gapNode, '[data-state="not-recorded"]': gapNode })),
    { outcome: 'not-recorded', detail: 'Not recorded at this time.', notMigrated: false },
  );
  assert.deepEqual(
    classify(makeNode({}, { state: 'loading', drawing: true })),
    { outcome: 'content', detail: null, notMigrated: false },
  );
  assert.deepEqual(
    classify(makeNode({}, { state: 'loading', drawing: false })),
    { outcome: 'error', detail: 'still loading', notMigrated: false },
  );
});
