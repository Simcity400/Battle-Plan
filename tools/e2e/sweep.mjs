import { existsSync } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';
import { gzip } from 'node:zlib';
import { connect, log, sleep } from './cdp.mjs';
import { Session, settleCall } from './session.mjs';
import { scenarioId, scopeScenario } from './perfBaseline.mjs';
import { CASES } from './controls.mjs';
import { PINS, readHistory } from './coverage.mjs';
import { Visual } from './visual.mjs';
import { NativeInput, hover, interactions, mapModes, mapPictures, zoomedMapPicture } from './map.mjs';
import { classifyWidgetSnapshot, pageShown } from './verdicts.mjs';
import { buildWidgetClassifierExpr, catalogueItemKey, createOutcomeWatcher, describeUncleanCataloguePage, finishCatalogueCard, measureIdleCost, mergeWidgetOutcomes, runThrottled, selectQuickCatalogueCards } from './sweepHelpers.mjs';
import { TRACE_MARKS, describeNodes, startTrace, summariseTrace, summaryNodes } from './trace.mjs';
import { ReplyRecorder } from './replies.mjs';
import { allWidgetPages, costPages, HEAVIEST, HEAVIEST_COUNT, SCREEN, SHELL_HEADER_HEIGHT, VIEWS, screenMetrics, tallViewportHeight } from './allWidgets.mjs';

const flag = (name, fallback = null) => { const at = process.argv.indexOf(name); return at < 0 ? fallback : process.argv[at + 1]; };
const port = Number(flag('--port'));
const out = flag('--out');
const work = flag('--work');
const desktop = flag('--desktop');
const appPid = Number(flag('--app-pid'));
const session = flag('--session');
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
const only = flag('--cases')?.split(',') ?? null;
const onlyPages = flag('--pages')?.split(',') ?? null;
const onlyKinds = flag('--kinds')?.split(',').filter(Boolean) ?? null;
const allViews = process.argv.includes('--all-views');
const costsFile = flag('--costs');
const heaviestCount = flag('--heaviest') === null ? HEAVIEST_COUNT : Number(flag('--heaviest'));
if (!(heaviestCount >= 1 && Number.isInteger(heaviestCount))) throw new Error(`--heaviest takes a positive integer, not ${flag('--heaviest')}`);
const coresCount = flag('--cores-count') === null ? null : Number(flag('--cores-count'));
if (coresCount !== null && !(coresCount >= 1 && Number.isInteger(coresCount))) throw new Error(`--cores-count takes a positive integer, not ${flag('--cores-count')}`);
const runningCosts = phases.includes('widget-costs');
const costFixture = runningCosts ? await costPages(onlyKinds, { allViews }) : null;
const loadedCosts = costsFile ? JSON.parse(await readFile(costsFile, 'utf8')) : null;
if (onlyPages?.includes(HEAVIEST) && !loadedCosts) throw new Error('--pages Heaviest requires --costs <widget-costs.json>');
const fixture = !costFixture && onlyPages?.some(name => Object.hasOwn(VIEWS, name))
  ? await allWidgetPages({ costs: loadedCosts, heaviestCount, allViews }) : null;
const pageOf = name => fixture && Object.hasOwn(VIEWS, name) ? VIEWS[name].page : name;
const pagesAt = flag('--pages-at') === null ? null : Number(flag('--pages-at'));
const recordReplies = flag('--record-replies');
if (pagesAt !== null && !(pagesAt >= 0 && pagesAt <= 1)) throw new Error(`--pages-at takes a share of the timeline from 0 to 1, not ${flag('--pages-at')}`);
export const SEEKS = [['early', 0.05], ['mid-war', 0.5], ['late', 0.95]];
const SEEK_STEP = 0.004;
const PICTURE_SEEK = SEEKS[1];
export const SPEEDS = [24, 1440];
const QUICK_PLAYBACK = { pages: ['Politics', 'Overview', 'Army'], speeds: [1440] };
const GLANCE_AT = 0.5;
const PRESETS = [0.25, 1, 4, 24, 168, 720, 1440];
const PLAY_SECONDS = Number((process.env.BATTLEPLAN_PLAY_SECONDS ?? process.env.HOI4_PLAY_SECONDS) ?? 8);
const COST_SECONDS = Number(process.env.BATTLEPLAN_COST_SECONDS ?? 4);
const rate = Number(flag('--cpu-throttle', '1'));
if (!(rate >= 1 && Number.isFinite(rate) && PLAY_SECONDS > 0 && Number.isFinite(PLAY_SECONDS) && COST_SECONDS > 0 && Number.isFinite(COST_SECONDS))) throw new Error('Invalid playback duration or throttle');
const TRACE_SPEED = 1440;
const TRACE_SECONDS = 4;
const CATALOGUE_PAGE = 'Intelligence';
const visibleControl = selector => `(() => { const el = document.querySelector(${JSON.stringify(selector)}); return !!el && el.checkVisibility({ visibilityProperty: true }); })()`;
const discardLayout = async () => {
  await s.click('button[aria-label="Discard layout changes"]');
  const confirm = '.layout-discard-menu:popover-open button.is-danger';
  await s.until(`${visibleControl(confirm)} || !!document.querySelector('button[aria-label="Edit layout"]')`, 1500);
  if (await s.holds(visibleControl(confirm))) await s.click(confirm);
};

const app = await connect(port);
const applyConfinement = async () => {
  if (coresCount !== null) await app.send('Emulation.setHardwareConcurrencyOverride', { hardwareConcurrency: coresCount });
};
await applyConfinement();
const activeFixture = costFixture ?? fixture;
if (activeFixture) {
  await app.evaluate(`(${JSON.stringify(activeFixture.entries)}.forEach(([key, value]) => localStorage.setItem(key, value)), true)`);
  const loaded = new Promise(resolve => { const stop = app.on('Page.loadEventFired', () => { stop(); resolve(); }); });
  await app.send('Page.reload');
  await loaded;
  await applyConfinement();
  if (fixture) for (const entry of Object.values(fixture.pages)) log('fixture', `${entry.label}: ${entry.widgets.length} widgets, ${entry.rows} rows`);
  if (costFixture) log('fixture', `widget-costs: ${costFixture.kinds.length} widget placements plus empty baseline`);
}
const replies = recordReplies ? await ReplyRecorder.start(app, recordReplies,
  { pid: appPid, revision: flag('--app-revision'), root: path.resolve(import.meta.dirname, '../..') }) : null;
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
s.recipe = { version: 1, branchIndex: 0, tier, phases, cases: only, pages: onlyPages, rate, coresCount, allViews, heaviestCount, kinds: onlyKinds, pagesAt, playSeconds: PLAY_SECONDS };
const visual = new Visual(app, out, session);
await visual.init();
const started = new Date();
const speeds = quick ? QUICK_PLAYBACK.speeds : SPEEDS;
const readConfinement = () => app.evaluate(`({
  hardwareConcurrency: navigator.hardwareConcurrency,
  lanes: window.__perf?.lanes?.() ?? null,
  expectedLanes: Math.max(1, Math.min(4, (navigator.hardwareConcurrency || 2) - 2)),
})`);
const meta = { session, tier, requestedPages: onlyPages, cpuThrottle: rate, coresCount, confinement: null, started: started.toISOString(), port, phases, epoch: s.epoch, pages: [], viewport: null, cataloguePage: CATALOGUE_PAGE, speeds, seeks: SEEKS, pictureSeek: PICTURE_SEEK };
const save = async () => {
  meta.recipe = s.recipe;
  await replies?.take();
  if (phases.includes('seeks')) meta.seekInputs = await app.evaluate('__e2e.seekInputs ?? []');
  const pageConf = await readConfinement().catch(() => null);
  if (pageConf) meta.confinement = { coresCount, cpuThrottle: rate, ...pageConf, lanes: pageConf.lanes ?? meta.confinement?.lanes ?? null };
  const results = s.results.map(({ probe, ...result }) => ({ ...result, session }));
  await writeFile(path.join(out, `sweep-${session}.json`), JSON.stringify({ meta, results, console: app.messages.map(message => ({ ...message, session })) }, null, 1));
  await visual.finish({ pictureSeek: PICTURE_SEEK, viewport: meta.viewport });
};
const settledForReplies = async () => {
  if (!replies) return;
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
const seekTo = async fraction => {
  const line = await s.timeline();
  await app.click(Session.timelineX(line, fraction), line.y);
  await app.evaluate(`(__e2e.mark(null), ${settleCall(15000, 300)})`, 60_000);
};
const pausePlayback = async () => {
  const at = await s.locate('button.header-play[aria-label="Pause"]');
  if (at) await app.click(at.x, at.y);
};
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

app.setCase('startup');
const loadRow = '.sidebar-menu button.sidebar-row[aria-label^="Load "]';
const shelf = '.sidebar-library button.sidebar-group-toggle[aria-expanded="false"]';
for (let waited = 0; !await app.evaluate(`document.querySelectorAll(${JSON.stringify(loadRow)}).length > 0`).catch(() => false); waited++) {
  if (waited >= 120) throw new Error(`the library never listed the fixture: ${(await app.evaluate('document.body.innerText').catch(() => '')).slice(0, 400)}`);
  if (await app.evaluate(`!!document.querySelector(${JSON.stringify(shelf)})`).catch(() => false)) await s.click(shelf);
  await sleep(500);
}
const rows = await app.evaluate(`[...document.querySelectorAll(${JSON.stringify(loadRow)})].map(b => b.getAttribute('aria-label'))`);
if (rows.length !== 1) throw new Error(`the fresh profile's library lists ${rows.length} recordings, not the fixture alone: ${rows.join(', ')}`);
await s.frames(2);
await runThrottled(throttle, rate, () => s.step('startup', 'open campaign', rows[0], () => s.click(loadRow),
  { done: `document.querySelector('input.header-timeline:not([disabled])') && document.querySelectorAll('.sidebar-page-list button.sidebar-subrow').length > 0`, timeout: 120_000, quiet: 500, extra: { cpuThrottle: rate } }));
if (!await s.holds(`document.querySelector('input.header-timeline:not([disabled])')`)) {
  await save().catch(() => {});
  throw new Error(`the campaign never opened (console: ${JSON.stringify(app.messages.slice(-10))}): ${(await app.evaluate('document.body.innerText').catch(() => '')).slice(0, 400)}`);
}
meta.pages = await s.pages();
meta.viewport = await app.evaluate(`({ width: innerWidth, height: innerHeight, dpr: devicePixelRatio })`);
meta.startPage = await s.currentPage();
log('pages', meta.pages.join(', '), 'viewport', JSON.stringify(meta.viewport), 'start', meta.startPage);
if (replies) {
  await s.until(`document.querySelector('.dash-grid article.dash-widget')`, 30_000);
  await settledForReplies();
  await replies.mark('start', { page: meta.startPage ?? meta.pages[0] });
}

const THROTTLED_PHASES = new Set(['pages', 'pages-at', 'glance', 'seeks', 'catalogue', 'controls']);
async function phase(name, run) {
  if (!phases.includes(name)) return;
  log('phase', name);
  const began = Date.now();
  try {
    if (THROTTLED_PHASES.has(name)) await runThrottled(throttle, rate, run);
    else await run();
  } catch (error) {
    await s.fail(name, 'phase', null, `stopped: ${error.stack ?? error.message}`);
  }
  meta[`${name}Seconds`] = (Date.now() - began) / 1000;
  await s.escape().catch(() => {});
  await save();
}

async function visit(pass, page, { area = 'pages', extra = {} } = {}) {
  if (meta.startPage === page && pass === 'cold')
    await s.fail(area, `cold/${page}`, 'switch', 'shown at startup before any input: its cold visit cannot be measured from input');
  const check = pass !== 'warm';
  await s.step(area, `${pass}/${page}`, 'switch', () => s.click('.sidebar-page-list button.sidebar-subrow', { text: page }),
    { done: pageShown(page), timeout: 8000, quiet: 300, check, shot: true, extra });
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

await phase('pages-at', async () => {
  if (pagesAt === null) throw new Error('no --pages-at: the hour to visit the pages at was not given');
  meta.pagesAt = { fraction: pagesAt, hours: [] };
  for (const page of meta.pages) {
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

await phase('glance', async () => {
  meta.pagesAt = { fraction: GLANCE_AT, hours: [] };
  await seekTo(GLANCE_AT);
  const hour = (await s.timeline()).value;
  meta.pagesAt.hours.push(hour);
  const first = meta.pages.indexOf(await s.currentPage());
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
      const check = await app.evaluate(`(() => { const a = document.querySelector(${JSON.stringify(selector)});
        return { kind: [...a.classList].find(c => c.startsWith('kind-'))?.slice(5), title: a.getAttribute('aria-label'), at: performance.timeOrigin + performance.now() }; })()`);
      s.results.push({ area: 'hours', item: check.kind, label: `${page}: ${check.title}`, page, title: check.title, kind: check.kind, widget: id, at: check.at, hour: (await s.timeline()).value, pass: true, reasons: [] });
    }
    await app.evaluate(`document.querySelector('.dash-grid-scroll').scrollTo({ top: 0 })`);
    await save();
  }
  await s.openPage('Overview');
  const states = [
    ['speed menu', '.header-speed', '.playback-speed-menu'],
    ['date picker', '.header-date button.date-time-trigger', '.date-time-popover'],
    ['country picker', '.header-workspace-bar button[aria-label^="Choose country"]', '.country-picker'],
    ['saved layouts', '.header-layout-trigger', '.header-layout-menu'],
    ['widget settings', 'article.dash-widget:not(.kind-map) [data-widget-settings]', '.widget-settings-menu'],
    ['map settings', 'article.dash-widget.kind-map :is([data-widget-settings], button[aria-label="Map settings"])', ':is(.widget-settings-menu, .map-controls-menu)'],
  ];
  for (const [name, trigger, popover] of states) {
    try {
      await s.click(trigger);
      for (let tries = 0; tries < 50 && !await s.holds(`document.querySelector('${popover}:popover-open')`); tries++) await sleep(20);
      await visual.capture(`state/${name}`, 'state', `${popover}:popover-open`);
    } catch (error) { visual.fail(`state/${name}`, 'state', error.message); }
    await s.escape();
  }
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
  try {
    const pictures = await mapPictures(s, native, visual, meta.viewport.dpr, work, SEEKS);
    await zoomedMapPicture(s, native, visual, meta.viewport.dpr, work, SEEKS.at(-1), pictures.at(-1)?.state ?? null);
  } finally { native.close(); }
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
  const hour = (await s.timeline()).value;
  const loaded = `__e2e.popoutStates.find(x => x.receivedAt > __e2e.t0Epoch && x.status?.ready && x.status.viewer?.path && x.status.viewer.hour === ${hour} && !x.status.error && !x.error)`;
  await s.step('map', 'pop-out', 'open pop-out', () => s.click('.map-controls-menu:popover-open .map-menu-action', { text: 'Pop out map', partial: true }),
    { done: loaded, timeout: 20_000 });
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

const traceWindow = async page => {
  let stop = null;
  try {
    await setSpeed(TRACE_SPEED);
    await seekTo(0.3);
    const { from, to, events } = await runThrottled(throttle, rate, async () => {
      await app.evaluate('window.__perf.start()');
      stop = await startTrace(app);
      const from = s.epoch + await app.evaluate(`(performance.mark(${JSON.stringify(TRACE_MARKS.from)}), performance.now())`);
      await replies?.mark('play', { page, speed: TRACE_SPEED, trace: true });
      await s.click('button.header-play[aria-label="Play"]');
      await sleep(TRACE_SECONDS * 1000);
      await pausePlayback();
      await replies?.mark('pause', { page, speed: TRACE_SPEED, trace: true });
      const to = s.epoch + await app.evaluate(`(performance.mark(${JSON.stringify(TRACE_MARKS.to)}), performance.now())`);
      const events = await stop();
      stop = null;
      return { from, to, events };
    });
    await app.evaluate('window.__perf.stop()', 120_000);
    const summary = summariseTrace(events);
    summary.described = await describeNodes(app, summaryNodes(summary));
    const file = path.join('traces', `${session}-${page.replace(/[^\w.-]+/g, '_')}.json.gz`);
    await mkdir(path.join(out, 'traces'), { recursive: true });
    await writeFile(path.join(out, file), await promisify(gzip)(JSON.stringify({ traceEvents: events })));
    const reasons = summary.reason ? [summary.reason] : !summary.frames ? ['the trace holds no frames of the window'] : [];
    await s.record({ area: 'trace', item: page, label: null, pass: true, reasons, from, to, speed: TRACE_SPEED, cpuThrottle: rate, page, trace: summary, file, screenshot: false });
  } catch (error) {
    if (stop) await stop().catch(() => {});
    const pause = 'button.header-play[aria-label="Pause"]';
    if (await app.evaluate(`!!document.querySelector(${JSON.stringify(pause)})`).catch(() => false)) await s.click(pause).catch(() => {});
    await app.evaluate('window.__perf.stop()', 120_000).catch(() => {});
    await s.record({ area: 'trace', item: page, label: null, pass: false, reasons: [`the browser trace could not be taken: ${error.message}`], page, speed: TRACE_SPEED, cpuThrottle: rate, screenshot: false });
  }
};

const CLASSIFY_WIDGET = buildWidgetClassifierExpr(classifyWidgetSnapshot);

async function watchWidgetOutcomes({ waitForPlaying = false } = {}) {
  await app.evaluate(`(() => {
    const classify = ${CLASSIFY_WIDGET};
    const makeWatcher = ${createOutcomeWatcher.toString()};
    const watcher = makeWatcher({
      grid: document.querySelector('.dash-grid-scroll'),
      playHost: document.querySelector('button.header-play')?.parentElement ?? document.body,
      isPlaying: () => !!document.querySelector('button.header-play[aria-label="Pause"]'),
      classify,
      observe: (node, options, cb) => { const mo = new MutationObserver(cb); mo.observe(node, options); return () => mo.disconnect(); },
      requestFrame: cb => requestAnimationFrame(cb),
      cancelFrame: id => cancelAnimationFrame(id),
      waitForPlaying: ${Boolean(waitForPlaying)},
    });
    __e2e.endWidgetOutcomes = () => watcher.end();
  })()`);
}

const DASHBOARD_INVENTORY = `(() => {
  const classify = ${CLASSIFY_WIDGET};
  const grid = document.querySelector('.dash-grid-scroll'), viewport = grid.getBoundingClientRect();
  const visible = node => {
    if (!node) return false;
    if (!node.checkVisibility({ visibilityProperty: true, opacityProperty: true })) return false;
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
    const { outcome, detail } = classify(a);
    const byPlacement = new Map();
    for (const c of a.querySelectorAll('[data-surface]')) {
      const id = c.dataset.placement ?? c.dataset.surface, held = byPlacement.get(id);
      if (!held || held === 'page') byPlacement.set(id, c.dataset.surface);
    }
    const surfaces = [...new Set([...byPlacement.values()].filter(Boolean))];
    return { id: a.dataset.widget, title: a.getAttribute('aria-label'), kind, visible: visible(a), surfaces, outcome, detail };
  });
})()`;

async function playbackInputs() {
  await app.evaluate(`(() => {
    const visibility = [], actions = [];
    const scan = () => {
      const shown = [];
      for (const c of document.querySelectorAll('article [data-placement]')) {
        if (c.hidden || !c.checkVisibility({ visibilityProperty: true, opacityProperty: true })) continue;
        const bounds = c;
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
    __e2e.inputMark = (kind, notApplicable = null, at = null) => actions.push({ kind,
      ...(notApplicable ? { notApplicable } : {}),
      at: at ?? performance.timeOrigin + performance.now() });
    __e2e.inputFail = (kind, reason, at = null) => actions.push({ kind, failed: reason, at: at ?? performance.timeOrigin + performance.now() });
    __e2e.setPinWidget = widget => {
      const action = actions.at(-1);
      if (action?.kind === 'pin') action.pinWidget = widget;
    };
    __e2e.endInputs = () => { observer.disconnect(); removeEventListener('scroll', scan, true);
      return { actions, visibility, to: performance.timeOrigin + performance.now() }; };
  })()`);
  const mark = (kind, notApplicable = null, at = null) => app.evaluate(`__e2e.inputMark(${JSON.stringify(kind)}, ${JSON.stringify(notApplicable)}, ${JSON.stringify(at)})`);
  const failures = [];
  const failInput = async (kind, reason) => {
    failures.push(`${kind}: ${reason}`);
    log('input-fail', kind, reason);
    await app.evaluate(`__e2e.inputFail(${JSON.stringify(kind)}, ${JSON.stringify(reason)})`).catch(() => {});
  };
  const recordFailure = (kind, reason) => failInput(kind, reason);
  const unsettled = `(() => {
    const grid = document.querySelector('.dash-grid-scroll'), viewport = grid.getBoundingClientRect();
    const visible = node => {
      if (!node.checkVisibility({ visibilityProperty: true, opacityProperty: true })) return false;
      if (__e2e.visuallyHidden(node)) return false;
      const box = node.getBoundingClientRect();
      let top = Math.max(box.top, viewport.top), bottom = Math.min(box.bottom, viewport.bottom);
      for (let p = node.parentElement; p && p !== grid; p = p.parentElement) {
        const style = getComputedStyle(p);
        if (__e2e.visuallyHidden(p, style)) return false;
        if (style.overflowY === 'visible') continue;
        const clip = p.getBoundingClientRect(); top = Math.max(top, clip.top); bottom = Math.min(bottom, clip.bottom);
      }
      return box.width > 0 && top < bottom;
    };
    const loading = article => [...article.querySelectorAll('[data-state="loading"], .skeleton, .list-skeleton, [aria-busy="true"], .spinner, .is-loading, [role="status"]')]
      .some(node => visible(node) && (node.matches('[data-state="loading"], .skeleton, .list-skeleton, [aria-busy="true"], .spinner, .is-loading')
        || /^Loading\\b/i.test((node.textContent || '') + ' ' + (node.getAttribute('aria-label') || ''))));
    const lacking = article => {
      if (loading(article)) return 'loading';
      if ([...article.querySelectorAll('[data-state="error"], [role="alert"]')].some(visible)) return 'error';
      const canvases = [...article.querySelectorAll('[data-placement]')].filter(visible);
      if (canvases.some(canvas => canvas.closest('[data-table-pending]'))) return 'pending rows';
      if (canvases.length) return null;
      const content = article.querySelector('.dash-widget-content');
      if (!content) return 'no content';
      if ([...content.querySelectorAll('.dash-empty[role="status"], [data-state="empty"], [data-state="unavailable"], [data-state="outside"], [data-state="not-recorded"], [data-state="nothing-chosen"], [data-state="no-members"]')].some(visible)) return null;
      if ([...content.querySelectorAll('[data-row-ink="pending"]')].some(visible)) return 'pending rows';
      if ([...content.querySelectorAll('canvas, img, svg')].some(visible)) return null;
      return [...content.querySelectorAll('*')].some(node => {
        if (node.children.length || !node.textContent.trim() || node.closest('[data-row-ink="worker"], [data-row-ink="pending"]')) return false;
        return visible(node);
      }) ? null : 'blank';
    };
    const articles = [...grid.querySelectorAll('article.dash-widget')].filter(visible);
    if (!articles.length) return ['no visible widget'];
    return articles.filter(article => article.querySelector('[data-placement]'))
      .map(article => [article.getAttribute('aria-label') || article.dataset.widget, lacking(article)])
      .filter(([, why]) => why).map(([widget, reason]) => ({ widget, reason }));
  })()`;
  const settleInputs = async () => {
    const started = Date.now();
    try {
      const settled = await s.until(`${unsettled}.length === 0`, 10_000);
      if (settled) return { settledMs: Date.now() - started, unsettled: [] };
      try {
        const remaining = await app.evaluate(unsettled);
        return {
          settledMs: Date.now() - started,
          unsettled: Array.isArray(remaining) ? remaining : [{ widget: 'dashboard', reason: 'settlement probe returned no evidence' }]
        };
      } catch (cause) {
        return { settledMs: Date.now() - started, unsettled: [{ widget: 'dashboard', reason: `settlement probe failed: ${cause?.message ?? cause}` }] };
      }
    } catch (cause) {
      return { settledMs: Date.now() - started, unsettled: [{ widget: 'dashboard', reason: `settlement wait failed: ${cause?.message ?? cause}` }] };
    }
  };
  let rewind = { settledMs: 0, unsettled: [] }, scrub = { settledMs: 0, unsettled: [] };
  const line = await s.timeline();
  try {
    await mark('rewind');
    await app.click(Session.timelineX(line, .22), line.y);
    if (!await s.until(`Number(document.querySelector('input.header-timeline').value) < ${line.value}`))
      await recordFailure('rewind', 'Playback rewind did not move back');
    rewind = await settleInputs();
    await s.frames(4);
  } catch (error) { await recordFailure('rewind', error.message); }
  try {
    const x = fraction => Session.timelineX(line, fraction);
    await app.mouse('mousePressed', x(.22), line.y, { buttons: 1 });
    await mark('scrub');
    for (const fraction of [.3,.36,.27,.31]) {
      await app.mouse('mouseMoved', x(fraction), line.y, { buttons: 1 }); await s.frames(1);
    }
    const scrubTarget = Math.round(line.min + .31 * (line.max - line.min));
    await app.mouse('mouseReleased', x(.31), line.y);
    if (!await s.until(`Number(document.querySelector('input.header-timeline').value) === ${scrubTarget}`, 10_000))
      await recordFailure('scrub', `Playback scrub target ${scrubTarget} was not reached`);
    await s.frames(4);
    scrub = await settleInputs();
  } catch (error) {
    await app.mouse('mouseReleased', Session.timelineX(line, .31), line.y).catch(() => {});
    await recordFailure('scrub', error.message);
  }
  const article = 'article.dash-widget';
  const config = article + ' [data-widget-settings]', time = '.widget-settings-menu:popover-open [aria-label="Time"] button';
  let pinApplied = false, pinWidget = null;
  try {
    const { owners, maps } = await app.evaluate(`(() => {
      for (const node of document.querySelectorAll('[data-e2e-settings]')) node.removeAttribute('data-e2e-settings');
      const view = document.querySelector('.dash-grid-scroll')?.getBoundingClientRect();
      const inside = box => box.width && box.height && view && box.top >= view.top && box.bottom <= view.bottom && box.left >= view.left && box.right <= view.right;
      const owners = [];
      const seen = box => box.width && box.height && view && box.bottom > view.top && box.top < view.bottom && box.right > view.left && box.left < view.right;
      const maps = [...document.querySelectorAll('article.dash-widget.kind-map')].filter(map => seen(map.getBoundingClientRect())).length;
      for (const node of document.querySelectorAll(${JSON.stringify(config)})) {
        const owner = node.closest('article.dash-widget');
        if (!owner || owner.classList.contains('kind-map') || !inside(node.getBoundingClientRect())) continue;
        node.setAttribute('data-e2e-settings', String(owners.length));
        owners.push(owner.dataset.widget ?? null);
      }
      return { owners, maps };
    })()`);
    let unread = 0;
    for (let index = 0; index < owners.length && !pinApplied; index++) {
      const at = await s.locate(`[data-e2e-settings="${index}"]`);
      if (!at) continue;
      await app.click(at.x, at.y);
      if (!await s.until("document.querySelector('.widget-settings-menu:popover-open')")) { unread++; await s.escape(); continue; }
      const pinned = await s.until(`[...document.querySelectorAll('${time}')].some(button => button.textContent.trim() === 'Pinned')`);
      if (pinned) {
        const owner = `${article}[data-widget=${JSON.stringify(owners[index])}]`;
        const held = await app.evaluate(`document.querySelector(${JSON.stringify(owner)})?.dataset.fixedHour ?? null`);
        await mark('pin');
        await s.click(time, { text: 'Pinned' });
        if (!await s.until(`(hour => hour !== undefined && hour !== ${JSON.stringify(held)})(document.querySelector(${JSON.stringify(owner)})?.dataset.fixedHour)`)) {
          await recordFailure('pin', `Pinning widget ${owners[index]} did not fix its hour`);
        } else {
          await s.frames(4);
          pinWidget = owners[index];
          await app.evaluate(`__e2e.setPinWidget(${JSON.stringify(pinWidget)})`);
          pinApplied = true;
        }
      }
      await s.escape();
    }
    if (!pinApplied && (unread || maps)) await recordFailure('pin', `Pin was not exercised: ${unread} settings menus did not open and ${maps} map widgets keep their time setting elsewhere`);
    else if (!pinApplied) await mark('pin', 'no widget offers a pinnable time setting');
  } catch (error) {
    await s.escape().catch(() => {});
    await recordFailure('pin', error.message);
  }
  try {
    await s.click('button.header-play[aria-label="Play"]');
    await mark('speed'); await setSpeed(168); await s.frames(5);
  } catch (error) {
    await s.escape().catch(() => {});
    await recordFailure('speed', error.message);
  }
  try {
    const candidates = await s.app.evaluate(`(() => {
      for (const node of document.querySelectorAll('[data-e2e-scroll]')) node.removeAttribute('data-e2e-scroll');
      const scrolls = node => node.scrollHeight - node.clientHeight > 2 && /(auto|scroll)/.test(getComputedStyle(node).overflowY);
      let count = 0;
      for (const node of document.querySelectorAll('.dash-widget *')) {
        if (!scrolls(node)) continue;
        const box = node.getBoundingClientRect(), x = box.left + box.width / 2, y = box.top + box.height / 2;
        if (y < 0 || y > innerHeight || x < 0 || x > innerWidth) continue;
        let hit = document.elementFromPoint(x, y);
        while (hit && hit !== node && !scrolls(hit)) hit = hit.parentElement;
        if (hit !== node) continue;
        node.setAttribute('data-e2e-scroll', String(count++));
      }
      return count;
    })()`);
    if (candidates) {
      let scrolled = false;
      for (let candidate = 0; candidate < candidates && !scrolled; candidate++) {
        const target = `[data-e2e-scroll="${candidate}"]`;
        const point = await s.locate(target);
        if (!point) continue;
        const [before, wheel, at] = await app.evaluate(`(() => { const node = document.querySelector(${JSON.stringify(target)});
          const down = node ? node.scrollHeight - node.clientHeight - node.scrollTop > 2 : true;
          return [node?.scrollTop ?? 0, down ? 250 : -250, performance.timeOrigin + performance.now()]; })()`);
        await app.send('Input.dispatchMouseEvent', { type: 'mouseWheel', x: point.x, y: point.y, deltaX: 0, deltaY: wheel });
        scrolled = await s.until(`((document.querySelector(${JSON.stringify(target)})?.scrollTop ?? ${before}) - ${before}) * ${Math.sign(wheel)} > 0`, 2_000);
        if (scrolled) await mark('scroll', null, at);
      }
      if (!scrolled) await failInput('scroll', `None of ${candidates} scrollable widget areas moved under the wheel`);
    } else await mark('scroll', 'nothing to scroll');
  } catch (error) { await failInput('scroll', error.message); }
  await s.frames(5);
  try {
    await mark('pause');
    if (await s.holds(`document.querySelector('button.header-play[aria-label="Pause"]')`)) await s.click('button.header-play[aria-label="Pause"]');
    if (!await s.until("document.querySelector('button.header-play[aria-label=\"Play\"]')", 10_000))
      await recordFailure('pause', 'Playback pause did not stop the clock');
  } catch (error) { await recordFailure('pause', error.message); }
  await s.frames(5);
  const inputs = await app.evaluate('__e2e.endInputs()');
  if (pinApplied) {
    try {
      const pinnedConfig = pinWidget ? `${article}[data-widget=${JSON.stringify(pinWidget)}] [data-widget-settings]` : config;
      if (await s.holds(`document.querySelector(${JSON.stringify(time)})`)) await s.escape();
      if (!await s.until(`!document.querySelector(${JSON.stringify(time)})`)) throw new Error('Pinned widget settings menu did not close before restore');
      await s.click(pinnedConfig);
      if (!await s.until(`document.querySelector(${JSON.stringify(time)})`)) throw new Error('Pinned widget settings menu did not reopen for restore');
      await s.click(time, { text: 'Live' }); await s.escape();
    } catch (error) {
      await s.escape().catch(() => {});
      failures.push(`pin-restore: ${error.message}`);
    }
  }
  return { ...inputs, failures, rewindSettledMs: rewind.settledMs, rewindUnsettled: rewind.unsettled,
    scrubSettledMs: scrub.settledMs, unsettled: scrub.unsettled };
}

async function playbackVisibility() {
  await app.evaluate(`(() => {
    const reports = [], observed = new Map(), grid = document.querySelector('.dash-grid-scroll');
    const visible = entry => entry.target.isConnected && entry.isIntersecting && entry.intersectionRect.width > 0 && entry.intersectionRect.height > 0;
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        const held = observed.get(entry.target);
        if (!held) continue;
        reports.push({ id: held.id, placement: held.placement, at: performance.timeOrigin + entry.time, visible: visible(entry) });
      }
    });
    const scan = () => {
      for (const [canvas, held] of observed) if (!canvas.isConnected) {
        observer.unobserve(canvas); observed.delete(canvas);
        reports.push({ id: held.id, placement: held.placement, at: performance.timeOrigin + performance.now(), visible: false });
      }
      for (const canvas of grid.querySelectorAll('article.dash-widget [data-placement]'))
        if (!observed.has(canvas)) { observed.set(canvas, { id: canvas.closest('article.dash-widget').dataset.widget, placement: Number(canvas.dataset.placement) }); observer.observe(canvas); }
    };
    const layout = new MutationObserver(scan);
    layout.observe(grid, { subtree: true, childList: true, attributes: true, attributeFilter: ['hidden', 'style', 'data-placement'] });
    addEventListener('scroll', scan, true); scan();
    __e2e.endPlaybackVisibility = () => { layout.disconnect(); observer.disconnect(); removeEventListener('scroll', scan, true); return reports; };
  })()`);
}

await phase('playback', async () => {
  if (!await s.holds('window.__perf')) throw new Error('the app was not started with BATTLEPLAN_PERF_LOG: playback cannot be measured');
  const requested = onlyPages ?? (quick ? QUICK_PLAYBACK.pages : meta.pages);
  for (const requestedPage of requested) {
    const page = pageOf(requestedPage);
    if (!meta.pages.includes(page)) { await s.fail('playback', requestedPage, null, 'page missing from the recording'); continue; }
    await s.openPage(page);
    const fixed = fixture && Object.hasOwn(VIEWS, requestedPage) ? VIEWS[requestedPage] : null;
    if (fixed?.view === 'screen') {
      const need = await app.evaluate(`(() => { const grid = document.querySelector('.dash-grid-scroll'); return { height: grid.scrollHeight, viewport: grid.clientHeight }; })()`);
      await app.send('Emulation.setDeviceMetricsOverride', { ...screenMetrics(Math.max(need.height + SHELL_HEADER_HEIGHT, SCREEN.height)), mobile: false });
      await app.evaluate(`new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))`);
    } else if (fixed?.view === 'tall') {
      let height = meta.viewport.height, fitted = false;
      for (let attempt = 0; attempt < 4; attempt++) {
        const before = await app.evaluate(`(() => { const grid = document.querySelector('.dash-grid-scroll'); return { height: grid.scrollHeight, viewport: grid.clientHeight }; })()`);
        height = tallViewportHeight(before.height, height, SHELL_HEADER_HEIGHT);
        await app.send('Emulation.setDeviceMetricsOverride', { width: meta.viewport.width, height,
          deviceScaleFactor: meta.viewport.dpr, mobile: false });
        await app.evaluate(`new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))`);
        const after = await app.evaluate(`(() => { const grid = document.querySelector('.dash-grid-scroll'); return { height: grid.scrollHeight, viewport: grid.clientHeight }; })()`);
        if (after.viewport >= after.height) { fitted = true; break; }
        height = tallViewportHeight(after.height, height, SHELL_HEADER_HEIGHT);
      }
      if (!fitted) throw new Error('the tall viewport did not fit the dashboard grid');
      const tallInventory = await app.evaluate(DASHBOARD_INVENTORY);
      const tallExpected = fixture.pages[page]?.widgets ?? [];
      if (tallExpected.some(expected => !tallInventory.some(widget => widget.id === expected.id)))
        throw new Error('the tall-view inventory does not cover every fixture widget');
    } else if (fixed?.view === 'legible') {
      await app.send('Emulation.setDeviceMetricsOverride', { width: meta.viewport.width, height: meta.viewport.height,
        deviceScaleFactor: meta.viewport.dpr, mobile: false });
      await app.evaluate(`new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))`);
    } else if (fixture) {
      await app.send('Emulation.setDeviceMetricsOverride', { width: meta.viewport.width, height: meta.viewport.height,
        deviceScaleFactor: meta.viewport.dpr, mobile: false });
      await app.evaluate(`new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))`);
    }
    const stops = fixed?.view === 'screen' || fixed?.view === 'tall' ? [0] : await app.evaluate(`(() => {
      const grid = document.querySelector('.dash-grid-scroll');
      const max = Math.max(0, grid.scrollHeight - grid.clientHeight);
      const offsets = [0];
      for (let y = grid.clientHeight; y < max; y += grid.clientHeight) offsets.push(y);
      if (max) offsets.push(max);
      return offsets;
    })()`);
    const pageSpeeds = requestedPage === HEAVIEST ? [1440] : speeds;
    for (const scroll of stops) {
      await app.evaluate(`new Promise(done => { document.querySelector('.dash-grid-scroll').scrollTop = ${scroll};
        requestAnimationFrame(() => requestAnimationFrame(() => done(true))); })`);
      for (const speed of pageSpeeds) {
        let trace = null, perfStopped = false;
        try {
          await setSpeed(speed);
          const seekStart = s.epoch + await app.evaluate('performance.now()');
          await seekTo(0.3);
          await throttle(rate);
          const dated = speed === 1440;
          const pressureBefore = dated && await app.evaluate('window.__perf.pressureMerges()');
          if (dated) await app.evaluate('window.__perf.beginAudit()');
          trace = dated && await startTrace(app, ['disabled-by-default-display.framedisplayed', 'blink.user_timing']);
          const clockMark = trace && await app.evaluate("performance.timeOrigin + performance.mark('dated-playback-clock').startTime");
          await app.evaluate('window.__perf.start()');
          await s.watch();
          const perfContext = await app.evaluate('__e2e.perfContext()');
          const perfScenario = { ...scopeScenario(s.recipe, { ...perfContext, page }, 'play'), speed, scroll };
          const from = s.epoch + await app.evaluate('performance.now()');
          if (dated) { await playbackVisibility(); await s.frames(2); }
          await settledForReplies();
          await replies?.mark('play', { page, speed });
          await watchWidgetOutcomes({ waitForPlaying: true });
          await s.click('button.header-play[aria-label="Play"]');
          await s.frames(2);
          let widgets = await app.evaluate(DASHBOARD_INVENTORY);
          const expectedWidgets = fixture?.pages[page]?.widgets
            ?.filter(expected => requestedPage === HEAVIEST || !widgets.some(widget => widget.id === expected.id) || widgets.some(widget => widget.id === expected.id && widget.visible))
            .map(({ id }) => ({ id }));
          const windowReasons = [];
          if (requestedPage === HEAVIEST) {
            const hidden = (fixture?.pages[page]?.widgets ?? []).filter(expected => !widgets.some(widget => widget.id === expected.id && widget.visible));
            if (hidden.length) windowReasons.push(`${hidden.length} heaviest widgets not visible without scrolling: ${hidden.map(w => w.kind).join(', ')}`);
          }
          if (dated) {
            const line = await s.timeline();
            const endHour = line.value + Math.ceil((PLAY_SECONDS + 0.4) * speed);
            if (endHour > line.max || !await s.until(`${Session.CLOCK_HOUR} >= ${endHour}`, 2 * PLAY_SECONDS * 1000 + 5000))
              windowReasons.push('Playback did not cover the requested game hours');
          } else await sleep(PLAY_SECONDS * 1000);
          const observedOutcomes = await app.evaluate('__e2e.endWidgetOutcomes?.() ?? {}').catch(() => ({}));
          const endInventory = await app.evaluate(DASHBOARD_INVENTORY).catch(() => null);
          widgets = mergeWidgetOutcomes(widgets, observedOutcomes);
          if (Array.isArray(endInventory)) {
            const byId = Object.fromEntries(endInventory.map(w => [w.id, w]));
            widgets = mergeWidgetOutcomes(widgets, byId);
          }
          await pausePlayback();
          await replies?.mark('pause', { page, speed });
          const to = s.epoch + await app.evaluate('performance.now()');
          const playbackReasons = await s.unwatch();
          const visibility = dated ? await app.evaluate('__e2e.endPlaybackVisibility()') : undefined;
          let presented, audit, pressureMerges, inputs, idleCost;
          if (trace) {
            inputs = await playbackInputs();
            const events = await trace();
            trace = null;
            const markEvent = events.find(e => e.name === 'dated-playback-clock');
            const origin = markEvent ? clockMark - markEvent.ts / 1000 : 0;
            presented = markEvent ? [...new Set(events.filter(e => e.name === 'Display::FrameDisplayed').map(e => origin + e.ts / 1000))].sort((a, b) => a - b) : [];
            const firstInput = inputs.actions[0]?.at ?? from;
            inputs.presented = presented.filter(at => at >= firstInput && at <= inputs.to);
            await writeFile(path.join(out, `dated-playback-compositor-${page.replace(/[^\w.-]+/g, '_')}-${scroll}.json.gz`), await promisify(gzip)(JSON.stringify(events)));
            pressureMerges = await app.evaluate('window.__perf.pressureMerges()') - pressureBefore;
            idleCost = await measureIdleCost({
              pausePlayback,
              waitPlayButton: () => s.until("document.querySelector('button.header-play[aria-label=\"Play\"]')", 3_000),
              setSpeed,
              clickPlay: () => s.click('button.header-play[aria-label="Play"]'),
              now: () => app.evaluate('performance.timeOrigin + performance.now()'),
              frames: count => s.frames(count),
              onFailure: idleError => windowReasons.push(`idle cost measurement failed: ${idleError.message}`),
            });
            await throttle(1);
            audit = [];
            let auditDone = false;
            while (!auditDone) {
              const batch = await app.evaluate('window.__perf.auditPicturesBatch(64)', 120_000);
              if (!batch || !Array.isArray(batch.results)) { windowReasons.push('picture audit returned no batch'); break; }
              audit.push(...batch.results); auditDone = batch.done;
            }
          }
          const summary = await app.evaluate('window.__perf.stop()', 120_000);
          perfStopped = true;
          const observedVisibility = visibility ?? inputs?.visibility;
          await throttle(1);
          const allReasons = [...windowReasons, ...playbackReasons, ...(dated ? (inputs?.failures ?? []) : [])];
          await s.record({ area: 'playback', item: `${page}/${speed}`, label: null, pass: true, reasons: allReasons, playbackReasons, from, to, seekStart, speed, page, scroll, visibility: observedVisibility, seconds: PLAY_SECONDS, widgets, expectedWidgets, lastHour: (await s.timeline()).max,
            perf: { family: 'play', context: perfContext, recipe: s.recipe, scenario: scenarioId(perfScenario), page, start: from, settled: to, complete: summary.complete },
            presented, pressureMerges, audit, inputs, idleCost, frames: summary.ui_frames, complete: summary.complete, losses: summary.losses, maps: summary.playback?.maps ?? null, screenshot: false });
        } catch (error) {
          if (trace) await trace().catch(() => {});
          await app.evaluate('__e2e.endWidgetOutcomes?.()').catch(() => {});
          if (await app.evaluate(`!!document.querySelector('button.header-play[aria-label="Pause"]')`).catch(() => false))
            await s.click('button.header-play[aria-label="Pause"]').catch(() => {});
          await s.unwatch().catch(() => {});
          await throttle(1).catch(() => {});
          if (!perfStopped) await app.evaluate('window.__perf.stop()', 120_000).catch(() => {});
          await s.record({ area: 'playback', item: `${page}/${speed}`, label: null, pass: false, reasons: [`playback window failed: ${error.message}`], page, speed, scroll, seconds: PLAY_SECONDS, complete: false, screenshot: false });
        }
      }
    }
    await traceWindow(page);
    await save();
  }
  await setSpeed(1);
});

await phase('widget-costs', async () => {
  if (!await s.holds('window.__perf')) throw new Error('the app was not started with BATTLEPLAN_PERF_LOG: widget-costs cannot be measured');
  if (!costFixture) throw new Error('widget-costs fixture was not prepared');
  const warmEmptyBaseline = async () => {
    const warmupReasons = [];
    await s.openPage(costFixture.empty.label);
    if (!await s.holds(pageShown(costFixture.empty.label))) await s.openPage(costFixture.empty.label, { force: true });
    await throttle(1);
    const preload = await app.evaluate(`(async () => {
      if (typeof window.__perf?.waitPreloadComplete !== 'function') return { supported: false, done: false, remaining: null, total: null };
      const timeout = new Promise(resolve => setTimeout(() => resolve('timeout'), 30_000));
      const outcome = await Promise.race([window.__perf.waitPreloadComplete().then(() => 'done'), timeout]);
      const status = await window.__perf.preloadStatus();
      return { supported: true, outcome, ...status };
    })()`, 35_000);
    if (!preload.supported) warmupReasons.push('widget renderer preload completion hook is unavailable');
    else if (!preload.done) warmupReasons.push(`widget renderer preload did not finish within 30 s (${preload.remaining}/${preload.total} remaining)`);
    await s.frames(4);
    log('widget-cost', `warm-up preload status: ${JSON.stringify(preload)}`);
    if (!await s.holds(speedTaken(1440))) await setSpeed(1440);
    await seekTo(0.3);
    await runThrottled(throttle, rate, async () => {
      await app.evaluate('window.__perf.start()');
      await s.click('button.header-play[aria-label="Play"]');
      await sleep(2000);
      await pausePlayback();
      await s.until("document.querySelector('button.header-play[aria-label=\"Play\"]')", 10_000);
    });
    await app.evaluate('window.__perf.flush()', 60_000).catch(() => {});
    return warmupReasons;
  };
  const measureOne = async ({ kind, view = null, isDefaultView = true, title, label, widget, isEmpty, sampleLabel = null, extraReasons = [] }) => {
    let from = null, to = null, seekStart = null, audit = [], outcome = isEmpty ? 'empty' : 'content', detail = null, surfaces = [], playingAtSnap = null;
    const reasons = [...extraReasons];
    try {
      await s.openPage(label);
      if (!await s.holds(pageShown(label))) await s.openPage(label, { force: true });
      if (!await s.holds(pageShown(label))) throw new Error(`could not switch to ${label}`);
      if (!await s.holds(speedTaken(1440))) await setSpeed(1440);
      seekStart = s.epoch + await app.evaluate('performance.now()');
      await seekTo(0.3);
      await throttle(rate);
      await app.evaluate('window.__perf.beginAudit()');
      await app.evaluate('window.__perf.start()');
      if (!isEmpty) await watchWidgetOutcomes({ waitForPlaying: true });
      await s.click('button.header-play[aria-label="Play"]');
      await s.frames(2);
      from = s.epoch + await app.evaluate('performance.now()');
      await sleep(COST_SECONDS * 1000);
      playingAtSnap = await app.evaluate(`!!document.querySelector('button.header-play[aria-label="Pause"]')`);
      if (!playingAtSnap) reasons.push('clock stopped playing before end of window');
      if (!isEmpty) {
        const observed = await app.evaluate('__e2e.endWidgetOutcomes?.() ?? {}').catch(() => ({}));
        const snap = await app.evaluate(`(() => {
          const classify = ${CLASSIFY_WIDGET};
          const a = document.querySelector('article.dash-widget');
          if (!a) return { outcome: 'error', detail: 'widget element not mounted', surfaces: [] };
          const { outcome, detail } = classify(a);
          const surfaces = [...new Set([...a.querySelectorAll('[data-surface]')].map(c => c.dataset.surface).filter(Boolean))];
          return { outcome, detail, surfaces, id: a.dataset.widget };
        })()`);
        const merged = mergeWidgetOutcomes([{ id: snap.id ?? widget, outcome: snap.outcome, detail: snap.detail }], observed)[0];
        outcome = merged.outcome; detail = merged.detail; surfaces = snap.surfaces;
      }
      to = s.epoch + await app.evaluate('performance.now()');
      await pausePlayback();
      await s.until("document.querySelector('button.header-play[aria-label=\"Play\"]')", 10_000);
      await throttle(1);
      let auditDone = false;
      while (!auditDone) {
        const batch = await app.evaluate('window.__perf.auditPicturesBatch(64)', 120_000);
        if (!batch || !Array.isArray(batch.results)) break;
        audit.push(...batch.results); auditDone = batch.done;
      }
      await app.evaluate('window.__perf.flush()', 60_000);
    } catch (error) {
      outcome = 'error'; detail = error.message; reasons.push(error.message);
      await app.evaluate('__e2e.endWidgetOutcomes?.()').catch(() => {});
      if (await app.evaluate(`!!document.querySelector('button.header-play[aria-label="Pause"]')`).catch(() => false))
        await s.click('button.header-play[aria-label="Pause"]').catch(() => {});
      await throttle(1).catch(() => {});
      await app.evaluate('window.__perf.flush()', 60_000).catch(() => {});
    }
    const itemName = sampleLabel ? `${kind}/${sampleLabel}` : !isDefaultView && view ? `${kind}/${view}` : kind;
    await s.record({ area: 'widget-costs', item: itemName, label, kind, ...(view ? { view } : {}), title, widget, isEmpty, sampleLabel, from, to, seekStart, seconds: COST_SECONDS,
      playingAtSnap, outcome, detail, surfaces, audit, complete: reasons.length === 0, pass: reasons.length === 0, reasons, screenshot: false });
  };
  const warmupReasons = await warmEmptyBaseline();
  await measureOne({ kind: '__empty__', title: 'Empty page (start)', label: costFixture.empty.label, widget: null, isEmpty: true, sampleLabel: 'start', extraReasons: warmupReasons });
  for (let index = 0; index < costFixture.kinds.length; index++) {
    const entry = costFixture.kinds[index];
    log('widget-cost', `${index + 1}/${costFixture.kinds.length}`, !entry.isDefaultView && entry.view ? `${entry.kind}/${entry.view}` : entry.kind);
    await measureOne({ kind: entry.kind, view: entry.view, isDefaultView: entry.isDefaultView, title: entry.title, label: entry.label, widget: entry.widget, isEmpty: false });
    if ((index + 1) % 15 === 0) await save();
  }
  await measureOne({ kind: '__empty__', title: 'Empty page (end)', label: costFixture.empty.label, widget: null, isEmpty: true, sampleLabel: 'end' });
  await app.evaluate('window.__perf.stop()', 120_000);
  await setSpeed(1);
  await save();
});

await phase('catalogue', async () => {
  await s.step('catalogue', 'setup', 'add a page', () => s.click('button[aria-label="Add a page"]'), { done: `document.querySelector('.sidebar-page-list input')` });
  await s.key('Enter');
  await s.until(`!document.querySelector('.sidebar-page-list input')`, 3000);
  if (await app.evaluate(`document.querySelectorAll('article.dash-widget').length`)) throw new Error('a new page is not empty');
  await seekTo(PICTURE_SEEK[1]);
  const editing = `document.querySelector('button[aria-label="Done editing"]')`;
  const notEditing = `document.querySelector('button[aria-label="Edit layout"]')`;
  const catalogueOpen = `document.querySelector('.widget-catalogue:popover-open')`;
  const saveConfirm = '.layout-save-menu:popover-open .layout-save-actions button.primary';
  const removeButton = ':is(.widget-settings-menu, .map-controls-menu):popover-open button[aria-label="Delete widget"]';
  const empty = `document.querySelectorAll('article.dash-widget').length === 0`;
  const dirty = `document.querySelector('.toolbar-save.is-dirty')`;
  const closePopovers = async () => {
    for (let tries = 0; tries < 3 && await s.holds(`document.querySelector(':popover-open:not(.app-tooltip)')`); tries++) await s.escape();
  };
  const readPageState = () => app.evaluate(`({ widgets: document.querySelectorAll('article.dash-widget').length, dirty: !!document.querySelector('.toolbar-save.is-dirty'),
    editing: !!document.querySelector('button[aria-label="Done editing"]'),
    popovers: [...document.querySelectorAll(':popover-open:not(.app-tooltip)')].map(el => el.className?.toString?.().trim() || el.tagName) })`);
  const openSaveConfirm = async (fromEditing = true) => {
    if (await s.holds(visibleControl(saveConfirm))) return;
    await s.click(fromEditing ? 'button[aria-label="Done editing"]' : 'button[aria-label="Save layout"]');
    if (!await s.until(visibleControl(saveConfirm), 3000)) throw new Error('the Save layout confirm button did not become visible within 3 s');
  };
  const leaveEditing = async (stepArea, stepItem, stepLabel, options = {}) => {
    const isDirty = await s.holds(dirty);
    if (isDirty) await openSaveConfirm(true);
    return s.step(stepArea, stepItem, stepLabel, () => s.click(isDirty ? saveConfirm : 'button[aria-label="Done editing"]'), options);
  };
  const recoverCleanPage = async () => {
    for (let tries = 0; tries < 6; tries++) {
      await closePopovers();
      const state = await readPageState();
      if (!describeUncleanCataloguePage(state)) return;
      if (state.widgets > 0) {
        if (!state.editing) { await s.click('button[aria-label="Edit layout"]').catch(() => {}); await s.until(editing, 3000); }
        await s.click('article.dash-widget :is([data-widget-settings], button[aria-label="Map settings"])').catch(() => {});
        if (await s.until(visibleControl(removeButton), 2000)) {
          await s.click(removeButton).catch(() => {});
          await s.until(`document.querySelectorAll('article.dash-widget').length < ${state.widgets}`, 3000);
        } else if (state.dirty) { await closePopovers(); await discardLayout().catch(() => {}); await s.until(notEditing, 3000); }
      } else if (state.dirty) {
        await openSaveConfirm(state.editing).catch(() => {});
        if (await s.holds(visibleControl(saveConfirm))) { await s.click(saveConfirm).catch(() => {}); await s.until(`!(${dirty})`, 3000); }
      }
    }
  };
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
  const selection = quick ? selectQuickCatalogueCards(cards, history) : null;
  if (selection?.source === 'default') log('catalogue', `no matching slowest cards in history: sampling ${selection.indices.size} default cards across the catalogue`);
  for (const [index, card] of cards.entries()) {
    const item = catalogueItemKey(card);
    if (selection && !selection.indices.has(index)) continue;
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
    await closePopovers();
    const map = await s.holds(`document.querySelector('article.dash-widget.kind-map')`);
    if (map && added.widget) {
      try {
        const shown = await leaveEditing('catalogue', item, 'show map',
          { done: `__e2e.mapState(${JSON.stringify(added.widget)}, __e2e.t0Epoch, x => !x.loading && !x.error && x.hour === ${added.hour})`, timeout: 10_000, quiet: 300 });
        shown.drawnAt = await app.evaluate(`__e2e.mapState(${JSON.stringify(added.widget)}, __e2e.t0Epoch, x => !x.loading && !x.error && x.hour === ${added.hour})?.drawnAt ?? null`);
        added.shownFrom = shown.input;
      } catch (error) {
        await s.fail('catalogue', item, 'show map', error.message);
      }
    }
    if (!quick) await visual.capture(`catalogue/${String(index).padStart(3, '0')} ${card.title}`, 'catalogue', 'article.dash-widget');
    const extra = { card: item };
    await finishCatalogueCard(item, {
      cleanup: async () => {
        await closePopovers();
        if (!await app.evaluate(`document.querySelectorAll('article.dash-widget').length`)) return;
        if (map) {
          if (!await s.holds(editing)) await edit(item);
          await s.click('article.dash-widget :is([data-widget-settings], button[aria-label="Map settings"])');
          if (!await s.until(visibleControl(removeButton), 3000)) throw new Error('the Delete widget button did not become visible within 3 s');
          await s.step('catalogue', 'setup', 'delete', () => s.click(removeButton), { done: empty, extra });
          await leaveEditing('catalogue', 'setup', 'done', { done: `${empty} && ${notEditing}`, extra });
        } else {
          let offered = false;
          if (!s.everFailed('catalogue', item, 'discard')) {
            await s.click('article.dash-widget [data-widget-settings], article.dash-widget button[aria-label="Map settings"]');
            offered = await s.until(visibleControl(removeButton), 3000);
            if (!offered) await closePopovers();
          }
          if (offered) await s.step('catalogue', 'setup', 'delete', () => s.click(removeButton), { done: `${empty} && ${editing}`, extra });
          else await s.step('catalogue', 'setup', 'discard', () => discardLayout(),
            { from: 'last', done: `${empty} && ${notEditing}`, extra });
        }
      },
      readState: readPageState,
      recover: recoverCleanPage,
      fail: (cardItem, label, reason) => s.fail('catalogue', cardItem, label, reason),
    });
    if ((index + 1) % 10 === 0) await save();
  }
  if (await s.holds(editing)) await leaveEditing('catalogue', 'setup', 'done', { done: `${empty} && ${notEditing}` });
});

await phase('controls', async () => {
  s.native = new NativeInput(desktop, appPid, input);
  s.work = work;
  try { await controls(); } finally { s.native.close(); s.native = null; }
});

async function controls() {
  for (const control of CASES) {
    if (only && !only.includes(control.id)) continue;
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
      if (await s.holds(`document.querySelector('button[aria-label="Done editing"]')`)) await discardLayout().catch(() => {});
      await save();
    }
  }
}

try { await app.evaluate('window.__perf.stop()', 120_000); } catch (error) { await s.fail('measurement', 'finish', null, `measurement did not finish: ${error.message}`); }
if (replies) {
  meta.replies = { file: recordReplies, counts: await replies.finish() };
  log('replies recorded', JSON.stringify(meta.replies.counts), 'to', recordReplies);
}
meta.finished = new Date().toISOString();
meta.seconds = (Date.now() - started.getTime()) / 1000;
await save();
log('sweep done', Math.round(meta.seconds), 's,', s.results.filter(r => !r.pass).length, 'failed of', s.results.length);
app.close();
