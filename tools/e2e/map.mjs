import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, unlink } from 'node:fs/promises';
import path from 'node:path';
import readline from 'node:readline';
import { fileURLToPath } from 'node:url';
import { log, sleep } from './cdp.mjs';
import { Session } from './session.mjs';

export const HOVER_CASES = [
  { id: 'division counter', field: 'hover', match: value => /^division:/.test(value ?? '') },
  { id: 'province tooltip', field: 'tooltip', match: value => /^province:/.test(value ?? '') },
  { id: 'map GUI button', field: 'hover', match: value => /^button:/.test(value ?? '') },
];

export const MAP_INTERACTIONS = [
  { id: 'zoom: continuous', where: 'widget', run: async c => { await c.wheel(12, 16); await c.wheel(-12, 16); } },
  { id: 'zoom: wheel bursts', where: 'widget', run: async c => {
    for (const notches of [5, -5, 5, -5]) { await c.wheel(notches, 8); await sleep(250); } } },
  { id: 'pan: arrow keys', where: 'widget', run: async c => { await c.keys(['ArrowRight'], 1500); await c.keys(['ArrowLeft'], 1500); } },
  { id: 'pan: middle drag', where: 'widget', run: async c => { await c.drag(-250, 0); await c.drag(250, 0); } },
  { id: 'playback', where: 'widget', run: async () => { await sleep(4000); } },
  { id: 'playback', where: 'campaign', camera: 'Map', run: async () => { await sleep(4000); } },
  { id: 'fly', where: 'campaign', camera: 'Fly', moves: true, run: async c => { await c.click(); await sleep(500); await c.keys(['W'], 2000); await c.keys(['W', 'D'], 1000); await c.keys(['Escape'], 50); } },
  { id: 'walk', where: 'campaign', camera: 'Walk', moves: true, run: async c => { await c.click(); await sleep(500); await c.keys(['W'], 2000); await c.keys(['W', 'A'], 1000); await c.keys(['Escape'], 50); } },
];

export class NativeInput {
  constructor(desktop, appPid, input) {
    this.process = spawn('python', [fileURLToPath(new URL('./native_input.py', import.meta.url)), '--desktop', desktop, '--app-pid', String(appPid), '--input', input],
      { stdio: ['pipe', 'pipe', 'inherit'], windowsHide: true });
    this.replies = readline.createInterface({ input: this.process.stdout })[Symbol.asyncIterator]();
  }

  async ask(command) {
    this.process.stdin.write(`${JSON.stringify(command)}\n`);
    const { value, done } = await this.replies.next();
    if (done) throw new Error('the native input helper exited');
    const reply = JSON.parse(value);
    if (reply.error) throw new Error(reply.error);
    return reply;
  }

  close() { this.process.stdin.end(); }
}

async function mapArea(s, dpr) {
  const area = await s.app.evaluate(`(() => { const a = document.querySelector('article.dash-widget.kind-map'); if (!a) return null;
    a.scrollIntoView({ block: 'nearest' }); const r = a.querySelector('.dash-widget-content').getBoundingClientRect();
    return { view: a.dataset.widget, left: r.left + 12, top: r.top + 12, right: r.right - 12, bottom: r.bottom - 12 }; })()`);
  if (!area) throw new Error('the Overview page has no map widget');
  return { ...area, px: (x, y) => [x * dpr, y * dpr] };
}

const stateAfter = (view, after, test) => `__e2e.mapState(${JSON.stringify(view)}, ${after}, ${test})`;

async function drawnState(s, view, after, test, timeout) {
  for (const end = Date.now() + timeout; Date.now() < end; await sleep(16)) {
    const state = await s.app.evaluate(stateAfter(view, after, test));
    if (state) return state;
  }
  return null;
}

export async function mapModes(s, dpr) {
  const { view } = await mapArea(s, dpr);
  const modes = await s.app.evaluate(`__e2e.mapModes[${JSON.stringify(view)}] ?? null`);
  if (!modes?.length) throw new Error('the viewer reported no modes for the map');
  const views = s.app.lastSent('set_dashboard_maps')?.args.views ?? null;
  if (!views?.some(entry => entry.id === view)) throw new Error('the app has sent no layout with the map in it');
  const send = list => s.app.evaluate(`window.__TAURI_INTERNALS__.invoke('set_dashboard_maps', { views: ${JSON.stringify(list)} }).then(() => true)`);
  const active = await s.app.evaluate(`__e2e.mapStates.filter(x => x.id === ${JSON.stringify(view)} && x.mapMode).at(-1)?.mapMode ?? null`);
  const ordered = [...modes.filter(mode => mode.token !== active), ...modes.filter(mode => mode.token === active)];
  for (const mode of ordered) {
    const item = `mode: ${mode.token}`;
    if (mode.unavailable) { await s.record({ area: 'map', item, label: mode.label, pass: false, reasons: [`the recording cannot draw it: ${mode.unavailable}`], ms: null, unavailable: mode.unavailable, screenshot: false }); continue; }
    await s.watch();
    const sent = Date.now();
    await send(views.map(entry => entry.id === view ? { ...entry, options: { ...entry.options, mapMode: mode.token } } : entry));
    const state = await drawnState(s, view, sent, `x => x.mapMode === ${JSON.stringify(mode.token)} && !x.loading`, 8000);
    const ms = state ? state.drawnAt - sent : null;
    const reasons = [...(state ? [] : ['the viewer never drew a frame in this mode within 8 s']), ...await s.unwatch()];
    await s.record({ area: 'map', item, label: mode.label, ms, sent, drawnAt: state?.drawnAt ?? null, pass: true, reasons, screenshot: false });
  }
  await send(views);
}

async function campaignArea(s, dpr) {
  const area = await s.app.evaluate(`(() => { const r = document.querySelector('.map-panel:not([hidden])')?.getBoundingClientRect(); if (!r) return null;
    return { left: r.left + 12, top: r.top + 12, right: r.right - 12, bottom: r.bottom - 12 }; })()`);
  if (!area) throw new Error('no campaign map area');
  return { ...area, view: null, px: (x, y) => [x * dpr, y * dpr] };
}

export async function interactions(s, input, dpr, speedIndex, { label = null, only = null } = {}) {
  for (const where of ['widget', 'campaign']) {
    const cases = MAP_INTERACTIONS.filter(entry => entry.where === where && (only === null || only.includes(entry.id)));
    if (!cases.length) continue;
    if (where === 'campaign') { await s.click('#tab-map'); await sleep(3000); } else await s.openPage('Overview');
    const area = where === 'campaign' ? await campaignArea(s, dpr) : await mapArea(s, dpr);
    const [cx, cy] = area.px((area.left + area.right) / 2, (area.top + area.bottom) / 2);
    const c = {
      wheel: (notches, interval) => input.ask({ op: 'wheel', x: cx, y: cy, notches, interval_ms: interval }),
      keys: (keys, ms) => input.ask({ op: 'keys', x: cx, y: cy, keys, ms }),
      click: () => input.ask({ op: 'click', x: cx, y: cy }),
      drag: (dx, dy) => input.ask({ op: 'drag', interval_ms: 16, points: Array.from({ length: 21 }, (_, i) => [cx + dx * dpr * i / 20, cy + dy * dpr * i / 20]) }),
    };
    for (const playing of [false, true]) {
      if (playing) {
        await s.click('.header-speed');
        await s.until(`document.querySelector('.playback-speed-menu:popover-open .playback-speed-presets button')`, 3000);
        await s.click('.playback-speed-menu:popover-open .playback-speed-presets button', { index: speedIndex });
        const line = await s.timeline();
        await s.app.click(Session.timelineX(line, 0.1), line.y);
        await sleep(1000);
        await s.click('button[aria-label="Play recording"]');
        await sleep(1000);
      }
      for (const entry of cases) {
        if (entry.id === 'playback' && !playing) continue;
        const item = `interaction: ${entry.id}${where === 'campaign' && !entry.moves ? ' (campaign map)' : ''}`;
        const while_ = `${label ? `${label}, ` : ''}${playing ? 'playing' : 'paused'}`;
        try {
          if (entry.camera) { await s.click('.segmented[aria-label="Camera"] button', { text: entry.camera }); await sleep(1500); }
          await s.watch();
          const from = Date.now();
          await entry.run(c);
          await sleep(300);
          const to = Date.now();
          const flashed = await s.unwatch();
          const drawn = area.view === null ? null : await s.app.evaluate(`(() => { const states = __e2e.mapStates.filter(x => x.id === ${JSON.stringify(area.view)} && x.drawnAt >= ${from} && x.drawnAt <= ${to});
            return { states: states.length, loading: states.some(x => x.loading), zooms: new Set(states.filter(x => x.zoom != null).map(x => x.zoom)).size,
              centres: new Set(states.filter(x => x.center).map(x => x.center.join())).size }; })()`);
          const reasons = [...flashed];
          const moves = entry.id.startsWith('zoom') ? 'zooms' : entry.id.startsWith('pan') ? 'centres' : null;
          if (entry.moves) reasons.push('the full map reports no camera, so its movement is unconfirmed');
          else if (moves && drawn?.states === 0) reasons.push('the map acknowledged no frame during it: it was still loading or had stalled');
          else if (moves && drawn[moves] < 2) reasons.push(`the map did not ${moves === 'zooms' ? 'zoom' : 'pan'} (${drawn[moves]} ${moves === 'zooms' ? 'zoom levels' : 'camera centres'} drawn${drawn.loading ? ', while loading' : ''})`);
          const zooms = drawn?.zooms ?? null;
          await s.record({ area: label ? 'mapfps' : 'map', item, label: while_, pass: true, reasons, from, to, view: area.view, zoomLevels: zooms, screenshot: false });
        } catch (error) {
          await s.fail(label ? 'mapfps' : 'map', item, while_, `could not send its input: ${error.message}`);
        } finally {
          if (entry.moves) {
            await s.click('.segmented[aria-label="Camera"] button', { text: 'Map' })
              .catch(error => s.fail(label ? 'mapfps' : 'map', item, while_, `the map camera could not be chosen again after it: ${error.message}`));
            await sleep(500);
          }
        }
      }
      if (playing) {
        await s.click('button[aria-label="Pause playback"]');
        await s.click('.header-speed');
        await s.until(`document.querySelector('.playback-speed-menu:popover-open .playback-speed-presets button')`, 3000);
        await s.click('.playback-speed-menu:popover-open .playback-speed-presets button', { index: 1 });
      }
    }
  }
  await s.openPage('Overview');
}

export async function hover(s, input, dpr) {
  const area = await mapArea(s, dpr);
  const from = Date.now();
  const seen = new Map();
  const at = async (x, y) => {
    const [px, py] = area.px(x, y);
    const [posted] = (await input.ask({ op: 'move', points: [[px, py]], interval_ms: 0 })).posted;
    const state = await drawnState(s, area.view, posted, 'x => true', 400);
    return { posted, state, point: [px, py] };
  };
  const points = [];
  for (let row = 0; row < 12; row++) for (let column = 0; column < 20; column++)
    points.push([area.left + (area.right - area.left) * (column + 0.5) / 20, area.top + (area.bottom - area.top) * (row + 0.5) / 12]);
  for (let y = area.bottom - 4; y > area.bottom - 140; y -= 12) for (let x = area.left + 4; x < area.right; x += 12) points.push([x, y]);
  let neutral = null;
  for (const [x, y] of points) {
    if (HOVER_CASES.every(hoverCase => seen.has(hoverCase.id)) && neutral) break;
    const { state, point } = await at(x, y);
    if (state && !state.hover && !state.tooltip && !neutral) {
      await sleep((state.tooltipDelayMs ?? 0) + 200);
      const now = await s.app.evaluate(`__e2e.mapStates.filter(x => x.id === ${JSON.stringify(area.view)}).at(-1) ?? null`);
      if (now && !now.hover && !now.tooltip) neutral = [x, y];
    }
    for (const hoverCase of HOVER_CASES) if (!seen.has(hoverCase.id) && hoverCase.match(state?.[hoverCase.field] ?? null))
      seen.set(hoverCase.id, { x, y, hovered: state[hoverCase.field], point });
  }
  for (const hoverCase of HOVER_CASES) {
    const target = seen.get(hoverCase.id);
    if (!target) { await s.fail('map', `hover: ${hoverCase.id}`, null, `no ${hoverCase.id} under any of ${points.length} points over the map`); continue; }
    if (!neutral) { await s.fail('map', `hover: ${hoverCase.id}`, null, `no point over the map drew without a highlight or tooltip, so its appearance cannot be timed`); continue; }
    const cleared = await at(...neutral);
    if (!cleared.state || cleared.state.hover || cleared.state.tooltip) {
      const gone = await drawnState(s, area.view, cleared.posted, 'x => !x.hover && !x.tooltip', 4000);
      if (!gone) { await s.fail('map', `hover: ${hoverCase.id}`, null, 'its highlight or tooltip never cleared away from it, so its appearance cannot be timed'); continue; }
    }
    const [px, py] = target.point;
    await s.watch();
    const [posted] = (await input.ask({ op: 'move', points: [[px, py]], interval_ms: 0 })).posted;
    const state = await drawnState(s, area.view, posted, `x => x.${hoverCase.field} === ${JSON.stringify(target.hovered)}`, 4000);
    const flashed = await s.unwatch();
    const delay = hoverCase.field === 'tooltip' ? state?.tooltipDelayMs ?? null : 0;
    const ms = state ? state.drawnAt - posted - (delay ?? 0) : null;
    const reasons = !state ? [`never ${hoverCase.field === 'tooltip' ? 'showed its tooltip' : 'highlighted'} within 4 s`]
      : delay === null ? ['the viewer reported no tooltip delay'] : [];
    reasons.push(...flashed);
    await s.record({ area: 'map', item: `hover: ${hoverCase.id}`, label: target.hovered, ms, posted, delay, drawnAt: state?.drawnAt ?? null, pass: true, reasons, screenshot: false });
  }
  await input.ask({ op: 'move', points: [area.px(area.left + 2, area.top + 2)], interval_ms: 0 }).catch(() => {});
  await s.record({ area: 'map', item: 'interaction: hover', label: 'paused', pass: true, reasons: [], from, to: Date.now(), screenshot: false });
}

export async function mapPictures(s, input, visual, dpr, work, seeks) {
  const area = await mapArea(s, dpr);
  const exportAs = caseId => exportPicture(s, input, visual, work, caseId);
  for (const [label, fraction] of seeks) {
    const line = await s.timeline();
    const before = Date.now();
    await s.app.click(Session.timelineX(line, fraction), line.y);
    const drawn = await drawnState(s, area.view, before, 'x => !x.loading && x.hour != null', 15_000);
    if (!drawn) { visual.fail(`map/default camera/${label}`, 'map', 'the map never drew the sought hour'); continue; }
    await exportAs(`map/default camera/${label}`);
  }
  log('map pictures', visual.items.filter(item => item.kind === 'map').length);
}

export function zoomedMapPicture(visual, [label]) {
  visual.fail(`map/zoomed camera/${label}`, 'map', "no repeatable close camera: wheel zoom moves by frame time, as the game's does, and no command sets the camera");
}

async function exportPicture(s, input, visual, work, caseId) {
  const folder = path.join(work, 'map-exports');
  await mkdir(folder, { recursive: true });
  const file = path.join(folder, `${caseId.replace(/[^\w.-]+/g, '_')}.png`);
  if (existsSync(file)) await unlink(file);
  await s.click('article.dash-widget.kind-map button[aria-label="Map settings"]');
  await sleep(150);
  await s.click('.map-controls-menu:popover-open .map-menu-action', { text: 'Export map', partial: true });
  await input.ask({ op: 'save_dialog', path: file, seconds: 20 });
  for (const end = Date.now() + 20_000; !existsSync(file) && Date.now() < end;) await sleep(100);
  await sleep(300);
  if (!existsSync(file)) { visual.fail(caseId, 'map', 'the export wrote no image within 20 s'); return; }
  await visual.addFile(caseId, 'map', file);
  await s.escape();
}
