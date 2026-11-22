import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {
  CLOSE_CAMERA_MIN_HEIGHT,
  CLOSE_CAMERA_ZOOM_SHARE,
  cameraPoseMatches,
  deriveCloseCameraPose,
  zoomedMapPicture,
} from './map.mjs';

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

function createFakeHarness({ reachClose = true, reachReset = true } = {}) {
  const mapStates = [];
  const commands = [];
  const clicks = [];
  const visualItems = [];
  let escaped = 0;

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
      async click() {},
    },
    async click(selector, options) {
      clicks.push({ selector, options });
    },
    async escape() {
      escaped++;
    },
    async timeline() {
      return { left: 10, width: 500, y: 20, min: 0, max: 200, value: 190 };
    },
  };

  const input = {
    async ask(command) {
      commands.push(command);
      const posted = Date.now() + 5;
      if (command.op === 'camera' && !command.reset && reachClose) {
        mapStates.push({
          id: command.view,
          hour: 190,
          loading: false,
          error: null,
          center: [...command.center],
          zoom: command.height,
          drawnAt: posted + 1,
        });
      } else if (command.op === 'camera' && command.reset && reachReset) {
        mapStates.push({
          id: command.view,
          hour: 190,
          loading: false,
          error: null,
          center: [2816, 1024],
          zoom: 3000,
          drawnAt: posted + 1,
        });
      } else if (command.op === 'save_dialog') {
        await writeFile(command.path, Buffer.from([0x89, 0x50, 0x4e, 0x47]));
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

test('zoomedMapPicture sets repeatable close pose, verifies frame, exports and resets camera', async () => {
  const work = await mkdtemp(path.join(os.tmpdir(), 'battleplan-map-e2e-'));
  try {
    const harness = createFakeHarness({ reachClose: true, reachReset: true });
    const defaultState = { id: 'overview-map', hour: 190, loading: false, error: null, center: [2816, 1024], zoom: 3000 };
    await zoomedMapPicture(harness.s, harness.input, harness.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 20, stepDelayMs: 0 });

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

test('zoomedMapPicture fails without exporting when the close pose is never drawn and still resets', async () => {
  const work = await mkdtemp(path.join(os.tmpdir(), 'battleplan-map-e2e-'));
  try {
    const harness = createFakeHarness({ reachClose: false, reachReset: true });
    const defaultState = { id: 'overview-map', hour: 190, loading: false, error: null, center: [2816, 1024], zoom: 3000 };
    harness.mapStates.push({ ...defaultState, drawnAt: Date.now() - 10 });
    await zoomedMapPicture(harness.s, harness.input, harness.visual, 1, work, ['late', 0.95], defaultState, { timeoutMs: 0, stepDelayMs: 0 });

    assert.equal(harness.commands.some(command => command.op === 'save_dialog'), false);
    assert.deepEqual(harness.commands.at(-1), { op: 'camera', view: 'overview-map', reset: true });
    assert.equal(harness.visualItems.length, 1);
    assert.equal(harness.visualItems[0].caseId, 'map/zoomed camera/late');
    assert.equal(harness.visualItems[0].status, 'error');
    assert.match(harness.visualItems[0].error, /close camera pose/);
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
