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
  hover,
  interactions,
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

test('mapModes passes an expected named refusal for supply and fails blank, unexpected, or rendering failures', async () => {
  const runModes = async (modes, drawState) => {
    const invokes = [];
    const recorded = [];
    let activeMode = 'default';
    const lastArgs = {
      clockId: 'page-e2e-1',
      views: [{ id: 'overview-map', options: { mapMode: 'default' } }],
    };
    const s = {
      app: {
        lastSent(command) { return command === 'set_dashboard_maps' ? { args: lastArgs } : null; },
        async evaluate(expression) {
          if (expression.includes('article.dash-widget.kind-map')) {
            return { view: 'overview-map', left: 12, top: 12, right: 200, bottom: 200 };
          }
          if (expression.startsWith('__e2e.mapModes[')) return modes;
          if (expression.includes('x.mapMode).at(-1)?.mapMode')) return activeMode;
          if (expression.includes("invoke('set_dashboard_maps'")) {
            const fakeWindow = {
              __TAURI_INTERNALS__: {
                async invoke(_command, payload) {
                  invokes.push(payload);
                  const mapView = payload.views?.find(entry => entry.id === 'overview-map');
                  if (mapView?.options?.mapMode) activeMode = mapView.options.mapMode;
                },
              },
            };
            return await new Function('window', `return ${expression}`)(fakeWindow);
          }
          if (expression.startsWith('__e2e.mapState(')) return drawState(activeMode);
          throw new Error(`unexpected evaluate: ${expression}`);
        },
      },
      async watch() {},
      async unwatch() { return []; },
      async record(entry) { recorded.push(entry); },
    };
    await mapModes(s, 1);
    return { invokes, recorded };
  };

  const { invokes, recorded } = await runModes([
    { token: 'default', label: 'Default', unavailable: null },
    { token: 'supply', label: 'Supply', unavailable: 'Not available in replays: supply network is not recorded.' },
    { token: 'states', label: 'States', unavailable: 'States table missing.' },
    { token: 'terrain', label: 'Terrain', unavailable: null },
  ], mode => (mode === 'terrain'
    ? { id: 'overview-map', mapMode: 'terrain', loading: false, error: 'terrain shader failed', drawnAt: Date.now() }
    : { id: 'overview-map', mapMode: mode, loading: false, error: null, drawnAt: Date.now() }));

  const byItem = new Map(recorded.map(entry => [entry.item, entry]));
  assert.equal(invokes.some(payload => payload.views[0].options.mapMode === 'supply'), false);
  assert.equal(invokes.some(payload => payload.views[0].options.mapMode === 'states'), false);

  const supply = byItem.get('mode: supply');
  assert.equal(supply.pass, true);
  assert.equal(supply.verdict, 'refuses by name');
  assert.deepEqual(supply.reasons, []);
  assert.equal(supply.unavailable, 'Not available in replays: supply network is not recorded.');

  const states = byItem.get('mode: states');
  assert.equal(states.pass, false);
  assert.match(states.reasons.join('; '), /unexpected refusal/i);

  const terrain = byItem.get('mode: terrain');
  assert.equal(terrain.pass, false);
  assert.match(terrain.reasons.join('; '), /terrain shader failed/);

  const def = byItem.get('mode: default');
  assert.equal(def.pass, true);
  assert.deepEqual(def.reasons, []);

  const blankRefusal = await runModes([
    { token: 'supply', label: 'Supply', unavailable: '   ' },
  ], () => null);
  assert.equal(blankRefusal.recorded[0].pass, false);
  assert.match(blankRefusal.recorded[0].reasons.join('; '), /stated reason/i);
});

test('hover separates presence from timing and does not require a neutral point to confirm presence', async () => {
  const runHover = async stateForMove => {
    const mapStates = [];
    const recorded = [];
    let now = 1000;
    const s = {
      app: {
        async evaluate(expression) {
          if (expression.includes('article.dash-widget.kind-map')) {
            return { view: 'overview-map', left: 0, top: 0, right: 200, bottom: 120 };
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
      },
      async watch() {},
      async unwatch() { return []; },
      async record(entry) { recorded.push(entry); },
      async fail(area, item, label, reason) { recorded.push({ area, item, label, ms: null, pass: false, reasons: [reason] }); },
    };
    const input = {
      async ask(command) {
        if (command.op !== 'move') throw new Error(`unexpected input op: ${command.op}`);
        const posted = (now += 10);
        const [px, py] = command.points[0];
        const next = stateForMove(px, py, posted);
        for (const [index, frame] of (Array.isArray(next) ? next : next ? [next] : []).entries()) {
          mapStates.push({ id: 'overview-map', tooltipDelayMs: 0, drawnAt: posted + 5 + index, ...frame });
        }
        return { posted: [posted] };
      },
    };
    await hover(s, input, 1, { scanTimeoutMs: 0, timeoutMs: 0, neutralPauseMs: 0 });
    return recorded;
  };

  let movesNoNeutral = 0;
  const noNeutral = await runHover(() => {
    movesNoNeutral++;
    if (movesNoNeutral === 1) return { hover: 'division:1', tooltip: 'province:42' };
    return { hover: 'button:supply', tooltip: 'province:42' };
  });
  for (const [id, expected] of [['division counter', 'division:1'], ['province tooltip', 'province:42'], ['map GUI button', 'button:supply']]) {
    const presence = noNeutral.find(entry => entry.item === `hover presence: ${id}`);
    const timing = noNeutral.find(entry => entry.item === `hover: ${id}`);
    assert.ok(presence, `missing presence record for ${id}`);
    assert.equal(presence.pass, true);
    assert.equal(presence.label, expected);
    assert.deepEqual(presence.reasons, []);
    assert.ok(timing, `missing timing record for ${id}`);
    assert.equal(timing.pass, false);
    assert.equal(timing.label, expected);
    assert.match(timing.reasons.join('; '), /cannot be timed/i);
  }

  let neutralPoint = null;
  let movesStuckNeutral = 0;
  const stuckNeutral = await runHover((px, py) => {
    movesStuckNeutral++;
    if (movesStuckNeutral === 1) { neutralPoint = [px, py]; return { hover: null, tooltip: null }; }
    if (neutralPoint && px === neutralPoint[0] && py === neutralPoint[1]) return { hover: null, tooltip: 'province:stuck' };
    if (movesStuckNeutral === 2) return { hover: 'division:7', tooltip: 'province:9' };
    return { hover: 'button:terrain', tooltip: 'province:9' };
  });
  for (const [id, expected] of [['division counter', 'division:7'], ['province tooltip', 'province:9'], ['map GUI button', 'button:terrain']]) {
    const presence = stuckNeutral.find(entry => entry.item === `hover presence: ${id}`);
    const timing = stuckNeutral.find(entry => entry.item === `hover: ${id}`);
    assert.equal(presence?.pass, true);
    assert.equal(presence?.label, expected);
    assert.equal(timing?.pass, false);
    assert.match(timing?.reasons.join('; ') ?? '', /never cleared/i);
  }

  let cleanNeutral = null;
  let divPoint = null;
  let movesClean = 0;
  const timed = await runHover((px, py) => {
    movesClean++;
    if (movesClean === 1) { cleanNeutral = [px, py]; return { hover: null, tooltip: null }; }
    if (cleanNeutral && px === cleanNeutral[0] && py === cleanNeutral[1]) return { hover: null, tooltip: null };
    if (movesClean === 2 || (divPoint && px === divPoint[0] && py === divPoint[1])) {
      divPoint ??= [px, py];
      return { hover: 'division:3', tooltip: 'province:5' };
    }
    return { hover: 'button:states', tooltip: 'province:5' };
  });
  for (const [id, expected] of [['division counter', 'division:3'], ['province tooltip', 'province:5'], ['map GUI button', 'button:states']]) {
    const presence = timed.find(entry => entry.item === `hover presence: ${id}`);
    const timing = timed.find(entry => entry.item === `hover: ${id}`);
    assert.equal(presence?.pass, true);
    assert.equal(presence?.label, expected);
    assert.equal(timing?.pass, true);
    assert.equal(timing?.label, expected);
    assert.equal(timing?.ms, 5);
  }

  const missingButton = await runHover(() => ({ hover: 'division:2', tooltip: 'province:8' }));
  const missingPresence = missingButton.find(entry => entry.item === 'hover presence: map GUI button');
  const missingTiming = missingButton.find(entry => entry.item === 'hover: map GUI button');
  assert.equal(missingPresence?.pass, false);
  assert.equal(missingPresence?.label, null);
  assert.match(missingPresence?.reasons.join('; ') ?? '', /no map GUI button/);
  assert.equal(missingTiming?.pass, false);
  assert.equal(missingTiming?.ms, null);
  assert.match(missingTiming?.reasons.join('; ') ?? '', /never observed/i);

  let delayedNeutral = null;
  let delayedPoint = null;
  let delayedButtonPoint = null;
  let delayedMoves = 0;
  const delayedAfterNeutral = await runHover((px, py) => {
    delayedMoves++;
    if (delayedMoves === 1) { delayedNeutral = [px, py]; return { hover: null, tooltip: null }; }
    if (delayedNeutral && px === delayedNeutral[0] && py === delayedNeutral[1]) return { hover: null, tooltip: null };
    if (delayedMoves === 2 || (delayedPoint && px === delayedPoint[0] && py === delayedPoint[1])) {
      delayedPoint ??= [px, py];
      return [
        { hover: null, tooltip: null },
        { hover: 'division:4', tooltip: null },
        { hover: 'division:4', tooltip: 'province:77' },
      ];
    }
    if (delayedMoves === 3 || (delayedButtonPoint && px === delayedButtonPoint[0] && py === delayedButtonPoint[1])) {
      delayedButtonPoint ??= [px, py];
      return [{ hover: null, tooltip: null }, { hover: 'button:supply', tooltip: null }];
    }
    return { hover: null, tooltip: null };
  });
  const delayedDivisionPresence = delayedAfterNeutral.find(entry => entry.item === 'hover presence: division counter');
  const delayedDivisionTiming = delayedAfterNeutral.find(entry => entry.item === 'hover: division counter');
  assert.equal(delayedDivisionPresence?.pass, true);
  assert.equal(delayedDivisionPresence?.label, 'division:4');
  assert.equal(delayedDivisionTiming?.pass, true);
  assert.equal(delayedDivisionTiming?.ms, 6);
  const delayedPresence = delayedAfterNeutral.find(entry => entry.item === 'hover presence: province tooltip');
  const delayedTiming = delayedAfterNeutral.find(entry => entry.item === 'hover: province tooltip');
  assert.equal(delayedPresence?.pass, true);
  assert.equal(delayedPresence?.label, 'province:77');
  assert.equal(delayedTiming?.pass, true);
  assert.equal(delayedTiming?.label, 'province:77');
  assert.equal(delayedTiming?.ms, 7);
  const delayedButtonPresence = delayedAfterNeutral.find(entry => entry.item === 'hover presence: map GUI button');
  const delayedButtonTiming = delayedAfterNeutral.find(entry => entry.item === 'hover: map GUI button');
  assert.equal(delayedButtonPresence?.pass, true);
  assert.equal(delayedButtonPresence?.label, 'button:supply');
  assert.equal(delayedButtonTiming?.pass, true);
  assert.equal(delayedButtonTiming?.label, 'button:supply');
  assert.equal(delayedButtonTiming?.ms, 6);
});

test('interactions confirms fly and walk movement from the campaign map camera snapshot and fails when unmoved or stuck in map pose', async () => {
  const runCampaignCase = async ({ id, snapshots }) => {
    const recorded = [];
    const mapActions = [];
    let snapshotIndex = 0;
    const s = {
      app: {
        async evaluate(expression) {
          if (expression.includes('.map-panel:not([hidden])')) {
            return { left: 100, top: 100, right: 1100, bottom: 700 };
          }
          if (expression.includes("invoke('map_action'") || expression.includes("invoke('map_export_snapshot'")) {
            const fakeWindow = {
              __TAURI_INTERNALS__: {
                async invoke(command, payload) {
                  if (command === 'map_action') { mapActions.push(payload.action); return null; }
                  if (command === 'map_export_snapshot') {
                    const next = snapshots[Math.min(snapshotIndex++, snapshots.length - 1)];
                    if (next instanceof Error) throw next;
                    return next;
                  }
                  throw new Error(`unexpected invoke: ${command}`);
                },
              },
            };
            return await new Function('window', `return ${expression}`)(fakeWindow);
          }
          throw new Error(`unexpected evaluate: ${expression}`);
        },
        async click() {},
      },
      async click() {},
      async until() { return true; },
      async openPage() {},
      async timeline() { return { left: 0, width: 500, y: 10, min: 0, max: 100, value: 10 }; },
      async watch() {},
      async unwatch() { return []; },
      async record(entry) { recorded.push(entry); },
      async fail(area, item, label, reason) { recorded.push({ area, item, label, pass: false, reasons: [reason] }); },
    };
    const input = { async ask() { return { posted: [Date.now()] }; } };
    await interactions(s, input, 1, 0, { only: [id], playingModes: [false], settleMs: 0, moveDelayMs: 0 });
    return { recorded, mapActions };
  };

  const freeAt = (x, z, height) => ({ placement: { center: [x, z], height, pose: { kind: 'free', yaw: 0, pitch: -0.2, roll: 0, fovY: 1, near: 0.1, far: 1000 } } });
  const mapAt = (x, z, height) => ({ placement: { center: [x, z], height, pose: { kind: 'map' } } });

  const movedFly = await runCampaignCase({ id: 'fly', snapshots: [freeAt(100, 200, 40), freeAt(140, 230, 45)] });
  assert.equal(movedFly.recorded.length, 1);
  assert.equal(movedFly.recorded[0].pass, true);
  assert.deepEqual(movedFly.recorded[0].reasons, []);
  assert.deepEqual(movedFly.mapActions, [
    { kind: 'cameraMode', mode: 'fly', overlays: true },
    { kind: 'cameraMode', mode: 'fly', overlays: false },
    { kind: 'cameraMode', mode: 'fly', overlays: true },
    { kind: 'cameraMode', mode: 'fly', overlays: false },
  ]);

  const unmovedWalk = await runCampaignCase({ id: 'walk', snapshots: [freeAt(100, 200, 12), freeAt(100, 200, 12)] });
  assert.equal(unmovedWalk.recorded.length, 1);
  assert.equal(unmovedWalk.recorded[0].pass, false);
  assert.match(unmovedWalk.recorded[0].reasons.join('; '), /did not move/i);

  const stuckMapPose = await runCampaignCase({ id: 'fly', snapshots: [mapAt(100, 200, 500), mapAt(100, 200, 500)] });
  assert.equal(stuckMapPose.recorded.length, 1);
  assert.equal(stuckMapPose.recorded[0].pass, false);
  assert.match(stuckMapPose.recorded[0].reasons.join('; '), /map pose/i);

  const snapshotError = await runCampaignCase({ id: 'walk', snapshots: [new Error('The map stopped')] });
  assert.equal(snapshotError.recorded.length, 1);
  assert.equal(snapshotError.recorded[0].pass, false);
  assert.match(snapshotError.recorded[0].reasons.join('; '), /The map stopped/);
});

test('hover bounds its scan to a deterministic point set and reports a time-cap stop separately from absence', async () => {
  const runBoundedHover = async ({ stateForMove, width = 800, height = 400, options = {} }) => {
    const mapStates = [];
    const recorded = [];
    let moves = 0;
    let now = 1000;
    const s = {
      app: {
        async evaluate(expression) {
          if (expression.includes('article.dash-widget.kind-map')) {
            return { view: 'overview-map', left: 0, top: 0, right: width, bottom: height };
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
      },
      async watch() {},
      async unwatch() { return []; },
      async record(entry) { recorded.push(entry); },
      async fail(area, item, label, reason) { recorded.push({ area, item, label, ms: null, pass: false, reasons: [reason] }); },
    };
    const input = {
      async ask(command) {
        if (command.op !== 'move') throw new Error(`unexpected op: ${command.op}`);
        moves++;
        const posted = (now += 10);
        const [px, py] = command.points[0];
        const next = stateForMove(px, py, moves);
        if (next) mapStates.push({ id: 'overview-map', tooltipDelayMs: 0, drawnAt: posted + 2, ...next });
        return { posted: [posted] };
      },
    };
    await hover(s, input, 1, { scanTimeoutMs: 0, timeoutMs: 0, neutralPauseMs: 0, ...options });
    return { recorded, moves };
  };

  const missingDivision = await runBoundedHover({
    stateForMove: (px, py, move) => {
      if (move === 1) return { hover: null, tooltip: null };
      if (px >= 780 && py >= 380) return { hover: 'button:find-view', tooltip: null };
      return { hover: null, tooltip: 'province:101' };
    },
  });
  assert.ok(missingDivision.moves <= 125, `expected bounded scan <= 125 moves on 800x400 map, got ${missingDivision.moves}`);
  const divPresence = missingDivision.recorded.find(e => e.item === 'hover presence: division counter');
  const divTiming = missingDivision.recorded.find(e => e.item === 'hover: division counter');
  assert.equal(divPresence?.pass, false);
  assert.match(divPresence?.reasons.join('; ') ?? '', /no division counter under any of 113 points/);
  assert.equal(divTiming?.pass, false);
  assert.match(divTiming?.reasons.join('; ') ?? '', /never observed/i);
  assert.equal(missingDivision.recorded.find(e => e.item === 'hover presence: map GUI button')?.pass, true);
  assert.equal(missingDivision.recorded.find(e => e.item === 'hover presence: province tooltip')?.pass, true);

  let fakeClock = 0;
  const budgetBounded = await runBoundedHover({
    stateForMove: () => ({ hover: null, tooltip: null }),
    options: { scanBudgetMs: 25, now: () => (fakeClock += 10) },
  });
  assert.ok(budgetBounded.moves <= 10, `expected safety-net cap to stop quickly, got ${budgetBounded.moves}`);
  const timedOutPresence = budgetBounded.recorded.find(e => e.item === 'hover presence: division counter');
  const timedOutTiming = budgetBounded.recorded.find(e => e.item === 'hover: division counter');
  assert.equal(timedOutPresence?.pass, false);
  assert.match(timedOutPresence?.reasons.join('; ') ?? '', /search cut short after \d+ of \d+ points/);
  assert.doesNotMatch(timedOutPresence?.reasons.join('; ') ?? '', /no division counter/);
  assert.equal(timedOutTiming?.pass, false);
  assert.match(timedOutTiming?.reasons.join('; ') ?? '', /cut short/i);
});
