import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  QUICK_CATALOGUE_DEFAULT_COUNT,
  buildWidgetClassifierExpr,
  catalogueItemKey,
  createOutcomeWatcher,
  describeUncleanCataloguePage,
  finishCatalogueCard,
  measureIdleCost,
  mergeWidgetOutcomes,
  runThrottled,
  restoreCountHour,
  selectQuickCatalogueCards,
} from './sweepHelpers.mjs';
import { classifyWidgetSnapshot } from './verdicts.mjs';

const source = readFileSync(new URL('./sweep.mjs', import.meta.url), 'utf8');

test('free-running playback is restored to its captured hour before another count scope', async () => {
  for (const stopped of [7, 19]) {
    let hour = stopped;
    const session = { timeline: async () => ({ min: 0, max: 100, value: hour }) };
    await restoreCountHour(3, session, async fraction => { hour = Math.round(fraction * 100); });
    assert.equal(hour, 3);
  }
  await assert.rejects(() => restoreCountHour(3, { timeline: async () => ({ min: 0, max: 100, value: 19 }) }, async () => {}), /unmeasured.*starting hour/);
  const catalogue = source.slice(source.indexOf("await phase('catalogue'"));
  assert.ok(catalogue.indexOf('await seekTo(PICTURE_SEEK[1])') < catalogue.indexOf("await s.step('catalogue'"));
});

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
  const outcomes = { a: { outcome: 'content', detail: null }, b: { outcome: 'content', detail: null } };
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
  outcomes.a = { outcome: 'error', detail: 'boom' };
  gridCb([{ target: childOfA, addedNodes: [] }]);
  assert.deepEqual(classified, ['a'], 'grid mutation classifies only the touched widget, never untouched siblings');

  const result = watcher.end();
  assert.equal(result.a.outcome, 'error');
  assert.equal(result.b.outcome, 'content');
});

test('playback outcome watcher distinguishes recovered loading, persistent loading, held pictures and failure', () => {
  assert.deepEqual(classifyWidgetSnapshot({ probeState: 'loading' }, true), { outcome: 'loading', detail: 'still loading' });
  assert.deepEqual(classifyWidgetSnapshot({ domState: 'loading', errorDetail: 'section failed' }, true), { outcome: 'error', detail: 'section failed' });
  assert.deepEqual(classifyWidgetSnapshot({ domState: 'loading', unavailableText: 'refused' }, true), { outcome: 'refusal', detail: 'refused' });
  const widget = { dataset: { widget: 'spending' } };
  const grid = { querySelectorAll: () => [widget] };
  const observation = { selector: '.fh-overlay div.widget-state', text: 'Loading spending…', bodyStatus: 'loading', metaMode: 'picture', installedBinding: 12 };
  const loading = { frames: 2, firstHour: 100, lastHour: 148, observation };
  const held = { selector: 'div.fh-frame', bodyStatus: 'ready', metaMode: 'picture', installedBinding: 12 };
  for (const final of ['content', 'loading', 'error', 'later-loading']) {
    let state = 'loading';
    let hour = 100;
    let raf, changed;
    const watcher = createOutcomeWatcher({
      grid,
      isPlaying: () => true,
      readHour: () => hour,
      classify: (_widget, tracking) => {
        assert.equal(tracking, true);
        return { outcome: state, detail: state === 'error' ? 'actual failure' : null,
          ...(state === 'loading' ? { observation: { ...observation, installedBinding: hour === 100 ? 12 : 13 } } : {}) };
      },
      observe: (_node, _options, fn) => { changed = fn; return () => {}; },
      requestFrame: fn => { raf = fn; return 1; },
      cancelFrame: () => { raf = undefined; },
    });
    raf();
    raf();
    hour = 148;
    state = final === 'later-loading' ? 'content' : final;
    const mutation = [{ target: { nodeType: 1, closest: () => widget } }];
    changed(mutation);
    if (final === 'error') {
      state = 'content';
      changed(mutation);
    }
    if (final === 'later-loading') {
      hour = 170;
      state = 'loading';
      changed(mutation);
    }
    const outcome = watcher.end().spending;
    if (final === 'content') {
      assert.equal(outcome.outcome, 'loading-recovered');
      assert.deepEqual(outcome.loading, loading);
      assert.match(outcome.detail, /2 frames.*48 game hours/);
      assert.deepEqual(mergeWidgetOutcomes([{ id: 'spending', outcome: 'content' }], { spending: outcome })[0].loading, outcome.loading);
    } else if (final === 'loading' || final === 'later-loading') {
      assert.deepEqual(outcome, { outcome: 'error', detail: 'still loading', loading });
      assert.deepEqual(mergeWidgetOutcomes([{ id: 'spending', outcome: 'error' }], { spending: outcome })[0].loading, loading);
    } else {
      assert.deepEqual(outcome, { outcome: 'error', detail: 'actual failure', loading });
      const snapshot = { id: 'spending', outcome: 'loading', detail: 'loading at the end' };
      assert.deepEqual(mergeWidgetOutcomes([snapshot], { spending: outcome })[0], { id: 'spending', ...outcome });
    }
    assert.equal(raf, undefined, 'the watcher cancels its animation frame');
  }
  let hour = 100;
  const watcher = createOutcomeWatcher({
    grid, isPlaying: () => true, readHour: () => hour,
    classify: () => hour === 100 ? { outcome: 'content', detail: null, held } : { outcome: 'content', detail: null },
    observe: () => () => {},
  });
  hour = 148;
  const outcome = watcher.end().spending;
  assert.deepEqual(outcome, { outcome: 'content', detail: null, held: { hour: 100, ...held } });
  assert.deepEqual(mergeWidgetOutcomes([{ id: 'spending', outcome: 'content' }], { spending: outcome })[0].held, outcome.held);
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
  const win = { __e2e: { stateOf: a => a.__probe }, CSS: { escape: name => name } };
  const classify = runInPage(win);
  const errNode = nodeWith('error', 'Couldn’t load.', 'No table 44 in the schema');
  assert.deepEqual(
    classify(makeNode({ '.widget-state[data-state]:not(.is-section)': errNode, '[data-state="error"], [role="alert"]': errNode })),
    { outcome: 'error', detail: 'No table 44 in the schema' },
  );
  const unavailNode = nodeWith('unavailable', 'Not in this recording.', 'No table 44 in the schema');
  assert.deepEqual(
    classify(makeNode({ '.widget-state[data-state]:not(.is-section)': unavailNode, '[data-state="unavailable"]': unavailNode })),
    { outcome: 'refusal', detail: 'No table 44 in the schema' },
  );
  const outsideNode = nodeWith('outside', 'This time is outside the recording.');
  assert.deepEqual(
    classify(makeNode({ '.widget-state[data-state]:not(.is-section)': outsideNode, '[data-state="outside"]': outsideNode })),
    { outcome: 'outside', detail: 'This time is outside the recording.' },
  );
  const gapNode = nodeWith('not-recorded', 'Not recorded at this time.');
  assert.deepEqual(
    classify(makeNode({ '.widget-state[data-state]:not(.is-section)': gapNode, '[data-state="not-recorded"]': gapNode })),
    { outcome: 'not-recorded', detail: 'Not recorded at this time.' },
  );
  assert.deepEqual(
    classify(makeNode({}, { state: 'loading', drawing: true })),
    { outcome: 'content', detail: null },
  );
  assert.deepEqual(
    classify(makeNode({}, { state: 'loading', drawing: false })),
    { outcome: 'error', detail: 'still loading' },
  );
  const frame = { playbackDiagnostics: { bodyStatus: 'loading', metaMode: 'picture', installedBinding: 12 } };
  const element = { ...nodeWith('loading', 'Loading spending…'), localName: 'div', classList: ['widget-state'],
    closest: selector => selector === '.fh-frame' ? frame : {} };
  const observed = classify(makeNode({}, { state: 'loading', element }), true);
  assert.deepEqual(observed.observation, {
    source: 'probe', selector: '.fh-overlay div.widget-state', text: 'Loading spending…', domState: null,
    bodyStatus: 'loading', metaMode: 'picture', installedBinding: 12,
  });
  assert.deepEqual(classify(makeNode({}, { state: 'content', held: {
    ...element, classList: ['fh-frame'], textContent: 'Recorded value',
    closest: selector => selector === '.fh-frame' ? frame : null,
  } }), true), {
    outcome: 'content', detail: null,
    held: { source: 'held-picture', selector: 'div.fh-frame', text: 'Recorded value', domState: null,
      bodyStatus: 'loading', metaMode: 'picture', installedBinding: 12 },
  });
});

test('playback unsettled probe ignores hidden screen-reader aria-busy mirrors while flagging visible loading widgets', () => {
  const start = source.indexOf('const unsettled = `(() => {');
  const end = source.indexOf('})()`;', start);
  const expr = source.slice(start + 'const unsettled = `'.length, end + '})()'.length).replace(/\\\\/g, '\\');
  const makeNode = ({ tag = 'div', classes = [], attrs = {}, rect = { top: 10, bottom: 50, left: 10, right: 110, width: 100 }, style = { overflowY: 'visible' }, children = [] } = {}) => {
    const matchOne = p => (p === '[data-state="loading"]' ? attrs['data-state'] === 'loading'
      : p === '[aria-busy="true"]' ? attrs['aria-busy'] === 'true'
      : p === '[data-placement]' ? Object.hasOwn(attrs, 'data-placement')
      : p === 'article.dash-widget' ? tag === 'article' && classes.includes('dash-widget')
      : p.startsWith('.') ? classes.includes(p.slice(1)) : false);
    const node = {
      classList: { contains: name => classes.includes(name) }, textContent: '', dataset: { widget: attrs['data-widget'] ?? '' },
      parentElement: null, __style: style, __children: [], children: [],
      checkVisibility: () => true, getBoundingClientRect: () => rect,
      getAttribute: name => (Object.hasOwn(attrs, name) ? attrs[name] : null),
      matches: sel => sel.split(',').some(p => matchOne(p.trim())),
      closest(sel) { for (let c = this; c; c = c.parentElement) if (c.matches(sel)) return c; return null; },
      querySelector(sel) { return this.querySelectorAll(sel)[0] ?? null; },
      querySelectorAll(sel) { const o = []; const w = e => { for (const c of e.__children) { if (c.matches(sel)) o.push(c); w(c); } }; w(this); return o; },
    };
    for (const c of children) { c.parentElement = node; node.__children.push(c); node.children.push(c); }
    return node;
  };
  const srMirror = makeNode({
    tag: 'table', classes: ['fh-sr-only'], attrs: { 'aria-busy': 'true' },
    rect: { top: 10, bottom: 11, left: 10, right: 11, width: 1 },
    style: { position: 'absolute', overflowY: 'hidden', clip: 'rect(0px, 0px, 0px, 0px)', clipPath: 'inset(50%)' },
  });
  const hiddenAncestor = makeNode({
    classes: ['sr-only'], rect: { top: 10, bottom: 11, left: 10, right: 11, width: 1 },
    style: { position: 'absolute', overflowY: 'hidden', clip: 'rect(0px, 0px, 0px, 0px)' },
    children: [makeNode({ tag: 'span', attrs: { 'aria-busy': 'true' } })],
  });
  const okWidget = makeNode({ tag: 'article', classes: ['dash-widget'], attrs: { 'aria-label': 'Occupied states', 'data-widget': 'w1' }, children: [makeNode({ tag: 'canvas', attrs: { 'data-placement': '1' } }), srMirror, hiddenAncestor] });
  const stuckWidget = makeNode({ tag: 'article', classes: ['dash-widget'], attrs: { 'aria-label': 'Fuel', 'data-widget': 'w2' }, children: [makeNode({ tag: 'canvas', attrs: { 'data-placement': '2' } }), makeNode({ attrs: { 'data-state': 'loading' } })] });
  const grid = makeNode({ classes: ['dash-grid-scroll'], rect: { top: 0, bottom: 720, left: 0, right: 1280, width: 1280 }, children: [okWidget, stuckWidget] });
  const runUnsettled = new Function('document', 'getComputedStyle', '__e2e', `return (${expr});`);
  const probeStub = { visuallyHidden: el => el.classList.contains('fh-sr-only') || el.classList.contains('sr-only') };
  assert.deepEqual(
    runUnsettled({ querySelector: sel => (sel === '.dash-grid-scroll' ? grid : null) }, el => el?.__style ?? { overflowY: 'visible' }, probeStub),
    [{ widget: 'Fuel', reason: 'loading' }],
  );
});

test('selectQuickCatalogueCards uses matching history slowest cards and falls back to bounded indices on fresh state', () => {
  const cards = [
    { title: 'Campaign map', view: false, unavailable: null },
    { title: 'Scoreboard', view: false, unavailable: null },
    { title: 'Construction log', view: false, unavailable: 'Availability unavailable.' },
    { title: 'Game score', view: false, unavailable: null },
    { title: 'Game score race', view: true, unavailable: null },
    { title: 'Event feed', view: false, unavailable: null },
    { title: 'Duplicate mod idea', view: false, unavailable: null },
    { title: 'Duplicate mod idea', view: false, unavailable: null },
    { title: 'Political power', view: false, unavailable: null },
  ];

  const fromFresh = selectQuickCatalogueCards(cards, null);
  assert.equal(fromFresh.source, 'default');
  assert.equal(fromFresh.indices.size, QUICK_CATALOGUE_DEFAULT_COUNT);
  assert.equal(fromFresh.indices.has(0), true);
  assert.equal(fromFresh.indices.has(8), true);
  assert.equal(fromFresh.indices.has(2), false, 'unavailable cards skipped when available cards exist');

  const fromEmptyHistory = selectQuickCatalogueCards(cards, { format: 2, failed: {}, slowest: [] });
  assert.deepEqual([...fromEmptyHistory.indices], [...fromFresh.indices]);
  assert.equal(fromEmptyHistory.source, 'default');

  const fromStaleHistory = selectQuickCatalogueCards(cards, { format: 2, failed: {}, slowest: ['widget/Removed card'] });
  assert.deepEqual([...fromStaleHistory.indices], [...fromFresh.indices]);
  assert.equal(fromStaleHistory.source, 'default');

  const fromHistory = selectQuickCatalogueCards(cards, {
    format: 2,
    failed: {},
    slowest: ['view/Game score race', 'widget/Removed card', 'widget/Event feed'],
  });
  assert.equal(fromHistory.source, 'history');
  assert.deepEqual([...fromHistory.indices].map(i => catalogueItemKey(cards[i])), ['view/Game score race', 'widget/Event feed']);

  const allUnavailable = cards.slice(0, 3).map(card => ({ ...card, unavailable: 'not recorded' }));
  const fallbackUnavailable = selectQuickCatalogueCards(allUnavailable, null, 2);
  assert.equal(fallbackUnavailable.source, 'default');
  assert.deepEqual([...fallbackUnavailable.indices], [0, 2]);
});

test('describeUncleanCataloguePage names leftover widgets, dirty layouts, and open popovers', () => {
  assert.equal(describeUncleanCataloguePage({ widgets: 0, dirty: false, popovers: [] }), null);
  assert.match(describeUncleanCataloguePage({ widgets: 1, dirty: false, popovers: [] }), /1 widget left on page/);
  assert.match(describeUncleanCataloguePage({ widgets: 2, dirty: true, popovers: ['layout-save-menu'] }), /2 widgets left on page; layout still has unsaved changes; popover stayed open \(layout-save-menu\)/);
});

test('finishCatalogueCard fails the card by name when cleanup throws, recovers if unclean, and lets the next card run', async () => {
  const failures = [];
  const ran = [];
  let state = { widgets: 0, dirty: false, popovers: [] };
  let recoveries = 0;

  await finishCatalogueCard('widget/Broken add', {
    cleanup: async () => { throw new Error('the Delete widget button did not become visible within 3 s'); },
    readState: async () => state,
    recover: async () => { recoveries++; },
    fail: async (item, label, reason) => { failures.push({ item, label, reason }); },
  });
  assert.deepEqual(failures, [{ item: 'widget/Broken add', label: 'cleanup', reason: 'the Delete widget button did not become visible within 3 s' }]);
  assert.equal(recoveries, 0, 'clean page after thrown cleanup needs no recovery');

  state = { widgets: 1, dirty: true, popovers: ['layout-save-menu'] };
  await finishCatalogueCard('widget/Campaign map', {
    cleanup: async () => { throw new Error('the Save layout confirm button did not become visible within 3 s'); },
    readState: async () => state,
    recover: async () => { recoveries++; state = { widgets: 0, dirty: false, popovers: [] }; },
    fail: async (item, label, reason) => { failures.push({ item, label, reason }); },
  });
  await finishCatalogueCard('widget/Next card', {
    cleanup: async () => { ran.push('widget/Next card'); },
    readState: async () => state,
    recover: async () => { recoveries++; },
    fail: async (item, label, reason) => { failures.push({ item, label, reason }); },
  });
  assert.equal(recoveries, 1);
  assert.deepEqual(ran, ['widget/Next card'], 'second card runs after first card cleanup threw and recovered');
  assert.equal(failures.length, 2);

  state = { widgets: 1, dirty: false, popovers: [] };
  await assert.rejects(() => finishCatalogueCard('widget/Stuck', {
    cleanup: async () => {},
    readState: async () => state,
    recover: async () => {},
    fail: async (item, label, reason) => { failures.push({ item, label, reason }); },
  }), /cannot continue catalogue after widget\/Stuck: page not clean after card: 1 widget left on page/);
});
