import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, readFile, unlink } from 'node:fs/promises';
import path from 'node:path';
import readline from 'node:readline';
import { fileURLToPath } from 'node:url';
import { crc32, inflateSync } from 'node:zlib';
import { PNG } from 'pngjs';
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
  const end = Date.now() + timeout;
  while (true) {
    const state = await s.app.evaluate(stateAfter(view, after, test));
    if (state) return state;
    if (Date.now() >= end) return null;
    await sleep(16);
  }
}

export async function mapModes(s, dpr) {
  const { view } = await mapArea(s, dpr);
  const modes = await s.app.evaluate(`__e2e.mapModes[${JSON.stringify(view)}] ?? null`);
  if (!modes?.length) throw new Error('the viewer reported no modes for the map');
  const last = s.app.lastSent('set_dashboard_maps')?.args ?? null;
  const views = last?.views ?? null;
  if (!last?.clockId || !views?.some(entry => entry.id === view)) throw new Error('the app has sent no layout with the map in it');
  const send = list => s.app.evaluate(`window.__TAURI_INTERNALS__.invoke('set_dashboard_maps', ${JSON.stringify({ ...last, views: list })}).then(() => true)`);
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
        await s.click('button.header-play[aria-label="Play"]');
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
        await s.click('button.header-play[aria-label="Pause"]');
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

export const CLOSE_CAMERA_ZOOM_SHARE = 0.06;
export const CLOSE_CAMERA_MIN_HEIGHT = 120;
export const CLOSE_CAMERA_TOLERANCE = 0.5;
const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const PNG_SAMPLES = new Map([[0, 1], [2, 3], [3, 1], [4, 2], [6, 4]]);
const ADAM7_PASSES = [[0, 0, 8, 8], [4, 0, 8, 8], [0, 4, 4, 8], [2, 0, 4, 4], [0, 2, 2, 4], [1, 0, 2, 2], [0, 1, 1, 2]];

function expectedPngScanlineBytes(width, height, bitDepth, colorType, interlace) {
  const samples = PNG_SAMPLES.get(colorType);
  if (!samples || bitDepth <= 0) throw new Error(`unsupported PNG format (colorType=${colorType}, bitDepth=${bitDepth})`);
  const rowBytes = cols => 1 + Math.ceil((cols * samples * bitDepth) / 8);
  if (interlace === 0) return height * rowBytes(width);
  if (interlace !== 1) throw new Error(`unsupported PNG interlace method (${interlace})`);
  let total = 0;
  for (const [x0, y0, dx, dy] of ADAM7_PASSES) {
    const pw = Math.ceil((width - x0) / dx);
    const ph = Math.ceil((height - y0) / dy);
    if (pw > 0 && ph > 0) total += ph * rowBytes(pw);
  }
  return total;
}

export async function assertValidPng(filePath) {
  const bytes = await readFile(filePath);
  if (bytes.length < PNG_SIGNATURE.length + 12 || !bytes.subarray(0, 8).equals(PNG_SIGNATURE)) {
    throw new Error('file does not start with a valid PNG signature');
  }
  let offset = 8;
  let ihdr = null;
  let seenIend = false;
  const idatParts = [];
  while (offset < bytes.length) {
    if (offset + 12 > bytes.length) throw new Error('PNG chunk header is truncated');
    const length = bytes.readUInt32BE(offset);
    const type = bytes.toString('ascii', offset + 4, offset + 8);
    const dataStart = offset + 8;
    const dataEnd = dataStart + length;
    if (dataEnd + 4 > bytes.length) throw new Error(`PNG chunk ${type} is truncated`);
    const expectedCrc = crc32(bytes.subarray(offset + 4, dataEnd)) >>> 0;
    if (expectedCrc !== bytes.readUInt32BE(dataEnd)) throw new Error(`PNG chunk ${type} failed CRC verification`);
    if (!ihdr) {
      if (type !== 'IHDR' || length !== 13) throw new Error('PNG does not begin with a valid IHDR chunk');
      ihdr = {
        width: bytes.readUInt32BE(dataStart),
        height: bytes.readUInt32BE(dataStart + 4),
        bitDepth: bytes[dataStart + 8],
        colorType: bytes[dataStart + 9],
        interlace: bytes[dataStart + 12],
      };
      if (ihdr.width === 0 || ihdr.height === 0) throw new Error('PNG IHDR has zero dimensions');
    } else if (type === 'IDAT') {
      idatParts.push(bytes.subarray(dataStart, dataEnd));
    } else if (type === 'IEND') {
      if (length !== 0) throw new Error('PNG IEND chunk is non-empty');
      seenIend = true;
      offset = dataEnd + 4;
      break;
    }
    offset = dataEnd + 4;
  }
  if (!ihdr || !seenIend || offset !== bytes.length) {
    throw new Error('PNG is incomplete or missing a terminal IEND chunk');
  }
  if (!idatParts.length) throw new Error('PNG contains no IDAT image data');
  const inflated = inflateSync(Buffer.concat(idatParts));
  const needed = expectedPngScanlineBytes(ihdr.width, ihdr.height, ihdr.bitDepth, ihdr.colorType, ihdr.interlace);
  if (inflated.length !== needed) {
    throw new Error(`PNG scanline payload length ${inflated.length} does not match expected ${needed} bytes`);
  }
  const decoded = PNG.sync.read(bytes);
  if (decoded.width !== ihdr.width || decoded.height !== ihdr.height || decoded.data.length !== ihdr.width * ihdr.height * 4) {
    throw new Error('PNG decode mismatch');
  }
  return { width: decoded.width, height: decoded.height };
}

export function deriveCloseCameraPose(state) {
  if (!Array.isArray(state?.center) || state.center.length !== 2 || !state.center.every(Number.isFinite)
    || !Number.isFinite(state?.zoom) || state.zoom <= 0) {
    throw new Error('the map reported no default camera center and zoom to derive a close camera pose from');
  }
  const height = Math.round(Math.min(state.zoom * 0.5, Math.max(CLOSE_CAMERA_MIN_HEIGHT, state.zoom * CLOSE_CAMERA_ZOOM_SHARE)) * 100) / 100;
  const center = [Math.round(state.center[0] * 100) / 100, Math.round(state.center[1] * 100) / 100];
  return { center, height };
}

function resolveCameraReply(reply, fallback = null) {
  if (Array.isArray(reply?.center) && reply.center.length === 2 && reply.center.every(Number.isFinite)
    && Number.isFinite(reply?.height) && reply.height > 0) {
    return { center: [reply.center[0], reply.center[1]], height: reply.height };
  }
  if (fallback) return fallback;
  throw new Error('the camera command returned no resolved center and height');
}

export function cameraPoseMatches(state, pose, hour = null, tolerance = 0.5) {
  if (!state || state.loading || state.error) return false;
  if (hour !== null && state.hour !== hour) return false;
  if (!Array.isArray(state.center) || state.center.length !== 2 || !Number.isFinite(state.zoom)) return false;
  return Math.abs(state.zoom - pose.height) <= tolerance
    && Math.abs(state.center[0] - pose.center[0]) <= tolerance
    && Math.abs(state.center[1] - pose.center[1]) <= tolerance;
}

const poseCheck = (pose, hour, tolerance = CLOSE_CAMERA_TOLERANCE) =>
  `x => (${cameraPoseMatches.toString()})(x, ${JSON.stringify(pose)}, ${JSON.stringify(hour)}, ${tolerance})`;

async function seekMapFrame(s, view, fraction, timeoutMs, { requireCamera = false } = {}) {
  const line = await s.timeline();
  if (!line) return { hour: null, state: null };
  const expectedHour = Math.round(line.min + fraction * (line.max - line.min));
  const before = Date.now();
  const prevSent = s.app.lastSent?.('set_dashboard_maps') ?? null;
  await s.app.click(Session.timelineX(line, fraction), line.y);
  const deadline = Date.now() + timeoutMs;
  let soughtHour = (await s.timeline()).value;
  while (expectedHour !== line.value && soughtHour === line.value && Date.now() < deadline) {
    await sleep(16);
    soughtHour = (await s.timeline()).value;
  }
  if (expectedHour !== line.value && soughtHour === line.value) {
    return { hour: expectedHour, state: null };
  }
  const latestSent = s.app.lastSent?.('set_dashboard_maps') ?? null;
  const sentEntry = latestSent && latestSent !== prevSent
    ? latestSent.args?.views?.find(entry => entry.id === view && entry.hour === soughtHour) ?? null
    : null;
  const minSequence = Number.isFinite(sentEntry?.sequence) ? sentEntry.sequence : null;
  const cameraPredicate = requireCamera ? ' && Array.isArray(x.center) && x.center.length === 2 && Number.isFinite(x.zoom)' : '';
  const seqPredicate = minSequence !== null ? ` && (x.sequence ?? 0) >= ${minSequence}` : '';
  const predicate = `x => !x.loading && !x.error && x.hour === ${soughtHour}${seqPredicate}${cameraPredicate}`;
  const remaining = Math.max(0, deadline - Date.now());
  let drawn = await drawnState(s, view, before, predicate, remaining);
  if (!drawn && soughtHour === line.value) {
    const last = await s.app.evaluate(`__e2e.mapStates.filter(x => x.id === ${JSON.stringify(view)}).at(-1) ?? null`);
    const cameraOk = !requireCamera || (Array.isArray(last?.center) && last.center.length === 2 && Number.isFinite(last?.zoom));
    if (last && !last.loading && !last.error && last.hour === soughtHour && cameraOk) drawn = last;
  }
  return { hour: soughtHour, state: drawn };
}

export async function mapPictures(s, input, visual, dpr, work, seeks, { timeoutMs = 15_000, stepDelayMs = 150 } = {}) {
  const area = await mapArea(s, dpr);
  const exportAs = caseId => exportPicture(s, input, visual, work, caseId, { timeoutMs, stepDelayMs });
  const results = [];
  for (const [label, fraction] of seeks) {
    const { hour, state: drawn } = await seekMapFrame(s, area.view, fraction, timeoutMs, { requireCamera: true });
    results.push({ label, fraction, hour, state: drawn });
    if (!drawn) {
      visual.fail(`map/default camera/${label}`, 'map', `the map never drew the sought hour ${hour ?? label}`);
      continue;
    }
    await exportAs(`map/default camera/${label}`);
  }
  log('map pictures', visual.items.filter(item => item.kind === 'map').length);
  return results;
}

export async function zoomedMapPicture(s, input, visual, dpr, work, [label, fraction], defaultState = null, { timeoutMs = 15_000, stepDelayMs = 150 } = {}) {
  const caseId = `map/zoomed camera/${label}`;
  const area = await mapArea(s, dpr);
  const currentLine = await s.timeline();
  let base = defaultState;
  const baseMatchesCurrent = base && !base.loading && !base.error && base.hour != null
    && Array.isArray(base.center) && base.center.length === 2 && Number.isFinite(base.zoom)
    && (!currentLine || base.hour === currentLine.value);
  if (!baseMatchesCurrent) {
    const { hour, state } = await seekMapFrame(s, area.view, fraction, timeoutMs, { requireCamera: true });
    base = state;
    if (!base) {
      visual.fail(caseId, 'map', `the map never drew the sought hour ${hour ?? label} with a default camera before the close-camera capture`);
      return;
    }
  }
  let requested;
  try { requested = deriveCloseCameraPose(base); } catch (error) {
    visual.fail(caseId, 'map', error.message);
    return;
  }
  let dispatched = false;
  try {
    const before = Date.now();
    dispatched = true;
    const reply = await input.ask({ op: 'camera', view: area.view, center: requested.center, height: requested.height });
    const pose = resolveCameraReply(reply);
    if (!(pose.height < base.zoom - CLOSE_CAMERA_TOLERANCE)) {
      visual.fail(caseId, 'map', `the resolved close camera height (${pose.height}) is not closer than the default zoom (${base.zoom})`);
      return;
    }
    const reached = await drawnState(s, area.view, before, poseCheck(pose, base.hour), timeoutMs);
    if (!reached) {
      const last = await s.app.evaluate(`__e2e.mapStates.filter(x => x.id === ${JSON.stringify(area.view)}).at(-1) ?? null`);
      const shown = last ? `center=${JSON.stringify(last.center ?? null)}, zoom=${last.zoom ?? null}, hour=${last.hour ?? null}, loading=${last.loading}` : 'no state';
      visual.fail(caseId, 'map', `the map never drew the requested close camera pose (${pose.center.join(', ')} at height ${pose.height}, hour ${base.hour}; last ${shown})`);
      return;
    }
    await exportPicture(s, input, visual, work, caseId, { timeoutMs, stepDelayMs });
  } catch (error) {
    visual.fail(caseId, 'map', `close camera capture failed: ${error.message}`);
  } finally {
    if (dispatched) {
      try {
        const resetBefore = Date.now();
        const resetReply = await input.ask({ op: 'camera', view: area.view, reset: true });
        const resetPose = resolveCameraReply(resetReply, { center: base.center, height: base.zoom });
        const restored = await drawnState(s, area.view, resetBefore, poseCheck(resetPose, base.hour), timeoutMs);
        if (!restored) {
          const last = await s.app.evaluate(`__e2e.mapStates.filter(x => x.id === ${JSON.stringify(area.view)}).at(-1) ?? null`);
          const shown = last ? `center=${JSON.stringify(last.center ?? null)}, zoom=${last.zoom ?? null}, hour=${last.hour ?? null}, loading=${last.loading}` : 'no state';
          visual.fail(`${caseId}/reset`, 'map', `the map never redrew its default camera after reset (${resetPose.center.join(', ')} at height ${resetPose.height}, hour ${base.hour}; last ${shown})`);
        }
      } catch (error) {
        visual.fail(`${caseId}/reset`, 'map', `close camera reset failed: ${error.message}`);
      }
    }
  }
}

async function exportPicture(s, input, visual, work, caseId, { timeoutMs = 20_000, stepDelayMs = 150 } = {}) {
  const folder = path.join(work, 'map-exports');
  await mkdir(folder, { recursive: true });
  const file = path.join(folder, `${caseId.replace(/[^\w.-]+/g, '_')}.png`);
  if (existsSync(file)) await unlink(file);
  await s.click('article.dash-widget.kind-map :is([data-widget-settings], button[aria-label="Map settings"])');
  if (s.until) await s.until(`document.querySelector(':is(.widget-settings-menu, .map-controls-menu):popover-open')`, 3000);
  else if (stepDelayMs > 0) await sleep(stepDelayMs);
  const exportSince = Date.now();
  await s.click(':is(.widget-settings-menu, .map-controls-menu):popover-open .map-menu-action', { text: 'Export map', partial: true });
  await input.ask({ op: 'save_dialog', path: file, seconds: 20 });
  const done = await s.app.waitForIpc('export_map', exportSince, Math.max(timeoutMs, 20_000));
  if (!done) { visual.fail(caseId, 'map', 'export_map did not finish within 20 s'); await s.escape(); return; }
  if (!done.ok) { visual.fail(caseId, 'map', `export_map failed: ${done.error ?? `status ${done.status}`}`); await s.escape(); return; }
  if (done.result === null) { visual.fail(caseId, 'map', 'export_map was cancelled before writing an image'); await s.escape(); return; }
  try {
    await assertValidPng(file);
  } catch (error) {
    visual.fail(caseId, 'map', `exported PNG is invalid: ${error.message}`);
    await s.escape();
    return;
  }
  await visual.addFile(caseId, 'map', file);
  await s.escape();
}
