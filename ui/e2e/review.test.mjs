import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm, stat, symlink } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { runInNewContext } from 'node:vm';
import { promisify } from 'node:util';
import { tmpdir } from 'node:os';
import path from 'node:path';
import * as cache from './cache.mjs';
import * as app from './app.mjs';
import { copyPreparedSeed } from './nativeSeed.mjs';
import { captureCases, validateMatrix } from './matrix.mjs';
import { captureWidget } from './capture.mjs';

test('explicit prepared seed keeps every page and private writes with provenance', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'parity-prepared-seed-'));
  try {
    const source = path.join(root, 'input'), destination = path.join(root, 'adapter/HOI4Replay/prepared');
    await mkdir(path.join(source, 'synthetic-campaign'), { recursive: true });
    for (const [name, body] of [['manifest', 'synthetic committed manifest'], ['range-0', 'exact page zero'], ['range-1', 'exact page one']])
      await writeFile(path.join(source, 'synthetic-campaign', name), body);
    const receipt = await copyPreparedSeed(source, destination);
    assert.equal(receipt.files.length, 3);
    assert.ok(receipt.files.every(file => /^\d+$/.test(file.bytes) && /^\d+$/.test(file.modifiedNs)));
    assert.equal(await readFile(path.join(destination, 'synthetic-campaign/range-1'), 'utf8'), 'exact page one');
    await writeFile(path.join(destination, 'synthetic-campaign/range-1'), 'private prepared write');
    assert.equal(await readFile(path.join(source, 'synthetic-campaign/range-1'), 'utf8'), 'exact page one');
    assert.equal(await copyPreparedSeed(null, destination), null);
    await assert.rejects(copyPreparedSeed(source, path.join(source, 'nested')), /contain/);
    await symlink(path.join(source, 'synthetic-campaign'), path.join(source, 'linked-campaign'), 'junction');
    await assert.rejects(copyPreparedSeed(source, path.join(root, 'linked-copy')), /Linked prepared/);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture matrix preserves choices and repeated widget kinds without a country cap', () => {
  const tags = Array.from({ length: 5000 }, (_, index) => `opaque / country ${index}`);
  const matrix = validateMatrix({ version: 1, cases: [
    { id: 'first', kind: 'synthetic', hour: 7, countries: { kind: 'tags', tags }, options: { included: '?recorded:20:type', refits: true } },
    { id: 'second', kind: 'synthetic', hour: 8, time: { mode: 'follow' } },
  ] });
  const cases = captureCases([{ kind: 'synthetic' }], matrix, 99);
  assert.equal(cases.length, 2);
  assert.deepEqual(cases[0].countries.tags, tags);
  assert.equal(cases[0].options.included, '?recorded:20:type');
  assert.equal(cases[1].hour, 8);
  assert.throws(() => validateMatrix({ version: 1, cases: [{ ...cases[0] }, { ...cases[0] }] }), /Invalid capture case/);
  assert.throws(() => validateMatrix({ version: 1, cases: [{ ...cases[0], typo: true }] }), /fields/);
  assert.throws(() => captureCases([{ kind: 'other' }], matrix, 99), /not registered/);
});

test('configured parity hook applies exact countries, time and options before mounting', async () => {
  const { stripTypeScriptTypes } = await import('node:module');
  const source = stripTypeScriptTypes(await readFile(new URL('../src/dashboard/parityFixture.ts', import.meta.url), 'utf8'))
    .replace(/^import .*;$/gm, '').replace(/\bexport /g, '');
  const tags = Array.from({ length: 5000 }, (_, index) => `opaque / tag ${index}`);
  const countries = { tags, kind: 'tags' }, time = { mode: 'follow' }, options = { measure: 'week' };
  const template = { tracks: [{ id: 'main' }], widgets: [], layouts: {} };
  const host = { client: { product() {}, openSource() {} }, template,
    runtime: { tracks: [{ id: 'main', roleId: 'primary' }], defaultRoleId: 'primary', clock: { stop() {}, seek() {} } },
    setTemplate(value) { host.template = value; } };
  const window = {}; let reverseTags = false;
  const install = runInNewContext(source + '\ninstallParityFixture', { window,
    widgetDefinitions: () => [{ kind: 'synthetic', title: 'Synthetic' }],
    widgetDefinition: () => ({ timeModes: ['campaign', 'follow'], options: [{ key: 'measure' }] }),
    createWidget: (kind, trackId, id) => ({ kind, id, trackIds: [trackId], countries: { kind: 'selected' }, time: { mode: 'campaign' } }),
    GRID_COLUMNS: { lg: 12 }, validOption: (_field, value) => value === 'week',
    parseTemplate: input => ({ ...input, widgets: input.widgets.map(widget => ({ ...widget,
      countries: { kind: widget.countries.kind, tags: reverseTags ? widget.countries.tags?.toReversed() : widget.countries.tags } })) }),
  });
  const close = install(() => host);
  try {
    assert.equal(window.__hoi4Parity.capabilities().scenarioVersion, 1);
    window.__hoi4Parity.isolate('synthetic', { countries, time, options });
    assert.deepEqual(host.template.widgets[0].countries, countries);
    assert.deepEqual(host.template.widgets[0].time, time);
    assert.equal(host.template.widgets[0].options.measure, 'week');
    assert.throws(() => window.__hoi4Parity.isolate('synthetic', { options: { unregistered: 'week' } }), /Invalid parity option/);
    assert.throws(() => window.__hoi4Parity.isolate('synthetic', { time: { mode: 'unsupported' } }), /Invalid parity time mode/);
    reverseTags = true;
    assert.throws(() => window.__hoi4Parity.isolate('synthetic', { countries, time, options }), /saved widget schema/);
  } finally { close(); }
});

test('required exact model refuses absent provider and retains duplicate chart operands', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'parity-required-model-'));
  try {
    let lists = [];
    // The mock returns the surface evidence; it never creates a browser or a DOM.
    let evaluations = 0;
    const locator = { evaluate: async () => ++evaluations % 2 ? { header: true, body: true, text: '' } : undefined,
      screenshot: async () => {}, locator: () => ({ count: async () => 0 }) };
    const driver = { prepare: async () => {}, install: async () => {}, widget: () => locator,
      settle: async () => {}, prepareFullRows: async () => {}, fullRows: async () => lists, errors: [] };
    const page = { bringToFront: async () => {}, evaluate: async () => {}, close: async () => {} };
    const scenario = { id: 'exact-balance', kind: 'synthetic', hour: 10, models: ['Country balance chart'] };
    const missing = await captureWidget({ page, app: driver, kind: 'synthetic', scenario, directory: root, fileStem: 'missing' });
    assert.equal(missing.status, 'error'); assert.match(missing.error, /Required exact model is missing/);
    lists = [{ kind: 'chart', label: 'Country balance chart', complete: true, total: 2,
      rows: [0, 1].map(index => ({ index, key: `observed-payment-${index}`, raw: '-9007199254740993', readHour: 9, amount: null })),
      model: { title: '', known: false, refusal: { name: 'synthetic supplying table unavailable', count: 2 } } }];
    const captured = await captureWidget({ page, app: driver, kind: 'synthetic', scenario, directory: root, fileStem: 'complete' });
    assert.equal(captured.status, 'captured');
    assert.deepEqual(captured.model.lists, lists);
    assert.deepEqual(JSON.parse(await readFile(path.join(root, 'complete.json'), 'utf8')).model.lists, lists);
  } finally { await rm(root, { recursive: true, force: true }); }
});

// No real foreign PID is ever passed to a termination API.
test('stale parents, reused PIDs and delayed observers cannot tree-kill', async () => {
  const scripts = JSON.parse(await readFile(new URL('../../package.json', import.meta.url), 'utf8')).scripts;
  assert.match(scripts['parity:compare'], /owned_job\.py/);
  const source = await readFile(new URL('./process-guard.cjs', import.meta.url), 'utf8'), requests = [];
  const foreign = () => assert.fail('Foreign PID reached execution');
  const cp = Object.fromEntries(['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync'].map(name => [name, foreign]));
  cp.execFile = Object.assign(() => {}, { [promisify.custom]: foreign });
  const process = { env: { BATTLE_PLAN_OWNED_JOB: 'owned', BATTLE_PLAN_JOB_STOP_FILE: 'owned' }, kill: foreign };
  runInNewContext(source, { require: name => name === 'node:child_process' ? cp : name === 'node:fs' ? { writeFileSync: () => requests.push('job') }
    : name === 'node:util' ? { promisify } : { syncBuiltinESMExports() {} }, process, Error });
  for (const state of ['stale parent', 'reused PID', 'delayed observer']) {
    assert.throws(() => cp.spawnSync('taskkill /pid 42 /T /F'), /owned Job/, state);
    assert.throws(() => process.kill(42), /PID-only/, state);
  }
  assert.throws(() => cp.execFile[promisify.custom]('taskkill.exe', ['/pid', '42', '/T', '/F']), /owned Job/);
  assert.equal(requests.length, 4);

});

test('incomplete instrumentation and mismatched installed dependencies refuse', async () => {
  await assert.rejects(app.validateHook({ evaluate: async fn => { globalThis.window = { __hoi4Parity: { lists() {} } }; try { await fn(); } finally { delete globalThis.window; } } }), /capabilities/);
  const root = await mkdtemp(path.join(tmpdir(), 'parity-lock-check-'));
  try { await symlink(new URL('../../node_modules', import.meta.url).pathname.replace(/^\/(.:)/, '$1'), path.join(root, 'node_modules'), 'junction'); await writeFile(path.join(root, 'package-lock.json'), '{}'); await assert.rejects(cache.validateDependencyLock(root), /dependency lock/); }
  finally { await rm(root, { recursive: true, force: true }); }
});

test('disposable adapter refuses debug input and teardown never follows shader input junctions', async () => {
  const { startUI, removeAdapter } = await import('./server.mjs');
  const root = await mkdtemp(path.join(tmpdir(), 'parity-adapter-')), outside = await mkdtemp(path.join(tmpdir(), 'parity-input-'));
  try {
    await assert.rejects(startUI({}, { bridge: path.join(root, 'debug/dashboard-bridge.exe') }, root), /debug/);
    const adapter = path.join(root, 'bridge'); await mkdir(path.join(adapter, '.cargo-target/debug'), { recursive: true });
    await writeFile(path.join(outside, 'tool'), 'keep');
    await symlink(outside, path.join(adapter, '.cargo-target/debug/shader-tools'), 'junction');
    await removeAdapter(adapter);
    assert.equal(await readFile(path.join(outside, 'tool'), 'utf8'), 'keep');
    assert.equal(await stat(adapter).catch(() => null), null);
  } finally { await rm(root, { recursive: true, force: true }); await rm(outside, { recursive: true, force: true }); }
});

// Retained raster and budget checks exercise the reduced capture path.
test('fresh owned Edge contexts preserve exact pixels and 5,000 rows; a stall expires', () => {
  const baseURL = new URL('./', import.meta.url).href;
  const program = `import assert from 'node:assert/strict';
    import {mkdtemp,readFile,writeFile,rm} from 'node:fs/promises';
    import {tmpdir} from 'node:os'; import path from 'node:path';
    const baseURL = ${JSON.stringify(baseURL)};
    (${rasterProof.toString()})().then(() => process.exit(0), error => { console.error(error); process.exit(1); });`;
  const launcher = new URL('./owned_job.py', import.meta.url).pathname.replace(/^\/(.:)/, '$1');
  const result = spawnSync('python', [launcher, 'node', '--input-type=module', '-e', program], {encoding: 'utf8', windowsHide: true, timeout: 120_000});
  assert.equal(result.status, 0, result.stdout + result.stderr);
  assert.match(result.stderr, /"activeAfterClose": 0/);
});
async function rasterProof() {
  const { chromium } = await import('playwright-core');
  const { PNG } = await import('pngjs');
  const { captureWidget } = await import(new URL('./capture.mjs', baseURL));
  const { compareCaptures } = await import(new URL('./compare.mjs', baseURL));
  const { contextOptions, launchOwnedBrowser } = await import(new URL('./settings.mjs', baseURL));
  const app = await import(new URL('./app.mjs', baseURL)), cache = await import(new URL('./cache.mjs', baseURL));
  const root = await mkdtemp(path.join(tmpdir(), 'parity-widget-'));
  const browser = await launchOwnedBrowser(chromium);
  try {
    const image = new PNG({ width: 85, height: 85 });
    for (let i = 0; i < image.data.length; i++) image.data[i] = (i * 73) % 256;
    const source = `data:image/png;base64,${PNG.sync.write(image).toString('base64')}`;
    const dirs = [path.join(root, 'a'), path.join(root, 'b')];
    for (const [index, directory] of dirs.entries()) {
      const context = await browser.newContext(contextOptions);
      try {
        const page = await context.newPage();
        await page.setContent(`<img src="${source}" width="${index ? 85 : 16}">`); await page.locator('img').evaluate(img => img.decode());
        await page.setContent(`<style>body{margin:0}.dash-widget{width:350px;height:1400px;background:#234567}img{width:18.5px;height:18.5px}</style>
          <article class="dash-widget"><header class="dash-widget-header"><h2>Synthetic</h2></header><div class="dash-widget-scaled"><img src="${source}"><span role="status" data-parity-loading>Loading widget?</span></div></article>`);
        await page.locator('img').evaluate(img => img.decode());
        const lists = [{ complete: true, total: 5000, rows: Array.from({ length: 5000 }, (_, index) => ({ index, key: String(index), value: index * 2 })) }];
        await page.evaluate(() => setTimeout(() => {
          const fallback = document.querySelector('[data-parity-loading]');
          fallback.removeAttribute('data-parity-loading'); fallback.textContent = 'Rows';
        }, 900));
        const real = app.createApp(page, {});
        const driver = { ...real, prepare: async () => {}, install: async () => {}, widget: () => page.locator('.dash-widget'), prepareFullRows: async () => {}, fullRows: async () => lists };
        const item = await captureWidget({ page, app: driver, kind: 'synthetic', directory });
        assert.equal(item.status, 'captured'); assert.equal(item.model.lists[0].rows.length, 5000);
        assert.match(item.model.text, /Rows/); assert.doesNotMatch(item.model.text, /Loading widget/);
        assert.deepEqual(await page.evaluate(() => [innerWidth, innerHeight, devicePixelRatio]), [1440, 1000, 1]);
        const png = PNG.sync.read(await readFile(path.join(directory, item.screenshot))); assert.deepEqual([png.width, png.height], [350, 1400]);
        await cache.writeJSON(path.join(directory, 'manifest.json'), { complete: true, items: [item] });
      } finally { await context.close(); }
    }
    assert.equal((await compareCaptures(...dirs)).pass, true);
    const file = path.join(dirs[1], 'synthetic.png'), png = PNG.sync.read(await readFile(file)); png.data[0] ^= 1; await writeFile(file, PNG.sync.write(png));
    assert.equal((await compareCaptures(...dirs)).counts.DIFF, 1);
    const failure = await captureWidget({ page: { close: async () => {} }, app: { prepare: () => new Promise(() => {}) }, kind: 'stall', directory: root, budgetMs: 20 });
    assert.equal(failure.status, 'error'); assert.match(failure.error, /budget exceeded/); assert.ok(failure.wallMs < 1000);
    await new Promise(resolve => setTimeout(resolve, 30)); assert.equal(JSON.parse(await readFile(path.join(root, 'stall.json'))).status, 'error');
  } finally { await rm(root, { recursive: true, force: true }); }
}

test('refs refuse without exporting anything', async () => {
  await assert.rejects(cache.resolveUI('HEAD'), /Prepared UI worktree/);
});

test('startup errors are cleared before isolated widget errors are collected', async () => {
  const window = { __hoi4Parity: { isolate() {} }, __hoi4ParityBrowserErrors: ['startup'] };
  const page = { on() {}, setDefaultTimeout() {}, evaluate: (fn, arg) => runInNewContext(`(${fn})(arg)`, { window, arg, requestAnimationFrame: callback => callback() }) };
  const driver = app.createApp(page, { hour: 1 }); driver.errors.push({ text: 'startup' });
  await driver.install('research'); assert.deepEqual(driver.errors, []); assert.equal(window.__hoi4ParityBrowserErrors.length, 0);
});

test('bridge cleanup awaits the retained child close before staging can be removed', async t => {
  const { stripTypeScriptTypes } = await import('node:module');
  const { EventEmitter } = await import('node:events');
  const child = new EventEmitter(); child.stdin = Object.assign(new EventEmitter(), { write() {}, end() {} });
  child.stdout = new EventEmitter(); child.stderr = new EventEmitter(); child.exitCode = child.signalCode = null;
  let closed = false, middleware;
  child.kill = () => { setTimeout(() => { closed = true; child.emit('close'); }, 20); return true; };
  const source = stripTypeScriptTypes(await readFile(new URL('../browserBridge.ts', import.meta.url), 'utf8')).replace(/^import .*;$/gm, '').replace(/\bexport /g, '');
  const factory = runInNewContext(source + '\nbrowserBridge', { promisify, execFile() {}, path, spawn: () => child,
    savedReplayFolder: async folder => folder, readFile: async () => { throw Object.assign(Error(), { code: 'ENOENT' }); },
    createInterface: () => new EventEmitter(), MAX_MESSAGE_BYTES: 1024 * 1024,
    Buffer, URL, AbortController, setTimeout, clearTimeout, console });
  const plugin = factory('synthetic', 'synthetic', '', '');
  const server = new EventEmitter(); t.after(() => server.emit('close'));
  plugin.configureServer({ httpServer: server, middlewares: { use(_path, handler) { middleware = handler; } } });
  const response = Object.assign(new EventEmitter(), { setHeader() {}, end() { this.writableEnded = true; } });
  const request = { method: 'POST', url: '/', headers: { 'content-type': 'application/json' }, async *[Symbol.asyncIterator]() { yield Buffer.from('{"type":"product","request":{}}'); } };
  const sending = middleware(request, response);
  await new Promise(resolve => setImmediate(resolve));
  const stopping = plugin.closeBundle?.(); server.emit('close'); await stopping; assert.equal(closed, true); await sending;
});

test('adapter preserves source-change timestamps and private native verification writes', async () => {
  const { startUI } = await import('./server.mjs');
  const { repository } = await import('./settings.mjs');
  const { execFileSync } = await import('node:child_process');
  const { utimes } = await import('node:fs/promises');
  const root = await mkdtemp(path.join(tmpdir(), 'parity-source-time-'));
  const previousLocal = process.env.LOCALAPPDATA;
  const hashInput = path.join(root, 'input/HOI4Replay/viewer/content-hashes-v1.json');
  let server;
  try {
    await mkdir(path.dirname(hashInput), { recursive: true }); await writeFile(hashInput, '{"entries":[]}');
    process.env.LOCALAPPDATA = path.join(root, 'input');
    const sourceTime = Number(execFileSync('git', ['log', '-1', '--format=%ct', '--', 'crates/dashboard/src/reply_cache.rs'], { cwd: repository, encoding: 'utf8' }));
    const binary = path.join(root, 'release', 'dashboard-bridge.exe'); await mkdir(path.dirname(binary)); await writeFile(binary, 'synthetic release');
    await utimes(binary, sourceTime + 60.123456, sourceTime + 60.123456);
    const historyInput = path.join(root, 'input/HOI4Replay/viewer/history/recording/reply.zst');
    await mkdir(path.dirname(historyInput), { recursive: true }); await writeFile(historyInput, 'synthetic reply');
    const config = { cache: root, bridge: binary, shaderTools: root, nativeCache: path.join(root, 'native'), recording: path.join(root, 'synthetic.battleplan'), gameDir: '', userDir: '' };
    server = await startUI({ root: repository, commit: 'synthetic' }, config, path.join(root, 'runs', 'capture'));
    assert.ok((await stat(path.join(root, 'runs/capture/bridge/crates/dashboard/src/reply_cache.rs'))).mtimeMs <= (await stat(binary)).mtimeMs);
    assert.equal((await stat(path.join(root, 'runs/capture/bridge/.cargo-target/debug/dashboard-bridge.exe'), { bigint: true })).mtimeNs,
      (await stat(binary, { bigint: true })).mtimeNs, 'release build identity preserves fractional timestamp precision');
    const hashCopy = path.join(root, 'runs/capture/bridge/localappdata/HOI4Replay/viewer/content-hashes-v1.json');
    assert.equal(await readFile(hashCopy, 'utf8'), '{"entries":[]}');
    await writeFile(hashCopy, 'private update'); assert.equal(await readFile(hashInput, 'utf8'), '{"entries":[]}');
    const historyCopy = path.join(root, 'runs/capture/bridge/localappdata/HOI4Replay/viewer/history/recording/reply.zst');
    assert.equal(await readFile(historyCopy, 'utf8'), 'synthetic reply');
    await writeFile(historyCopy, 'private reply'); assert.equal(await readFile(historyInput, 'utf8'), 'synthetic reply');
  } finally {
    await server?.close();
    if (previousLocal === undefined) delete process.env.LOCALAPPDATA; else process.env.LOCALAPPDATA = previousLocal;
    await rm(root, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
  }
});

test('runner publishes comparison before its Job reaps Edge, without PID cleanup', async () => {
  const source = (await readFile(new URL('./run.mjs', import.meta.url), 'utf8')).replace(/^import .*;$/gm, '');
  const exits = [], phases = [];
  const browser = { newContext: async () => ({ newPage: async () => ({}), close: async () => {} }), close: async () => { throw Error('Unsafe PID fallback'); } };
  const context = { process: { env: { BATTLE_PLAN_OWNED_JOB: 'owned' }, argv: ['node', 'run', 'compare', 'base', 'head'], exit: code => exits.push(code) },
    fixture: { cache: 'cache', bridge: 'release' }, path, performance, randomUUID: () => 'owned',
    mkdir: async () => {}, writeFile: async () => {}, writeJSON: async () => {}, outsideRepository: value => value,
    resolveUI: async target => ({ root: target }), startUI: async () => ({ url: 'synthetic', close: async () => {} }),
    readMatrix: async () => null, captureCases: catalogue => catalogue.map(item => ({ id: item.kind, kind: item.kind })),
    chromium: {}, launchOwnedBrowser: async () => browser, contextOptions: {}, deterministicPage: async () => {},
    createApp: () => ({ prepare: async () => {}, registry: async () => [{ kind: 'synthetic' }] }),
    captureWidget: async () => ({ status: 'captured' }), compareCaptures: async () => ({ pass: true }),
    console: { log: value => { if (String(value).includes('phase')) phases.push(value); } } };
  await runInNewContext(`(async () => {${source}})()`, context);
  assert.equal(phases.length >= 2, true); assert.deepEqual(exits, [0]);
});
