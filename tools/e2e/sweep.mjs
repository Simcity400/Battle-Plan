// The acceptance sweep over one running app with a prepared recording open: every page cold and warm,
// seeks on every page, pictures at a fixed hour, the map, playback, every catalogue view and every control case.
// usage: node tools/e2e/sweep.mjs --port <port> --out <dir> --work <dir> --session <name> --desktop <name> --app-pid <pid> --input <address>
//   [--phases pages,seeks,visual,map,mapfps,playback,catalogue,controls] [--cases id,...]
// One app session: its results go to sweep-<session>.json, its pictures to visual/manifest-<session>.json.
import { writeFile } from 'node:fs/promises';
import path from 'node:path';
import { connect, log, sleep } from './cdp.mjs';
import { Session } from './session.mjs';
import { CASES } from './controls.mjs';
import { Visual } from './visual.mjs';
import { NativeInput, hover, interactions, mapModes, mapPictures, zoomedMapPicture } from './map.mjs';

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
const phases = flag('--phases')?.split(',') ?? PHASES;
const only = flag('--cases')?.split(',') ?? null;
/** Early, mid-war and late: fixed shares of the fixture's span. */
export const SEEKS = [['early', 0.05], ['mid-war', 0.5], ['late', 0.95]];
/** How far apart each page's seek hours are, as a share of the span: no two pages seek the same hour. */
const SEEK_STEP = 0.004;
/** The hour pictures are taken at: mid-war. */
const PICTURE_SEEK = SEEKS[1];
/** Playback speeds measured, in game hours per second (presets of the speed menu). */
export const SPEEDS = [24, 1440];
const PRESETS = [0.25, 1, 4, 24, 168, 720, 1440];
const PLAY_SECONDS = 8;
/** The page catalogue views are added to, emptied first. */
const CATALOGUE_PAGE = 'Intelligence';

const app = await connect(port);
// From the start, so the map's first states and mode lists are seen.
await app.evaluate('__e2e.listenMaps()');
const s = new Session(app, out, session);
await s.init();
const visual = new Visual(app, out, session);
await visual.init();
const started = new Date();
const meta = { session, started: started.toISOString(), port, phases, epoch: s.epoch, pages: [], viewport: null, cataloguePage: CATALOGUE_PAGE, speeds: SPEEDS, seeks: SEEKS, pictureSeek: PICTURE_SEEK };
const save = async () => {
  const results = s.results.map(({ probe, ...result }) => ({ ...result, session }));
  await writeFile(path.join(out, `sweep-${session}.json`), JSON.stringify({ meta, results, console: app.messages.map(message => ({ ...message, session })) }, null, 1));
  await visual.finish({ pictureSeek: PICTURE_SEEK, viewport: meta.viewport });
};
/** Seeks to `fraction` without measuring it, and waits for the page to settle. */
const seekTo = async fraction => {
  const line = await s.timeline();
  await app.click(Session.timelineX(line, fraction), line.y);
  await app.evaluate(`(__e2e.mark(null), __e2e.settle(15000, 300))`, 60_000);
};
const setSpeed = async speed => {
  await s.click('.header-speed');
  await sleep(150);
  await s.click('.playback-speed-menu:popover-open .playback-speed-presets button', { index: PRESETS.indexOf(speed) });
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

async function phase(name, run) {
  if (!phases.includes(name)) return;
  log('phase', name);
  const began = Date.now();
  try { await run(); } catch (error) { await s.fail(name, 'phase', null, `stopped: ${error.stack ?? error.message}`); }
  meta[`${name}Seconds`] = (Date.now() - began) / 1000;
  await s.escape().catch(() => {});
  await save();
}

/** A page visit: the switch, then a wheel turn at a time to the bottom; each step measures the widgets it showed. */
async function visit(pass, page) {
  if (meta.startPage === page && pass === 'cold')
    await s.fail('pages', `cold/${page}`, 'switch', 'shown at startup before any input: its cold visit cannot be measured from input');
  const check = pass === 'cold';
  await s.step('pages', `${pass}/${page}`, 'switch', () => s.click('.sidebar-page-list button.sidebar-subrow', { text: page }), { timeout: 8000, quiet: 300, check });
  for (let turn = 1; ; turn++) {
    const grid = await app.evaluate(`(({ scrollTop, clientHeight, scrollHeight }) => ({ scrollTop, clientHeight, scrollHeight }))(document.querySelector('.dash-grid-scroll'))`);
    if (grid.scrollTop + grid.clientHeight >= grid.scrollHeight - 1) break;
    if (turn > 30) { await s.fail('pages', `${pass}/${page}`, 'scroll', 'still not at the bottom after 30 wheel turns'); break; }
    await s.step('pages', `${pass}/${page}`, `scroll ${turn}`, () => s.wheel(Math.round(grid.clientHeight * 0.9)), { timeout: 8000, quiet: 300, check });
  }
  await app.evaluate(`document.querySelector('.dash-grid-scroll').scrollTo({ top: 0 })`);
}

await phase('pages', async () => {
  for (const pass of ['cold', 'warm']) for (const page of meta.pages) { await visit(pass, page); await save(); }
});

await phase('seeks', async () => {
  for (const [index, page] of meta.pages.entries()) {
    await s.openPage(page);
    // Each page seeks hours of its own, so every cold seek is that hour's first visit in this fresh session; then the same
    // hours again (warm).
    for (const pass of ['cold', 'warm']) for (const [label, base] of SEEKS) {
      const fraction = base + index * SEEK_STEP;
      const line = await s.timeline();
      const result = await s.step('seeks', page, pass === 'cold' ? label : `${label} warm`, async () => {
        await app.click(Session.timelineX(line, fraction), line.y);
        return { fraction, pass };
      }, { timeout: 10_000, quiet: 300, check: pass === 'cold' });
      result.hour = (await s.timeline()).value;
      const shown = (await app.evaluate('__e2e.widgets()')).filter(widget => widget.visible);
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
      await app.evaluate(`(__e2e.mark(null), __e2e.settle(10000, 300))`, 60_000);
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
    await mapModes(s, meta.viewport.dpr);
    await interactions(s, native, meta.viewport.dpr, PRESETS.indexOf(SPEEDS.at(-1)));
    await hover(s, native, meta.viewport.dpr);
  } finally { native.close(); }
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
  try { await interactions(s, native, meta.viewport.dpr, PRESETS.indexOf(SPEEDS.at(-1)), { label: 'uncapped' }); } finally { native.close(); }
});

await phase('playback', async () => {
  if (!await s.holds('window.__perf')) throw new Error('the app was not started with BATTLE_PLAN_PERF_LOG: playback cannot be measured');
  for (const page of meta.pages) {
    await s.openPage(page);
    const widgets = await app.evaluate(`(() => { const view = document.querySelector('.dash-grid-scroll').getBoundingClientRect();
      return [...document.querySelectorAll('article.dash-widget')].map(a => { const r = a.getBoundingClientRect();
        return { id: a.dataset.widget, kind: [...a.classList].find(c => c.startsWith('kind-'))?.slice(5), title: a.getAttribute('aria-label'),
          visible: r.bottom > view.top && r.top < view.bottom && r.height > 0 }; }); })()`);
    for (const speed of SPEEDS) {
      await setSpeed(speed);
      await seekTo(0.3);
      await app.evaluate('window.__perf.start()');
      await s.watch();
      const from = s.epoch + await app.evaluate('performance.now()');
      await s.click('button[aria-label="Play recording"]');
      await sleep(PLAY_SECONDS * 1000);
      await s.click('button[aria-label="Pause playback"]');
      const to = s.epoch + await app.evaluate('performance.now()');
      const summary = await app.evaluate('window.__perf.stop()', 120_000);
      await s.record({ area: 'playback', item: `${page}/${speed}`, label: null, pass: true, reasons: await s.unwatch(), from, to, speed, page, widgets, lastHour: (await s.timeline()).max,
        frames: summary.ui_frames, complete: summary.complete, losses: summary.losses, maps: summary.playback?.maps ?? null, screenshot: false });
    }
    await save();
  }
  await setSpeed(1);
  if (phases.includes('visual')) zoomedMapPicture(visual, SEEKS.at(-1));
});

await phase('catalogue', async () => {
  // A new page starts empty, so each card lands alone, at the top, in view.
  await s.step('catalogue', 'setup', 'add a page', () => s.click('button[aria-label="Add a page"]'), { done: `document.querySelector('.sidebar-page-list input')` });
  await s.key('Enter');
  await sleep(300);
  if (await app.evaluate(`document.querySelectorAll('article.dash-widget').length`)) throw new Error('a new page is not empty');
  await seekTo(PICTURE_SEEK[1]);
  const edit = () => s.step('catalogue', 'setup', 'edit', () => s.click('button[aria-label="Edit layout"]'), { done: `document.querySelector('button[aria-label="Done arranging"]')` });
  const open = () => s.step('catalogue', 'setup', 'open catalogue', () => s.click('button[aria-label="Add widget"]'), { done: `document.querySelector('.widget-catalogue:popover-open')` });
  await edit();
  await open();
  const cards = await app.evaluate(`[...document.querySelectorAll('.widget-catalogue:popover-open .widget-card')].map(c => ({
    title: c.querySelector('.widget-card-text strong')?.textContent.trim(), view: c.hasAttribute('data-variant'),
    unavailable: c.querySelector('.widget-card-add')?.disabled ? (c.querySelector('.widget-card-text small')?.textContent.trim() || 'cannot be added') : null }))`);
  meta.catalogue = cards;
  for (const [index, card] of cards.entries()) {
    const item = `${card.view ? 'view' : 'widget'}/${card.title}`;
    if (card.unavailable) { await s.fail('catalogue', item, 'add', `cannot be added to this recording: ${card.unavailable}`); continue; }
    if (!await s.holds(`document.querySelector('button[aria-label="Done arranging"]')`)) await edit();
    if (!await s.holds(`document.querySelector('.widget-catalogue:popover-open')`)) await open();
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
    await visual.capture(`catalogue/${String(index).padStart(3, '0')} ${card.title}`, 'catalogue', 'article.dash-widget');
    if (map) {
      // A widget is deleted only while arranging; Done then keeps the page empty for the next card.
      if (!await s.holds(`document.querySelector('button[aria-label="Done arranging"]')`)) await edit();
      await s.click('article.dash-widget button[aria-label="Map settings"]');
      await sleep(150);
      await s.step('catalogue', 'setup', 'delete', () => s.click('.map-controls-menu:popover-open button[aria-label="Delete widget"]'),
        { done: `document.querySelectorAll('article.dash-widget').length === 0` });
      await s.step('catalogue', 'setup', 'done', () => s.click('button[aria-label="Done arranging"]'),
        { done: `document.querySelectorAll('article.dash-widget').length === 0 && document.querySelector('button[aria-label="Edit layout"]')` });
    } else await s.step('catalogue', 'setup', 'discard', () => s.click('button[aria-label="Cancel layout changes"]'),
      { done: `document.querySelectorAll('article.dash-widget').length === 0 && document.querySelector('button[aria-label="Edit layout"]')` });
    if ((index + 1) % 10 === 0) await save();
  }
});

await phase('controls', async () => {
  for (const control of CASES) {
    if (only && !only.includes(control.id)) continue;
    for (const page of control.page === '*' ? meta.pages : control.page === null ? [await s.currentPage()] : [control.page]) {
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
});

await app.evaluate('window.__perf?.flush()').catch(() => {});
meta.finished = new Date().toISOString();
meta.seconds = (Date.now() - started.getTime()) / 1000;
await save();
log('sweep done', Math.round(meta.seconds), 's,', s.results.filter(r => !r.pass).length, 'failed of', s.results.length);
app.close();
