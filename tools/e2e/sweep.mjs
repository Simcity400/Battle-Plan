// The acceptance sweep over one running app with a recording open: every page cold and warm,
// seeks on every page, pictures at a fixed hour, the map, playback, every catalogue view and every control case.
// usage: node tools/e2e/sweep.mjs --port <port> --out <dir> --work <dir> --session <name> --desktop <name> --app-pid <pid> --input <address>
//   [--phases pages,seeks,visual,map,mapfps,playback,catalogue,controls,pages-at,glance] [--cases id,...] [--pages page,...] [--pages-at <fraction>]
// `--cases` names the control cases to run, and in the map phases the map interactions to run, the rest of the map
// phase (modes, hover, the pop-out) being left out then.
//   [--tier full|quick] [--history <file>] [--pin all|product] [--record-replies <file.jsonl.gz>] [--app-revision <commit>]
// One app session: its results go to sweep-<session>.json, its pictures to visual/manifest-<session>.json. The quick tier
// glances at every page at mid-recording, plays its heaviest pages at 1,440 h/s under the trace, adds the catalogue's
// slowest cards and changes one setting of each widget kind. `--history` names the cases that ever failed (`coverage.mjs`);
// `--pin product` lets only the product's failures keep a case from being left out as a repeat. `--record-replies` records
// every reply and followed message the page receives, for tests on real replies (`replies.mjs`).
import { existsSync } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { gzip } from 'node:zlib';
import { connect, log, sleep } from './cdp.mjs';
import { Session, settleCall } from './session.mjs';
import { CASES } from './controls.mjs';
import { PINS, readHistory } from './coverage.mjs';
import { Visual } from './visual.mjs';
import { NativeInput, hover, interactions, mapModes, mapPictures, zoomedMapPicture } from './map.mjs';
import { pageShown } from './verdicts.mjs';
import { TRACE_MARKS, describeNodes, startTrace, summariseTrace, summaryNodes } from './trace.mjs';
import { ReplyRecorder } from './replies.mjs';

const flag = (name, fallback = null) => { const at = process.argv.indexOf(name); return at < 0 ? fallback : process.argv[at + 1]; };
const port = Number(flag('--port'));
const out = flag('--out');
const work = flag('--work');
const desktop = flag('--desktop');
const appPid = Number(flag('--app-pid'));
const session = flag('--session');
// The viewer's suite input channel (BATTLE_PLAN_E2E_INPUT), for pointer and key input to the native map.
const input = flag('--input');
if (!port || !out || !work || !desktop || !appPid || !session || !input) throw new Error('usage: sweep.mjs --port <port> --out <dir> --work <dir> --session <name> --desktop <name> --app-pid <pid> --input <address>');
export const PHASES = ['pages', 'seeks', 'visual', 'map', 'mapfps', 'playback', 'catalogue', 'controls'];
export const QUICK_PHASES = ['glance', 'playback', 'catalogue', 'controls'];
const tier = flag('--tier', 'full');
if (tier !== 'full' && tier !== 'quick') throw new Error(`--tier takes full or quick, not ${tier}`);
const quick = tier === 'quick';
const phases = flag('--phases')?.split(',') ?? (quick ? QUICK_PHASES : PHASES);
const historyFile = flag('--history');
const history = historyFile && existsSync(historyFile) ? readHistory(JSON.parse(await readFile(historyFile, 'utf8')), historyFile) : null;
if (!history) log('no failure history: every case runs, none is left out as a repeat');
const pin = flag('--pin', 'all');
if (!PINS.includes(pin)) throw new Error(`--pin takes ${PINS.join(' or ')}, not ${pin}`);
/** `--cases` names the control cases to run; in the map phases, the map interactions (`MAP_INTERACTIONS` ids),
 * whose modes, hover and pop-out are left out then, so one interaction can be measured on its own. */
const only = flag('--cases')?.split(',') ?? null;
/** The pages a control case visits (`--pages`): all by default, so a long controls phase can run in parts. */
const onlyPages = flag('--pages')?.split(',') ?? null;
/** The share of the span the `pages-at` phase visits every page at. */
const pagesAt = flag('--pages-at') === null ? null : Number(flag('--pages-at'));
const recordReplies = flag('--record-replies');
if (pagesAt !== null && !(pagesAt >= 0 && pagesAt <= 1)) throw new Error(`--pages-at takes a share of the timeline from 0 to 1, not ${flag('--pages-at')}`);
/** Early, mid-war and late: fixed shares of the fixture's span. */
export const SEEKS = [['early', 0.05], ['mid-war', 0.5], ['late', 0.95]];
/** How far apart each page's seek hours are, as a share of the span: no two pages seek the same hour. */
const SEEK_STEP = 0.004;
/** The hour pictures are taken at: mid-war. */
const PICTURE_SEEK = SEEKS[1];
/** Playback speeds measured, in game hours per second (presets of the speed menu). */
export const SPEEDS = [24, 1440];
/** The quick tier plays the heaviest pages at the fastest speed. */
const QUICK_PLAYBACK = { pages: ['Politics', 'Overview', 'Army'], speeds: [1440] };
/** The quick tier glances at every page at this share of the span. */
const GLANCE_AT = 0.5;
const PRESETS = [0.25, 1, 4, 24, 168, 720, 1440];
const PLAY_SECONDS = Number((process.env.BATTLE_PLAN_PLAY_SECONDS ?? process.env.HOI4_PLAY_SECONDS) ?? 8);
const rate = Number(flag('--cpu-throttle', '1'));
if (!(rate >= 1 && Number.isFinite(rate) && PLAY_SECONDS > 0 && Number.isFinite(PLAY_SECONDS))) throw new Error('Invalid playback duration or throttle');
/** One window a page is played again under the browser's trace, apart from the timed windows the tracing would slow. */
const TRACE_SPEED = 1440;
const TRACE_SECONDS = 4;
/** The page catalogue views are added to, emptied first. */
const CATALOGUE_PAGE = 'Intelligence';

const app = await connect(port);
// Before the recording opens, so its source and every reply after it are recorded.
const replies = recordReplies ? await ReplyRecorder.start(app, recordReplies,
  { pid: appPid, revision: flag('--app-revision'), root: path.resolve(import.meta.dirname, '../..') }) : null;
// From the start, so the map's first states and mode lists are seen.
await app.evaluate('__e2e.listenMaps()');
const throttle = async value => {
  if (rate === 1) return;
  await app.send('Emulation.setCPUThrottlingRate', { rate: value });
  await app.evaluate(`(window.__weakCpu = ${value}, window.dispatchEvent(new Event('weakcpu')), true)`);
};
const s = new Session(app, out, session);
await s.init();
s.history = history;
s.tier = tier;
s.pin = pin;
const visual = new Visual(app, out, session);
await visual.init();
const started = new Date();
const speeds = quick ? QUICK_PLAYBACK.speeds : SPEEDS;
const meta = { session, tier, requestedPages: onlyPages, cpuThrottle: rate, started: started.toISOString(), port, phases, epoch: s.epoch, pages: [], viewport: null, cataloguePage: CATALOGUE_PAGE, speeds, seeks: SEEKS, pictureSeek: PICTURE_SEEK };
const save = async () => {
  await replies?.take();
  if (phases.includes('seeks')) meta.seekInputs = await app.evaluate('__e2e.seekInputs ?? []');
  const results = s.results.map(({ probe, ...result }) => ({ ...result, session }));
  await writeFile(path.join(out, `sweep-${session}.json`), JSON.stringify({ meta, results, console: app.messages.map(message => ({ ...message, session })) }, null, 1));
  await visual.finish({ pictureSeek: PICTURE_SEEK, viewport: meta.viewport });
};
/** Recording replies, each step waits until nothing is in flight however long it takes, so the recording holds every
 * reply the page asks there; a step timed out by the suite's bound would leave the rest unasked. */
const settledForReplies = async () => {
  if (!replies) return;
  // Requests and the deliberate waits before one: what can still bring a reply. (Data updates are left out: on the
  // fed926180f app a paused page queues one every few frames and never holds still.)
  const quiet = await app.evaluate(`new Promise(resolve => {
    const until = performance.now() + 60000, kinds = ['requests', 'waits'];
    let since = null;
    const check = () => {
      const held = window.__busy?.kinds() ?? {};
      if (kinds.some(kind => held[kind] > 0)) since = null; else since ??= performance.now();
      if (since !== null && performance.now() - since >= 1000) resolve(true); else if (performance.now() > until) resolve(false); else setTimeout(check, 50);
    };
    check();
  })`, 90_000);
  if (!quiet) log('recording replies: the page still asked after 60 s; moving on');
};
/** Seeks to `fraction` without measuring it, and waits for the page to settle. */
const seekTo = async fraction => {
  const line = await s.timeline();
  await app.click(Session.timelineX(line, fraction), line.y);
  await app.evaluate(`(__e2e.mark(null), ${settleCall(15000, 300)})`, 60_000);
};
/** Chooses `speed` from the speed menu and proves the clock took it (the trigger's tooltip names the speed in force): a
 * window measured at another speed than its label is worthless (run 19: windows labelled 1,440 h/s played at 24). A
 * choice the app did not take is retried once, then fails by name. */
const speedTaken = speed => `document.querySelector('.header-speed')?.dataset.tooltip === ${JSON.stringify(`${speed} game ${speed === 1 ? 'hour' : 'hours'} per second`)}`;
const setSpeed = async speed => {
  for (let attempt = 1; attempt <= 2; attempt++) {
    await s.click('.header-speed');
    await s.until(`document.querySelector('.playback-speed-menu:popover-open .playback-speed-presets button')`, 3000);
    await s.click('.playback-speed-menu:popover-open .playback-speed-presets button', { index: PRESETS.indexOf(speed) });
    await s.until(`!document.querySelector('.playback-speed-menu:popover-open')`, 3000);
    if (await s.holds(speedTaken(speed))) return;
    log('speed', `the speed menu's ${speed} h/s was clicked but the clock did not take it (attempt ${attempt})`);
  }
  throw new Error(`the clock never took ${speed} h/s from the speed menu`);
};

// The fixture is the one recording in the fresh profile's library, on its shelf of recordings not yet loaded:
// the shelf opened and the row clicked, as a user does.
app.setCase('startup');
const loadRow = '.sidebar-menu button.sidebar-row[aria-label^="Load "]';
const shelf = 'button.sidebar-shelf-toggle[aria-expanded="false"]';
for (let waited = 0; !await app.evaluate(`document.querySelectorAll(${JSON.stringify(loadRow)}).length > 0`).catch(() => false); waited++) {
  if (waited >= 120) throw new Error(`the library never listed the fixture: ${(await app.evaluate('document.body.innerText').catch(() => '')).slice(0, 400)}`);
  if (await app.evaluate(`!!document.querySelector(${JSON.stringify(shelf)})`).catch(() => false)) await s.click(shelf);
  await sleep(500);
}
const rows = await app.evaluate(`[...document.querySelectorAll(${JSON.stringify(loadRow)})].map(b => b.getAttribute('aria-label'))`);
if (rows.length !== 1) throw new Error(`the fresh profile's library lists ${rows.length} recordings, not the fixture alone: ${rows.join(', ')}`);
await s.step('startup', 'open campaign', rows[0], () => s.click(loadRow),
  { done: `document.querySelector('input.header-timeline:not([disabled])') && document.querySelectorAll('.sidebar-page-list button.sidebar-subrow').length > 0`, timeout: 120_000, quiet: 500 });
if (!await s.holds(`document.querySelector('input.header-timeline:not([disabled])')`))
  throw new Error(`the campaign never opened: ${(await app.evaluate('document.body.innerText').catch(() => '')).slice(0, 400)}`);
meta.pages = await s.pages();
meta.viewport = await app.evaluate(`({ width: innerWidth, height: innerHeight, dpr: devicePixelRatio })`);
meta.startPage = await s.currentPage();
log('pages', meta.pages.join(', '), 'viewport', JSON.stringify(meta.viewport), 'start', meta.startPage);
// The page shown as the recording opened, at its opening hour: where a fixture replaying the run begins.
if (replies) {
  await s.until(`document.querySelector('.dash-grid article.dash-widget')`, 30_000);
  await settledForReplies();
  await replies.mark('start', { page: meta.startPage ?? meta.pages[0] });
}

async function phase(name, run) {
  if (!phases.includes(name)) return;
  log('phase', name);
  const began = Date.now();
  try { await run(); } catch (error) { await s.fail(name, 'phase', null, `stopped: ${error.stack ?? error.message}`); }
  meta[`${name}Seconds`] = (Date.now() - began) / 1000;
  await s.escape().catch(() => {});
  await save();
}

/** A page visit: the switch, then a wheel turn at a time to the bottom; each step measures the widgets it showed and
 * saves its screenshot, passed or not, so every widget on the page is seen. `area` is the phase it is recorded under;
 * `extra` goes on every step's result. */
async function visit(pass, page, { area = 'pages', extra = {} } = {}) {
  if (meta.startPage === page && pass === 'cold')
    await s.fail(area, `cold/${page}`, 'switch', 'shown at startup before any input: its cold visit cannot be measured from input');
  const check = pass !== 'warm';
  // The switch is done once the page it clicked is the one shown, and its widgets have settled (names landed included).
  await s.step(area, `${pass}/${page}`, 'switch', () => s.click('.sidebar-page-list button.sidebar-subrow', { text: page }),
    { done: pageShown(page), timeout: 8000, quiet: 300, check, shot: true, extra });
  // A click the app never took fails the switch by name; the page is then opened unmeasured, so its widgets are still seen.
  if (!await s.holds(pageShown(page))) await s.openPage(page, { force: true });
  await settledForReplies();
  await replies?.mark('page', { page, pass, area });
  for (let turn = 1; ; turn++) {
    const grid = await app.evaluate(`(({ scrollTop, clientHeight, scrollHeight }) => ({ scrollTop, clientHeight, scrollHeight }))(document.querySelector('.dash-grid-scroll'))`);
    if (grid.scrollTop + grid.clientHeight >= grid.scrollHeight - 1) break;
    if (turn > 30) { await s.fail(area, `${pass}/${page}`, 'scroll', 'still not at the bottom after 30 wheel turns'); break; }
    await s.step(area, `${pass}/${page}`, `scroll ${turn}`, () => s.wheel(Math.round(grid.clientHeight * 0.9)), { timeout: 8000, quiet: 300, check, shot: true, extra });
    await settledForReplies();
  }
  await app.evaluate(`document.querySelector('.dash-grid-scroll').scrollTo({ top: 0 })`);
}

await phase('pages', async () => {
  for (const pass of ['cold', 'warm']) for (const page of meta.pages) { await visit(pass, page); await save(); }
});

// Every page visited and scrolled as the page pass does, at one chosen hour: each its first visit at that hour.
await phase('pages-at', async () => {
  if (pagesAt === null) throw new Error('no --pages-at: the hour to visit the pages at was not given');
  meta.pagesAt = { fraction: pagesAt, hours: [] };
  for (const page of meta.pages) {
    // The switch is measured, so it starts from another page.
    if (await s.currentPage() === page) {
      const other = meta.pages.find(name => name !== page);
      if (other) await s.openPage(other);
    }
    await seekTo(pagesAt);
    const hour = (await s.timeline()).value;
    meta.pagesAt.hours.push(hour);
    await visit('at', page, { area: 'pages-at', extra: { fraction: pagesAt, hour } });
    await save();
  }
});

// The quick tier's look at every page: one seek to mid-recording, then each page's first visit there, scrolled to its
// bottom with a screenshot a step, recorded as the pages-at pass is.
await phase('glance', async () => {
  meta.pagesAt = { fraction: GLANCE_AT, hours: [] };
  await seekTo(GLANCE_AT);
  const hour = (await s.timeline()).value;
  meta.pagesAt.hours.push(hour);
  const first = meta.pages.indexOf(await s.currentPage());
  // The page shown at the seek is visited last, so its switch is measured from another page.
  const order = first < 0 ? meta.pages : [...meta.pages.slice(first + 1), ...meta.pages.slice(0, first + 1)];
  for (const page of order) { await visit('at', page, { area: 'pages-at', extra: { fraction: GLANCE_AT, hour } }); await save(); }
});

await phase('seeks', async () => {
  await app.evaluate(`(() => {
    const inputs = __e2e.seekInputs = [];
    for (const type of ['pointerdown', 'keydown', 'wheel']) addEventListener(type, event => {
      if (event.isTrusted) inputs.push(performance.timeOrigin + event.timeStamp);
    }, { capture: true, passive: true });
  })()`);
  for (const [index, page] of meta.pages.entries()) {
    await s.openPage(page);
    // Each page seeks hours of its own, so every cold seek is that hour's first visit in this fresh session; then the same
    // hours again (warm).
    for (const pass of ['cold', 'warm']) for (const [label, base] of SEEKS) {
      const fraction = base + index * SEEK_STEP;
      const line = await s.timeline();
      const pins = await app.evaluate(`Object.fromEntries([...document.querySelectorAll('article[data-fixed-hour]')].map(a => [a.dataset.widget, Number(a.dataset.fixedHour)]))`);
      const result = await s.step('seeks', page, pass === 'cold' ? label : `${label} warm`, async () => {
        await app.click(Session.timelineX(line, fraction), line.y);
        return { fraction, pass };
      }, { timeout: 10_000, quiet: 300, check: pass === 'cold' });
      result.hour = (await s.timeline()).value;
      const shown = (await app.evaluate('__e2e.widgets()')).filter(widget => widget.visible);
      result.pins = pins;
      result.maps = shown.filter(widget => widget.kind === 'map').map(widget => widget.key);
      result.widgetIds = Object.fromEntries(shown.map(widget => [widget.key, widget.title]));
    }
    await save();
  }
});

await phase('visual', async () => {
  await seekTo(PICTURE_SEEK[1]);
  for (const page of meta.pages) {
    await s.openPage(page);
    await visual.capture(`page/${page}`, 'page', '.dash-grid-scroll');
    const ids = await app.evaluate(`[...document.querySelectorAll('article.dash-widget')].map(a => a.dataset.widget)`);
    for (const id of ids) {
      const selector = `article.dash-widget[data-widget=${JSON.stringify(id)}]`;
      await app.evaluate(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({ block: 'start' })`);
      await app.evaluate(`(__e2e.mark(null), ${settleCall(10000, 300)})`, 60_000);
      await visual.capture(`widget/${page}/${id}`, 'widget', selector);
      // The hour it reports showing, read later from the perf log, against the clock's now.
      const check = await app.evaluate(`(() => { const a = document.querySelector(${JSON.stringify(selector)});
        return { kind: [...a.classList].find(c => c.startsWith('kind-'))?.slice(5), title: a.getAttribute('aria-label'), at: performance.timeOrigin + performance.now() }; })()`);
      s.results.push({ area: 'hours', item: check.kind, label: `${page}: ${check.title}`, page, title: check.title, kind: check.kind, widget: id, at: check.at, hour: (await s.timeline()).value, pass: true, reasons: [] });
    }
    await app.evaluate(`document.querySelector('.dash-grid-scroll').scrollTo({ top: 0 })`);
    await save();
  }
  // Key interaction states, each opened by trusted input and closed again.
  await s.openPage('Overview');
  const states = [
    ['speed menu', '.header-speed', '.playback-speed-menu'],
    ['date picker', '.header-date button.date-time-trigger', '.date-time-popover'],
    ['country picker', '.header-workspace-bar button[aria-label="Choose country"]', '.country-picker'],
    ['saved layouts', '.header-layout .select-trigger', '.select-menu'],
    ['widget settings', 'article.dash-widget:not(.kind-map) [data-widget-settings]', '.widget-settings-menu'],
    ['map settings', 'article.dash-widget.kind-map button[aria-label="Map settings"]', '.map-controls-menu'],
  ];
  for (const [name, trigger, popover] of states) {
    try {
      await s.click(trigger);
      for (let tries = 0; tries < 50 && !await s.holds(`document.querySelector('${popover}:popover-open')`); tries++) await sleep(20);
      await visual.capture(`state/${name}`, 'state', `${popover}:popover-open`);
    } catch (error) { visual.fail(`state/${name}`, 'state', error.message); }
    await s.escape();
  }
  // A tooltip, held where the pointer leaves it.
  try {
    const at = await s.need('button[aria-label="Next hour"]');
    await app.mouse('mouseMoved', at.x, at.y);
    for (let tries = 0; tries < 100 && !await s.holds(`document.querySelector('.app-tooltip:popover-open')`); tries++) await sleep(20);
    const tip = await app.evaluate(`(() => { const r = document.querySelector('.app-tooltip:popover-open')?.getBoundingClientRect(); return r ? { x: r.left, y: r.top, width: r.width, height: r.height } : null; })()`);
    if (!tip) throw new Error('no tooltip showed');
    const { data } = await app.send('Page.captureScreenshot', { format: 'png', clip: { ...tip, scale: 1 } });
    const file = visual.file('state/tooltip');
    await writeFile(path.join(visual.directory, file), Buffer.from(data, 'base64'));
    visual.items.push({ kind: 'state', caseId: 'state/tooltip', status: 'captured', model: { header: true, body: true, text: 'Next hour', lists: [] }, screenshot: file });
  } catch (error) { visual.fail('state/tooltip', 'state', error.message); }
  await app.mouse('mouseMoved', 1, 1);
  const native = new NativeInput(desktop, appPid, input);
  try { await mapPictures(s, native, visual, meta.viewport.dpr, work, SEEKS); } finally { native.close(); }
});

await phase('map', async () => {
  await s.openPage('Overview');
  const native = new NativeInput(desktop, appPid, input);
  try {
    if (only === null) await mapModes(s, meta.viewport.dpr);
    await interactions(s, native, meta.viewport.dpr, PRESETS.indexOf(SPEEDS.at(-1)), { only });
    if (only === null) await hover(s, native, meta.viewport.dpr);
  } finally { native.close(); }
  if (only !== null) return;
  // The pop-out is the campaign map's, from its own Map settings on the Campaign map tab. Opening or closing it
  // closes the menu once done; the menu, opened again, shows which state it reached.
  await s.click('#tab-map');
  await sleep(1500);
  const settings = '.header-workspace-tools button[aria-label="Map settings"]';
  const menu = `document.querySelector('.map-controls-menu:popover-open')`;
  const offers = text => s.app.evaluate(`!![...document.querySelectorAll('.map-controls-menu:popover-open .map-menu-action')].find(b => b.textContent.includes(${JSON.stringify(text)}))`);
  const reached = async text => {
    for (const end = Date.now() + 20_000; Date.now() < end; await sleep(250)) {
      if (!await s.holds(menu)) await s.click(settings);
      await sleep(150);
      if (await offers(text)) return true;
      await s.escape();
    }
    return false;
  };
  await s.step('map', 'pop-out', 'open settings', () => s.click(settings), { done: menu });
  // Open once its viewer reports the campaign loaded at the clock's hour; the report times it to that viewer's first
  // presented frame of that hour. Closed once the host reports the pop-out closed.
  const hour = (await s.timeline()).value;
  const loaded = `__e2e.popoutStates.find(x => x.receivedAt > __e2e.t0Epoch && x.status?.ready && x.status.viewer?.path && x.status.viewer.hour === ${hour} && !x.status.error && !x.error)`;
  const opened = await s.step('map', 'pop-out', 'open pop-out', () => s.click('.map-controls-menu:popover-open .map-menu-action', { text: 'Pop out map', partial: true }),
    { done: loaded, timeout: 20_000 });
  opened.hour = hour;
  if (!await reached('Close pop-out map')) await s.fail('map', 'pop-out', 'opened', 'the pop-out map never opened within 20 s');
  await s.step('map', 'pop-out', 'close pop-out', () => s.click('.map-controls-menu:popover-open .map-menu-action', { text: 'Close pop-out map', partial: true }),
    { done: `__e2e.popoutStates.some(x => x.receivedAt > __e2e.t0Epoch && x.open === false)`, timeout: 10_000 });
  if (!await reached('Pop out map')) await s.fail('map', 'pop-out', 'closed', 'the pop-out map never closed within 20 s');
  await s.escape();
  await s.openPage('Overview');
});

await phase('mapfps', async () => {
  const native = new NativeInput(desktop, appPid, input);
  try { await interactions(s, native, meta.viewport.dpr, PRESETS.indexOf(SPEEDS.at(-1)), { label: 'uncapped', only }); } finally { native.close(); }
});

/** `page` played under the browser's trace (`trace.mjs`): its summary is recorded, the raw trace kept beside the results
 * (it opens in DevTools' performance panel). The perf log runs too, so the report can take the app's own painting out. */
const traceWindow = async page => {
  let stop = null;
  try {
    await setSpeed(TRACE_SPEED);
    await seekTo(0.3);
    await app.evaluate('window.__perf.start()');
    stop = await startTrace(app);
    const from = s.epoch + await app.evaluate(`(performance.mark(${JSON.stringify(TRACE_MARKS.from)}), performance.now())`);
    await replies?.mark('play', { page, speed: TRACE_SPEED, trace: true });
    await s.click('button[aria-label="Play recording"]');
    await sleep(TRACE_SECONDS * 1000);
    await s.click('button[aria-label="Pause playback"]');
    await replies?.mark('pause', { page, speed: TRACE_SPEED, trace: true });
    const to = s.epoch + await app.evaluate(`(performance.mark(${JSON.stringify(TRACE_MARKS.to)}), performance.now())`);
    const events = await stop();
    stop = null;
    await app.evaluate('window.__perf.stop()', 120_000);
    const summary = summariseTrace(events);
    summary.described = await describeNodes(app, summaryNodes(summary));
    const file = path.join('traces', `${session}-${page.replace(/[^\w.-]+/g, '_')}.json.gz`);
    await mkdir(path.join(out, 'traces'), { recursive: true });
    await writeFile(path.join(out, file), await promisify(gzip)(JSON.stringify({ traceEvents: events })));
    const reasons = summary.reason ? [summary.reason] : !summary.frames ? ['the trace holds no frames of the window'] : [];
    await s.record({ area: 'trace', item: page, label: null, pass: true, reasons, from, to, speed: TRACE_SPEED, page, trace: summary, file, screenshot: false });
  } catch (error) {
    if (stop) await stop().catch(() => {});
    const pause = 'button[aria-label="Pause playback"]';
    if (await app.evaluate(`!!document.querySelector(${JSON.stringify(pause)})`).catch(() => false)) await s.click(pause).catch(() => {});
    await app.evaluate('window.__perf.stop()', 120_000).catch(() => {});
    await s.record({ area: 'trace', item: page, label: null, pass: false, reasons: [`the browser trace could not be taken: ${error.message}`], page, speed: TRACE_SPEED, screenshot: false });
  }
};

const RESEARCH_INVENTORY = `(() => {
  const grid = document.querySelector('.dash-grid-scroll'), viewport = grid.getBoundingClientRect();
  const visible = node => {
    if (!node) return false;
    const box = node.getBoundingClientRect();
    let top = Math.max(box.top, viewport.top), bottom = Math.min(box.bottom, viewport.bottom);
    for (let p = node.parentElement; p && p !== grid; p = p.parentElement) {
      if (getComputedStyle(p).overflowY === 'visible') continue;
      const clip = p.getBoundingClientRect();
      top = Math.max(top, clip.top); bottom = Math.min(bottom, clip.bottom);
    }
    return box.width > 0 && top < bottom;
  };
  return [...grid.querySelectorAll('article.dash-widget')].map(a => {
    const kind = [...a.classList].find(c => c.startsWith('kind-'))?.slice(5);
    const surfaces = kind === 'special-projects' ? [
      ...visible(a.querySelector('.special-project-list')) ? ['special-projects'] : [],
      ...visible(a.querySelector('.project-details')) ? ['special-project-details'] : [],
    ] : [kind === 'research' || kind === 'active-research' ? 'research-slots' : kind];
    return { id: a.dataset.widget, title: a.getAttribute('aria-label'), kind, visible: visible(a), surfaces };
  });
})()`;

async function researchInputs() {
  await app.evaluate(`(() => {
    const visibility = [], actions = [];
    const scan = () => {
      const shown = [];
      for (const c of document.querySelectorAll('article canvas[data-placement]')) {
        const bounds = c.hidden ? c.parentElement : c;
        if (!bounds?.checkVisibility()) continue;
        const article = c.closest('article'), pin = article.getAttribute('data-fixed-hour');
        const rect = bounds.getBoundingClientRect();
        let top = Math.max(0, rect.top), bottom = Math.min(innerHeight, rect.bottom);
        for (let p = c.parentElement; p && bottom > top; p = p.parentElement) {
          if (getComputedStyle(p).overflowY === 'visible') continue;
          const clip = p.getBoundingClientRect(); top = Math.max(top, clip.top); bottom = Math.min(bottom, clip.bottom);
        }
        if (bottom > top && rect.width > 0) shown.push([Number(c.dataset.placement), pin === null ? null : Number(pin), article.dataset.widget]);
      }
      if (JSON.stringify(shown) !== JSON.stringify(visibility.at(-1)?.shown))
        visibility.push({ at: performance.timeOrigin + performance.now(), shown });
    };
    const observer = new MutationObserver(scan);
    observer.observe(document.querySelector('.dash-grid-scroll'), { subtree: true, childList: true, attributes: true,
      attributeFilter: ['hidden','style','data-fixed-hour','data-placement'] });
    addEventListener('scroll', scan, true); scan();
    __e2e.inputMark = kind => actions.push({ kind,
      pinWidget: kind === 'pin' ? document.querySelector('article.kind-active-research')?.dataset.widget : undefined, at: performance.timeOrigin + performance.now() });
    __e2e.endInputs = () => { observer.disconnect(); removeEventListener('scroll', scan, true);
      return { actions, visibility, to: performance.timeOrigin + performance.now() }; };
  })()`);
  const mark = kind => app.evaluate(`__e2e.inputMark(${JSON.stringify(kind)})`);
  await mark('rewind');
  const line = await s.timeline();
  await app.click(Session.timelineX(line, .22), line.y);
  if (!await s.until(`Number(document.querySelector('input.header-timeline').value) < ${line.value}`)) throw new Error('Research rewind did not move back');
  await s.frames(4);
  const x = fraction => Session.timelineX(line, fraction);
  await app.mouse('mousePressed', x(.22), line.y, { buttons: 1 });
  for (const fraction of [.3,.36,.27,.31]) {
    await app.mouse('mouseMoved', x(fraction), line.y, { buttons: 1 }); await s.frames(1);
  }
  await app.mouse('mouseReleased', x(.31), line.y);
  await s.frames(4);
  const held = `['active-research','research','special-projects','nuclear-arsenal','doctrine-mastery'].every(kind => {
    const article = document.querySelector('article.kind-' + kind);
    return article && [...article.querySelectorAll('canvas[data-placement]:not([hidden])')].length > 0;
  })`;
  if (!await s.until(held)) throw new Error('Research scrub did not hold a complete visible position');
  await mark('scrub');
  const article = 'article.kind-active-research';
  const config = article + ' [data-widget-settings]', time = '.widget-settings-menu:popover-open [aria-label="Time"] button';
  await s.click(config);
  if (!await s.until(`document.querySelector('${time}')`)) throw new Error('Research pin settings did not open');
  await mark('pin'); await s.click(time, { text: 'Pinned' });
  if (!await s.until(`document.querySelector('${article}[data-fixed-hour]')`)) throw new Error('Active research did not take its pin');
  await s.escape(); await s.frames(4);
  await s.click('button[aria-label="Play recording"]');
  await mark('speed'); await setSpeed(168); await s.frames(5);
  await mark('scroll');
  const container = '.kind-active-research .active-research';
  const before = await app.evaluate(`document.querySelector('${container}').scrollTop`);
  const point = await s.need(container);
  await app.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: point.x, y: point.y, deltaX: 0, deltaY: 250 });
  if (!await s.until(`document.querySelector('${container}').scrollTop > ${before} && document.querySelector('${container} [data-worker-panel] canvas[data-placement]:not([hidden])')`))
    throw new Error('The inner scroller did not move with a worker-owned panel');
  await s.frames(5);
  await mark('pause'); await s.click('button[aria-label="Pause playback"]'); await s.frames(5);
  const inputs = await app.evaluate('__e2e.endInputs()');
  await s.click(config); await s.click(time, { text: 'Live' }); await s.escape();
  return inputs;
}

await phase('playback', async () => {
  if (!await s.holds('window.__perf')) throw new Error('the app was not started with BATTLE_PLAN_PERF_LOG: playback cannot be measured');
  const requested = onlyPages ?? (quick ? QUICK_PLAYBACK.pages : meta.pages);
  for (const page of requested) {
    if (!meta.pages.includes(page)) { await s.fail('playback', page, null, 'page missing from the recording'); continue; }
    await s.openPage(page);
    const stops = page === 'Research' ? await app.evaluate(`(() => {
      const grid = document.querySelector('.dash-grid-scroll');
      const max = Math.max(0, grid.scrollHeight - grid.clientHeight);
      const offsets = [0];
      for (let y = grid.clientHeight; y < max; y += grid.clientHeight) offsets.push(y);
      if (max) offsets.push(max);
      return offsets;
    })()`) : [0];
    for (const scroll of stops) {
      await app.evaluate(`new Promise(done => { document.querySelector('.dash-grid-scroll').scrollTop = ${scroll};
        requestAnimationFrame(() => requestAnimationFrame(() => done(true))); })`);
      for (const speed of speeds) {
        await setSpeed(speed);
        const seekStart = s.epoch + await app.evaluate('performance.now()');
        await seekTo(0.3);
        const widgets = await app.evaluate(RESEARCH_INVENTORY);
        await throttle(rate);
        const research = page === 'Research' && speed === 1440;
        const pressureBefore = research && await app.evaluate('window.__perf.pressureMerges()');
        if (research) await app.evaluate('window.__perf.beginAudit()');
        const trace = research && await startTrace(app, ['disabled-by-default-display.framedisplayed', 'blink.user_timing']);
        const clockMark = trace && await app.evaluate("performance.timeOrigin + performance.mark('research-clock').startTime");
        await app.evaluate('window.__perf.start()');
        if (page === 'Research') await app.evaluate(`(() => {
          const reports = [], observed = new Set();
          const observer = new IntersectionObserver(entries => {
            for (const e of entries) {
              const id = e.target.closest('article.dash-widget')?.dataset.widget;
              if (id) reports.push({ id, at: performance.timeOrigin + e.time,
                surface: e.target.matches('article') ? null : e.target.matches('.project-details') ? 'special-project-details' : 'special-projects',
                top: e.boundingClientRect.top, bottom: e.boundingClientRect.bottom, scrollTop: document.querySelector('.dash-grid-scroll').scrollTop,
                visible: e.isIntersecting && e.intersectionRect.width > 0 && e.intersectionRect.height > 0 });
            }
          });
          const scan = () => {
            for (const node of document.querySelectorAll('article.dash-widget,article.kind-special-projects .special-project-list,article.kind-special-projects .project-details')) {
              if (!observed.has(node)) { observed.add(node); observer.observe(node); }
            }
          };
          const layout = new MutationObserver(scan);
          for (const node of document.querySelectorAll('article.kind-special-projects .special-projects')) layout.observe(node, { childList: true });
          scan();
          __e2e.endResearchWindow = () => { layout.disconnect(); observer.disconnect(); return reports; };
        })()`);
        else await s.watch();
        const from = s.epoch + await app.evaluate('performance.now()');
        await settledForReplies();
        await replies?.mark('play', { page, speed });
        await s.click('button[aria-label="Play recording"]');
        if (research) {
          const line = await s.timeline();
          const endHour = line.value + Math.ceil((PLAY_SECONDS + 0.4) * speed);
          if (endHour > line.max || !await s.until(`Number(document.querySelector('input.header-timeline').value) >= ${endHour}`, 2 * PLAY_SECONDS * 1000 + 5000))
            throw new Error('Research playback did not cover the requested game hours');
        } else await sleep(PLAY_SECONDS * 1000);
        await s.click('button[aria-label="Pause playback"]');
        await replies?.mark('pause', { page, speed });
        const to = s.epoch + await app.evaluate('performance.now()');
        let presented, audit, pressureMerges, inputs, idleCost;
        if (trace) {
          inputs = await researchInputs();
          const events = await trace();
          const origin = clockMark - events.find(e => e.name === 'research-clock').ts / 1000;
          presented = [...new Set(events.filter(e => e.name === 'Display::FrameDisplayed').map(e => origin + e.ts / 1000))].sort((a, b) => a - b);
          inputs.presented = presented.filter(at => at >= inputs.actions[0].at && at <= inputs.to);
          await writeFile(path.join(out, `research-compositor-${scroll}.json.gz`), await promisify(gzip)(JSON.stringify(events)));
          pressureMerges = await app.evaluate('window.__perf.pressureMerges()') - pressureBefore;
          await setSpeed(1);
          await s.click('button[aria-label="Play recording"]');
          const start = await app.evaluate('performance.timeOrigin + performance.now()');
          await s.frames(120);
          idleCost = { from: start, to: await app.evaluate('performance.timeOrigin + performance.now()') };
          await s.click('button[aria-label="Pause playback"]');
          audit = await app.evaluate('window.__perf.auditPictures()', 240_000);
        }
        const summary = await app.evaluate('window.__perf.stop()', 120_000);
        const visibility = page === 'Research' ? await app.evaluate('__e2e.endResearchWindow()') : undefined;
        await throttle(1);
        await s.record({ area: 'playback', item: `${page}/${speed}`, label: null, pass: true, reasons: page === 'Research' ? [] : await s.unwatch(), from, to, seekStart, speed, page, scroll, visibility, seconds: PLAY_SECONDS, widgets, lastHour: (await s.timeline()).max,
          presented, pressureMerges, audit, inputs, idleCost, frames: summary.ui_frames, complete: summary.complete, losses: summary.losses, maps: summary.playback?.maps ?? null, screenshot: false });
      }
    }
    await traceWindow(page);
    await save();
  }
  await setSpeed(1);
  if (phases.includes('visual')) zoomedMapPicture(visual, SEEKS.at(-1));
});

await phase('catalogue', async () => {
  // A new page starts empty, so each card lands alone, at the top, in view.
  await s.step('catalogue', 'setup', 'add a page', () => s.click('button[aria-label="Add a page"]'), { done: `document.querySelector('.sidebar-page-list input')` });
  await s.key('Enter');
  await s.until(`!document.querySelector('.sidebar-page-list input')`, 3000);
  if (await app.evaluate(`document.querySelectorAll('article.dash-widget').length`)) throw new Error('a new page is not empty');
  await seekTo(PICTURE_SEEK[1]);
  const editing = `document.querySelector('button[aria-label="Done arranging"]')`;
  const catalogueOpen = `document.querySelector('.widget-catalogue:popover-open')`;
  const empty = `document.querySelectorAll('article.dash-widget').length === 0`;
  // The catalogue's first opening in an editing session is measured; opening it again for the next card on the same empty
  // page is the same case, done unmeasured on its signal, unless its step ever failed after that card.
  let openedThisEdit = false;
  const edit = card => { openedThisEdit = false; return s.step('catalogue', 'setup', 'edit', () => s.click('button[aria-label="Edit layout"]'), { done: editing, extra: { card } }); };
  const open = async card => {
    if (!openedThisEdit || s.everFailed('catalogue', card, 'open catalogue')) {
      openedThisEdit = true;
      return s.step('catalogue', 'setup', 'open catalogue', () => s.click('button[aria-label="Add widget"]'), { done: catalogueOpen, extra: { card } });
    }
    await s.click('button[aria-label="Add widget"]');
    if (!await s.until(catalogueOpen, 3000)) await s.record({ area: 'catalogue', item: 'setup', card, label: 'open catalogue', ms: null, pass: false, reasons: ['the catalogue never opened within 3 s'] });
  };
  await edit(null);
  await open(null);
  const cards = await app.evaluate(`[...document.querySelectorAll('.widget-catalogue:popover-open .widget-card')].map(c => ({
    title: c.querySelector('.widget-card-text strong')?.textContent.trim(), view: c.hasAttribute('data-variant'),
    unavailable: c.querySelector('.widget-card-add')?.disabled ? (c.querySelector('.widget-card-text small')?.textContent.trim() || 'cannot be added') : null }))`);
  meta.catalogue = cards;
  // The quick tier adds only the cards the last full run found slowest to show content.
  const slowest = quick ? new Set(history?.slowest ?? []) : null;
  if (slowest && !slowest.size) throw new Error('no record of the slowest catalogue cards: a full run with --history writes them');
  for (const [index, card] of cards.entries()) {
    const item = `${card.view ? 'view' : 'widget'}/${card.title}`;
    if (slowest && !slowest.has(item)) continue;
    if (card.unavailable) { await s.fail('catalogue', item, 'add', `cannot be added to this recording: ${card.unavailable}`); continue; }
    if (!await s.holds(editing)) await edit(item);
    if (!await s.holds(catalogueOpen)) await open(item);
    const added = await s.step('catalogue', item, 'add', async () => {
      const at = await app.evaluate(`(() => { const b = document.querySelectorAll('.widget-catalogue:popover-open .widget-card-add')[${index}]; if (!b) return null;
        b.scrollIntoView({ block: 'nearest' }); const r = b.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
      if (!at) throw new Error('its card is gone');
      await app.click(at.x, at.y);
    }, { done: `document.querySelectorAll('article.dash-widget').length === 1`, timeout: 10_000, quiet: 300 });
    added.view = card.view;
    added.widget = await app.evaluate(`document.querySelector('article.dash-widget')?.dataset.widget ?? null`);
    added.hour = (await s.timeline()).value;
    // The catalogue closed, so the picture is of the view alone.
    await s.escape();
    // The native map draws only once arranging is done: it is shown through Done arranging, timed from that click to the
    // frame presenting the clock's hour, and then deleted, as it is saved.
    const map = await s.holds(`document.querySelector('article.dash-widget.kind-map')`);
    if (map && added.widget) {
      const shown = await s.step('catalogue', item, 'show map', () => s.click('button[aria-label="Done arranging"]'),
        { done: `__e2e.mapState(${JSON.stringify(added.widget)}, __e2e.t0Epoch, x => !x.loading && !x.error && x.hour === ${added.hour})`, timeout: 10_000, quiet: 300 });
      shown.drawnAt = await app.evaluate(`__e2e.mapState(${JSON.stringify(added.widget)}, __e2e.t0Epoch, x => !x.loading && !x.error && x.hour === ${added.hour})?.drawnAt ?? null`);
      added.shownFrom = shown.input;
    }
    // The pictures are the full tier's visual check; the quick tier takes none.
    if (!quick) await visual.capture(`catalogue/${String(index).padStart(3, '0')} ${card.title}`, 'catalogue', 'article.dash-widget');
    const extra = { card: item };
    if (map) {
      // A widget is deleted only while arranging; Done then keeps the page empty for the next card.
      if (!await s.holds(editing)) await edit(item);
      await s.click('article.dash-widget button[aria-label="Map settings"]');
      await s.until(`document.querySelector('.map-controls-menu:popover-open button[aria-label="Delete widget"]')`, 3000);
      await s.step('catalogue', 'setup', 'delete', () => s.click('.map-controls-menu:popover-open button[aria-label="Delete widget"]'), { done: empty, extra });
      await s.step('catalogue', 'setup', 'done', () => s.click('button[aria-label="Done arranging"]'),
        { done: `${empty} && document.querySelector('button[aria-label="Edit layout"]')`, extra });
    } else {
      // Deleted from its own settings while arranging: the page is empty again, still being arranged, for the next card. A
      // card whose discard failed in a run before keeps that case, as does one whose menu offers no delete.
      const remove = ':is(.widget-settings-menu, .map-controls-menu):popover-open button[aria-label="Delete widget"]';
      let offered = false;
      if (!s.everFailed('catalogue', item, 'discard')) {
        await s.click('article.dash-widget [data-widget-settings], article.dash-widget button[aria-label="Map settings"]');
        offered = await s.until(`document.querySelector(${JSON.stringify(remove)})`, 3000);
        if (!offered) await s.escape();
      }
      if (offered) await s.step('catalogue', 'setup', 'delete', () => s.click(remove), { done: `${empty} && ${editing}`, extra });
      else await s.step('catalogue', 'setup', 'discard', () => s.click('button[aria-label="Cancel layout changes"]'),
        { done: `${empty} && document.querySelector('button[aria-label="Edit layout"]')`, extra });
    }
    if ((index + 1) % 10 === 0) await save();
  }
  // What is left arranging is the empty page, saved as it is.
  if (await s.holds(editing)) await s.step('catalogue', 'setup', 'done', () => s.click('button[aria-label="Done arranging"]'),
    { done: `${empty} && document.querySelector('button[aria-label="Edit layout"]')` });
});

await phase('controls', async () => {
  // The host's save dialog and a colour input's dialog are native windows: the suite's native input fills them.
  s.native = new NativeInput(desktop, appPid, input);
  s.work = work;
  try { await controls(); } finally { s.native.close(); s.native = null; }
});

async function controls() {
  for (const control of CASES) {
    if (only && !only.includes(control.id)) continue;
    // The quick tier: one setting of each widget kind.
    if (quick && control.id !== 'widget/settings') continue;
    const pages = control.page === '*' ? meta.pages : control.page === null ? [await s.currentPage()] : [control.page];
    for (const page of pages.filter(name => onlyPages === null || onlyPages.includes(name))) {
      try {
        await s.openPage(page);
        await control.run(s, page);
      } catch (error) {
        await s.fail('controls', control.id, page, error.message);
      }
      await s.escape();
      await s.escape();
      // A case left editing discards its changes, so the next starts from the saved layout.
      if (await s.holds(`document.querySelector('button[aria-label="Done arranging"]')`)) await s.click('button[aria-label="Cancel layout changes"]').catch(() => {});
      await save();
    }
  }
}

await app.evaluate('window.__perf?.flush()').catch(() => {});
if (replies) {
  meta.replies = { file: recordReplies, counts: await replies.finish() };
  log('replies recorded', JSON.stringify(meta.replies.counts), 'to', recordReplies);
}
meta.finished = new Date().toISOString();
meta.seconds = (Date.now() - started.getTime()) / 1000;
await save();
log('sweep done', Math.round(meta.seconds), 's,', s.results.filter(r => !r.pass).length, 'failed of', s.results.length);
app.close();
