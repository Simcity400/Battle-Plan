import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { crc32, deflateSync } from 'node:zlib';
import { PNG } from 'pngjs';
import { createIpcTracker } from './cdp.mjs';
import {
  CLOSE_CAMERA_MIN_HEIGHT,
  CLOSE_CAMERA_ZOOM_SHARE,
  assertValidPng,
  cameraPoseMatches,
  deriveCloseCameraPose,
  mapModes,
  mapPictures,
  zoomedMapPicture,
} from './map.mjs';

function makeValidPng(width = 1, height = 1) {
  const png = new PNG({ width, height });
  png.data.fill(255);
  return PNG.sync.write(png);
}

function makeEmptyIdatPng(width = 2, height = 2) {
  const chunk = (type, data) => {
    const header = Buffer.alloc(8);
    header.writeUInt32BE(data.length, 0);
    header.write(type, 4, 4, 'ascii');
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(Buffer.concat([header.subarray(4), data])) >>> 0, 0);
    return Buffer.concat([header, data, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr[8] = 8;
  ihdr[9] = 6;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(Buffer.alloc(0))),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

test('createIpcTracker ignores OPTIONS preflight and reads Tauri-Response header on HTTP 200', async () => {
  const listeners = new Map();
  const bodies = new Map();
  let clock = 1000;
  const emit = (method, params) => {
    for (const listener of listeners.get(method) ?? []) listener(params);
  };
  const tracker = createIpcTracker({
    on: (method, listener) => {
      if (!listeners.has(method)) listeners.set(method, new Set());
      listeners.get(method).add(listener);
    },
    getResponseBody: async requestId => ({ body: bodies.get(requestId) ?? '', base64Encoded: false }),
    now: () => ++clock,
  });

  const errorSince = clock;
  emit('Network.requestWillBeSent', { requestId: 'opt-1', request: { url: 'http://ipc.localhost/export_map', method: 'OPTIONS' } });
  emit('Network.responseReceived', { requestId: 'opt-1', response: { status: 204, headers: {} } });
  emit('Network.loadingFinished', { requestId: 'opt-1' });

  emit('Network.requestWillBeSent', { requestId: 'post-err', request: { url: 'http://ipc.localhost/export_map', method: 'POST' } });
  emit('Network.responseReceived', { requestId: 'post-err', response: { status: 200, headers: { 'Tauri-Response': 'error' } } });
  bodies.set('post-err', JSON.stringify('Couldn’t save the map image: disk full'));
  emit('Network.loadingFinished', { requestId: 'post-err' });

  const failed = await tracker.waitForIpc('export_map', errorSince, 0);
  assert.deepEqual(failed, { ok: false, status: 200, result: null, error: 'Couldn’t save the map image: disk full' });

  const okSince = clock;
  emit('Network.requestWillBeSent', { requestId: 'opt-2', request: { url: 'http://ipc.localhost/export_map', method: 'OPTIONS' } });
  emit('Network.responseReceived', { requestId: 'opt-2', response: { status: 204, headers: {} } });
  emit('Network.loadingFinished', { requestId: 'opt-2' });

  emit('Network.requestWillBeSent', { requestId: 'post-ok', request: { url: 'http://ipc.localhost/export_map', method: 'POST' } });
  emit('Network.responseReceived', { requestId: 'post-ok', response: { status: 200, headers: { 'tauri-response': 'ok' } } });
  bodies.set('post-ok', JSON.stringify('Saved map.png: branch 1, hour 190'));
  emit('Network.loadingFinished', { requestId: 'post-ok' });

  const succeeded = await tracker.waitForIpc('export_map', okSince, 0);
  assert.deepEqual(succeeded, { ok: true, status: 200, result: 'Saved map.png: branch 1, hour 190', error: null });
});

test('deriveCloseCameraPose derives a repeatable close pose from default camera state', () => {
  const first = deriveCloseCameraPose({ center: [2816, 1024], zoom: 3000 });
  const second = deriveCloseCameraPose({ center: [2816, 1024], zoom: 3000 });
  assert.deepEqual(first, second);
  assert.deepEqual(first.center, [2816, 1024]);
  assert.equal(first.height, Math.round(3000 * CLOSE_CAMERA_ZOOM_SHARE * 100) / 100);
  assert.ok(first.height < 3000);

  const smaller = deriveCloseCameraPose({ center: [512, 256], zoom: 500 });
  assert.equal(smaller.height, CLOSE_CAMERA_MIN_HEIGHT);

  const tiny = deriveCloseCameraPose({ center: [200, 100], zoom: 160 });
  assert.equal(tiny.height, 80);

  for (const invalid of [null, {}, { center: [100], zoom: 500 }, { center: [NaN, 100], zoom: 500 }, { center: [100, 100], zoom: 0 }, { center: [100, 100], zoom: -10 }]) {
    assert.throws(() => deriveCloseCameraPose(invalid), /close camera pose/);
  }
});

test('cameraPoseMatches verifies drawn frame camera, hour, loading and error state', () => {
  const pose = { center: [2816, 1024], height: 180 };
  assert.equal(cameraPoseMatches({ center: [2816.1, 1023.9], zoom: 180.2, hour: 120, loading: false, error: null }, pose, 120), true);
  assert.equal(cameraPoseMatches({ center: [2816, 1024], zoom: 180, hour: 120, loading: true, error: null }, pose, 120), false);
  assert.equal(cameraPoseMatches({ center: [2816, 1024], zoom: 180, hour: 120, loading: false, error: 'failed' }, pose, 120), false);
  assert.equal(cameraPoseMatches({ center: [2816, 1024], zoom: 180, hour: 121, loading: false, error: null }, pose, 120), false);
  assert.equal(cameraPoseMatches({ center: [2816, 1024], zoom: 300, hour: 120, loading: false, error: null }, pose, 120), false);
  assert.equal(cameraPoseMatches({ center: [2830, 1024], zoom: 180, hour: 120, loading: false, error: null }, pose, 120), false);
});

test('assertValidPng accepts a complete PNG and rejects prefix, truncated, and empty-scanline files', async () => {
  const work = await mkdtemp(path.join(os.tmpdir(), 'battleplan-png-e2e-'));
  try {
    const valid = path.join(work, 'valid.png');
    const prefix = path.join(work, 'prefix.png');
    const truncated = path.join(work, 'truncated.png');
    const emptyIdat = path.join(work, 'empty-idat.png');
    const pngBytes = makeValidPng(2, 2);
    await writeFile(valid, pngBytes);
    await writeFile(prefix, Buffer.from([0x89, 0x50, 0x4e, 0x47]));
    await writeFile(truncated, pngBytes.subarray(0, pngBytes.length - 6));
    await writeFile(emptyIdat, makeEmptyIdatPng(2, 2));

    assert.deepEqual(await assertValidPng(valid), { width: 2, height: 2 });
    await assert.rejects(() => assertValidPng(prefix));
    await assert.rejects(() => assertValidPng(truncated));
    await assert.rejects(() => assertValidPng(emptyIdat));
  } finally {
    await rm(work, { recursive: true, force: true });
  }
});

function createFakeHarness({
  reachClose = true,
  reachReset = true,
  placeError = null,
  resetError = null,
  minHeight = 50,
  defaultCenter = [2816, 1024],
  defaultZoom = 3000,
  initialHour = 100,
  exportBytes = makeValidPng(),
  exportReply = { ok: true, status: 200, result: 'Saved map.png', error: null },
  onTimelineClick = null,
} = {}) {
  const mapStates = [];
  const commands = [];
  const clicks = [];
  const visualItems = [];
  let escaped = 0;
  let timelineHour = initialHour;
  let lastExport = null;

  const s = {
    app: {
      async evaluate(expression) {
        if (expression.includes('article.dash-widget.kind-map')) {
          return { view: 'overview-map', left: 100, top: 100, right: 1100, bottom: 600 };
        }
        const match = expression.match(/^__e2e\.mapState\(("[^"]+"),\s*(\d+),\s*([\s\S]+)\)$/);
        if (match) {
          const view = JSON.parse(match[1]);
          const after = Number(match[2]);
          const predicate = new Function(`return (${match[3]});`)();
          return mapStates.find(state => state.id === view && (state.drawnAt ?? 0) > after && predicate(state)) ?? null;
        }
        if (expression.includes('__e2e.mapStates.filter')) {
          return mapStates.filter(state => state.id === 'overview-map').at(-1) ?? null;
        }
        throw new Error(`unexpected evaluate: ${expression}`);
      },
      async click(x) {
        const fraction = Math.max(0, Math.min(1, (x - 17) / (500 - 14)));
        const previous = timelineHour;
        timelineHour = Math.round(fraction * 200);
        if (onTimelineClick) onTimelineClick({ previous, next: timelineHour, mapStates });
      },
      async waitForIpc(command) {
        if (command !== 'export_map') throw new Error(`unexpected waitForIpc: ${command}`);
        return lastExport;
      },
    },
    async click(selector, options) {
      clicks.push({ selector, options });
    },
    async until() {
      return true;
    },
    async escape() {
      escaped++;
    },
    async timeline() {
      return { left: 10, width: 500, y: 20, min: 0, max: 200, value: timelineHour };
    },
  };

  const input = {
    async ask(command) {
      commands.push(command);
      const posted = Date.now() + 5;
      if (command.op === 'camera' && !command.reset) {
        if (placeError) throw new Error(placeError);
        const resolved = { center: [...command.center], height: Math.max(minHeight, command.height) };
        if (reachClose) {
          mapStates.push({
            id: command.view,
            hour: timelineHour,
            loading: false,
            error: null,
            center: [...resolved.center],
            zoom: resolved.height,
            drawnAt: posted + 1,
          });
        }
        return { posted: [posted], center: resolved.center, height: resolved.height };
      }
      if (command.op === 'camera' && command.reset) {
        if (resetError) throw new Error(resetError);
        const resolved = { center: [...defaultCenter], height: defaultZoom };
        if (reachReset) {
          mapStates.push({
            id: command.view,
            hour: timelineHour,
            loading: false,
            error: null,
            center: [...resolved.center],
            zoom: resolved.height,
            drawnAt: posted + 1,
          });
        }
        return { posted: [posted], center: resolved.center, height: resolved.height };
      }
      if (command.op === 'save_dialog') {
        if (exportBytes !== null) await writeFile(command.path, exportBytes);
        lastExport = typeof exportReply === 'function' ? exportReply(command.path) : exportReply;
        return { saved: command.path };
      }
      return { posted: [posted] };
    },
  };

  const visual = {
    items: visualItems,
    async addFile(caseId, kind, source) {
      visualItems.push({ caseId, kind, status: 'captured', source, exists: existsSync(source) });
    },
    fail(caseId, kind, error) {
      visualItems.push({ caseId, kind, status: 'error', error });
    },
  };

  return { s, input, visual, commands, clicks, visualItems, mapStates, escaped: () => escaped };
}

test('zoomedMapPicture sets repeatable close pose, verifies frame, exports valid PNG and resets camera', async () => {
  const work = await mkdtemp(path.join(os.tmpdir(), 'battleplan-map-e2e-'));
  try {
    const harness = createFakeHarness({ initialHour: 190, reachClose: true, reachReset: true });
    const defaultState = { id: 'overview-map', hour: 190, loading: false, error: null, center: [2816, 1024], zoom: 3000 };
    await zoomedMapPicture(harness.s, harness.input, harness.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 0, stepDelayMs: 0 });

    assert.equal(harness.commands.some(command => command.op === 'wheel'), false);
    assert.deepEqual(harness.commands[0], { op: 'camera', view: 'overview-map', center: [2816, 1024], height: 180 });
    assert.equal(harness.commands[1].op, 'save_dialog');
    assert.deepEqual(harness.commands[2], { op: 'camera', view: 'overview-map', reset: true });
    assert.equal(harness.visualItems.length, 1);
    assert.equal(harness.visualItems[0].caseId, 'map/zoomed camera/late');
    assert.equal(harness.visualItems[0].status, 'captured');
    assert.equal(harness.visualItems[0].exists, true);
    assert.equal(harness.escaped(), 1);
  } finally {
    await rm(work, { recursive: true, force: true });
  }
});

test('zoomedMapPicture accepts viewer-clamped modded-map height and rejects non-closer height', async () => {
  const work = await mkdtemp(path.join(os.tmpdir(), 'battleplan-map-e2e-'));
  try {
    const modded = createFakeHarness({ initialHour: 190, minHeight: 250, defaultZoom: 3000 });
    const defaultState = { id: 'overview-map', hour: 190, loading: false, error: null, center: [2816, 1024], zoom: 3000 };
    await zoomedMapPicture(modded.s, modded.input, modded.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 0, stepDelayMs: 0 });
    assert.equal(modded.visualItems.length, 1);
    assert.equal(modded.visualItems[0].status, 'captured');

    const unzoomable = createFakeHarness({ initialHour: 190, minHeight: 3000, defaultZoom: 3000 });
    await zoomedMapPicture(unzoomable.s, unzoomable.input, unzoomable.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 0, stepDelayMs: 0 });
    assert.equal(unzoomable.commands.some(command => command.op === 'save_dialog'), false);
    assert.equal(unzoomable.visualItems.length, 1);
    assert.equal(unzoomable.visualItems[0].status, 'error');
    assert.match(unzoomable.visualItems[0].error, /not closer/i);
  } finally {
    await rm(work, { recursive: true, force: true });
  }
});

test('mapPictures and zoomedMapPicture ignore stale frames from the previous hour after a timeline seek', async () => {
  const work = await mkdtemp(path.join(os.tmpdir(), 'battleplan-map-e2e-'));
  try {
    const staleThenFresh = createFakeHarness({
      initialHour: 100,
      onTimelineClick({ previous, next, mapStates }) {
        const now = Date.now();
        mapStates.push({ id: 'overview-map', hour: previous, loading: false, error: null, center: [1000, 500], zoom: 2500, drawnAt: now + 1 });
        mapStates.push({ id: 'overview-map', hour: next, loading: false, error: null, center: [2816, 1024], zoom: 3000, drawnAt: now + 2 });
      },
    });
    const results = await mapPictures(staleThenFresh.s, staleThenFresh.input, staleThenFresh.visual, 1, work, [['late', 0.95]], { timeoutMs: 0, stepDelayMs: 0 });
    assert.equal(results[0].state?.hour, 190);
    assert.equal(staleThenFresh.visualItems[0].status, 'captured');

    const staleOnly = createFakeHarness({
      initialHour: 100,
      onTimelineClick({ previous, mapStates }) {
        mapStates.push({ id: 'overview-map', hour: previous, loading: false, error: null, center: [1000, 500], zoom: 2500, drawnAt: Date.now() + 1 });
      },
    });
    await zoomedMapPicture(staleOnly.s, staleOnly.input, staleOnly.visual, 1, work, ['late', 0.95], null, { timeoutMs: 0, stepDelayMs: 0 });
    assert.equal(staleOnly.commands.some(command => command.op === 'camera'), false);
    assert.equal(staleOnly.visualItems[0].status, 'error');
    assert.match(staleOnly.visualItems[0].error, /hour 190/);
  } finally {
    await rm(work, { recursive: true, force: true });
  }
});

test('zoomedMapPicture fails export on export_map IPC error or invalid PNG', async () => {
  const work = await mkdtemp(path.join(os.tmpdir(), 'battleplan-map-e2e-'));
  try {
    const defaultState = { id: 'overview-map', hour: 190, loading: false, error: null, center: [2816, 1024], zoom: 3000 };
    const ipcFailed = createFakeHarness({
      initialHour: 190,
      exportReply: { ok: false, status: 200, result: null, error: 'Couldn’t save the map image: disk full' },
    });
    await zoomedMapPicture(ipcFailed.s, ipcFailed.input, ipcFailed.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 0, stepDelayMs: 0 });
    assert.equal(ipcFailed.visualItems[0].status, 'error');
    assert.match(ipcFailed.visualItems[0].error, /disk full/);

    const emptyIdat = createFakeHarness({
      initialHour: 190,
      exportBytes: makeEmptyIdatPng(2, 2),
    });
    await zoomedMapPicture(emptyIdat.s, emptyIdat.input, emptyIdat.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 0, stepDelayMs: 0 });
    assert.equal(emptyIdat.visualItems[0].status, 'error');
    assert.match(emptyIdat.visualItems[0].error, /PNG/);
  } finally {
    await rm(work, { recursive: true, force: true });
  }
});

test('zoomedMapPicture resets even on dropped placement ack and fails when reset fails or times out', async () => {
  const work = await mkdtemp(path.join(os.tmpdir(), 'battleplan-map-e2e-'));
  try {
    const defaultState = { id: 'overview-map', hour: 190, loading: false, error: null, center: [2816, 1024], zoom: 3000 };
    const unreached = createFakeHarness({ initialHour: 190, reachClose: false, reachReset: true });
    unreached.mapStates.push({ ...defaultState, drawnAt: Date.now() - 10 });
    await zoomedMapPicture(unreached.s, unreached.input, unreached.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 0, stepDelayMs: 0 });
    assert.equal(unreached.commands.some(command => command.op === 'save_dialog'), false);
    assert.deepEqual(unreached.commands.at(-1), { op: 'camera', view: 'overview-map', reset: true });
    assert.equal(unreached.visualItems.length, 1);
    assert.equal(unreached.visualItems[0].status, 'error');
    assert.match(unreached.visualItems[0].error, /close camera pose/);

    const droppedAck = createFakeHarness({ initialHour: 190, placeError: 'viewer did not apply camera command within 10 s', reachReset: true });
    await zoomedMapPicture(droppedAck.s, droppedAck.input, droppedAck.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 0, stepDelayMs: 0 });
    assert.deepEqual(droppedAck.commands.at(-1), { op: 'camera', view: 'overview-map', reset: true });
    assert.equal(droppedAck.visualItems.length, 1);
    assert.equal(droppedAck.visualItems[0].status, 'error');
    assert.match(droppedAck.visualItems[0].error, /viewer did not apply camera command/);

    const resetTimedOut = createFakeHarness({ initialHour: 190, reachClose: true, reachReset: false });
    await zoomedMapPicture(resetTimedOut.s, resetTimedOut.input, resetTimedOut.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 0, stepDelayMs: 0 });
    const resetTimeoutFailure = resetTimedOut.visualItems.find(item => item.status === 'error');
    assert.ok(resetTimeoutFailure);
    assert.match(resetTimeoutFailure.error, /default camera after reset/);

    const resetThrew = createFakeHarness({ initialHour: 190, reachClose: true, resetError: 'channel closed' });
    await zoomedMapPicture(resetThrew.s, resetThrew.input, resetThrew.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 0, stepDelayMs: 0 });
    const resetThrowFailure = resetThrew.visualItems.find(item => item.status === 'error');
    assert.ok(resetThrowFailure);
    assert.match(resetThrowFailure.error, /channel closed/);
  } finally {
    await rm(work, { recursive: true, force: true });
  }
});

test('sweep visual phase runs zoomedMapPicture and playback does not stub-fail it', () => {
  const sweep = readFileSync(new URL('./sweep.mjs', import.meta.url), 'utf8');
  const visualStart = sweep.indexOf("await phase('visual'");
  const mapStart = sweep.indexOf("await phase('map'", visualStart);
  const playbackStart = sweep.indexOf("await phase('playback'");
  const catalogueStart = sweep.indexOf("await phase('catalogue'", playbackStart);
  assert.ok(visualStart >= 0 && mapStart > visualStart);
  assert.ok(playbackStart > mapStart && catalogueStart > playbackStart);
  assert.match(sweep.slice(visualStart, mapStart), /await zoomedMapPicture\(/);
  assert.doesNotMatch(sweep.slice(playbackStart, catalogueStart), /zoomedMapPicture\(/);
});

test('mapModes preserves the set_dashboard_maps envelope including clockId on mode changes and restore', async () => {
  const invokes = [];
  const recorded = [];
  let activeMode = 'terrain';
  const lastArgs = {
    clockId: 'page-e2e-1',
    views: [
      { id: 'overview-map', options: { fog: true, mapMode: 'terrain' } },
      { id: 'other-map', options: { fog: false, mapMode: 'political' } },
    ],
  };
  const s = {
    app: {
      lastSent(command) {
        return command === 'set_dashboard_maps' ? { args: lastArgs } : null;
      },
      async evaluate(expression) {
        if (expression.includes("article.dash-widget.kind-map")) {
          return { view: 'overview-map', left: 12, top: 12, right: 200, bottom: 200 };
        }
        if (expression.startsWith('__e2e.mapModes[')) {
          return [
            { token: 'terrain', label: 'Terrain' },
            { token: 'states', label: 'States' },
          ];
        }
        if (expression.includes('x.mapMode).at(-1)?.mapMode')) {
          return activeMode;
        }
        if (expression.includes("invoke('set_dashboard_maps'")) {
          const fakeWindow = {
            __TAURI_INTERNALS__: {
              async invoke(command, payload) {
                assert.equal(command, 'set_dashboard_maps');
                invokes.push(payload);
                const mapView = payload.views?.find(entry => entry.id === 'overview-map');
                if (mapView?.options?.mapMode) activeMode = mapView.options.mapMode;
              },
            },
          };
          return await new Function('window', `return ${expression}`)(fakeWindow);
        }
        if (expression.startsWith('__e2e.mapState(')) {
          return { id: 'overview-map', mapMode: activeMode, loading: false, drawnAt: Date.now() };
        }
        throw new Error(`unexpected evaluate: ${expression}`);
      },
    },
    async watch() {},
    async unwatch() { return []; },
    async record(entry) { recorded.push(entry); },
  };

  await mapModes(s, 1);

  assert.equal(invokes.length, 3, 'two mode switches plus final restore');
  for (const payload of invokes) {
    assert.equal(payload.clockId, 'page-e2e-1', `missing or wrong clockId: ${JSON.stringify(payload)}`);
    assert.equal(payload.views.length, 2);
    assert.deepEqual(payload.views[1], lastArgs.views[1]);
  }
  assert.equal(invokes[0].views[0].options.mapMode, 'states');
  assert.equal(invokes[1].views[0].options.mapMode, 'terrain');
  assert.deepEqual(invokes[2], lastArgs);
  assert.equal(recorded.length, 2);
  assert.deepEqual(recorded.map(entry => entry.reasons), [[], []]);
});
