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
    const source = path.join(root, 'input'), destination = path.join(root, 'adapter/BattlePlan/prepared');
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

test('configured parity capture forwards exact countries, time and options before paint', async () => {
  const tags = Array.from({ length: 5000 }, (_, index) => `opaque / tag ${index}`);
  const countries = { tags, kind: 'tags' }, time = { mode: 'follow' }, options = { measure: 'week' };
  const calls = [];
  const window = { __hoi4ParityBrowserErrors: [], __hoi4Parity: {
    isolate: async (...args) => { await Promise.resolve(); calls.push(args); },
  } };
  const page = { on() {}, setDefaultTimeout() {},
    evaluate: (fn, arg) => runInNewContext(`(${fn})(arg)`, { window, arg,
      requestAnimationFrame: callback => { calls.push('paint'); callback(); } }),
  };
  const driver = app.createApp(page, { hour: 42, scenario: { countries, time, options } });
  await driver.install('synthetic');
  assert.deepEqual(calls, [['synthetic', { height: 18, hour: 42, countries, time, options }], 'paint', 'paint']);
});

test('required exact model refuses absent provider and retains duplicate chart operands', async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'parity-required-model-'));
  try {
    let lists = [];
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

test('stale parents, reused PIDs and delayed observers cannot tree-kill', async () => {
  const scripts = JSON.parse(await readFile(new URL('../../package.json', import.meta.url), 'utf8')).scripts;
  assert.match(scripts['parity:compare'], /owned_job\.py/);
  const source = await readFile(new URL('./process-guard.cjs', import.meta.url), 'utf8'), requests = [];
  const foreign = () => assert.fail('Foreign PID reached execution');
  const cp = Object.fromEntries(['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync'].map(name => [name, foreign]));
  cp.execFile = Object.assign(() => {}, { [promisify.custom]: foreign });
  const process = { env: { BATTLEPLAN_OWNED_JOB: 'owned', BATTLEPLAN_JOB_STOP_FILE: 'owned' }, kill: foreign };
  runInNewContext(source, { require: name => name === 'node:child_process' ? cp : name === 'node:fs' ? { writeFileSync: () => requests.push('job') }
    : name === 'node:util' ? { promisify } : { syncBuiltinESMExports() {} }, process, Error });
  for (const state of ['stale parent', 'reused PID', 'delayed observer']) {
    assert.throws(() => cp.spawnSync('taskkill /pid 42 /T /F'), /owned Job/, state);
    assert.throws(() => process.kill(42), /PID-only/, state);
  }
  assert.throws(() => cp.execFile[promisify.custom]('taskkill.exe', ['/pid', '42', '/T', '/F']), /owned Job/);
  assert.equal(requests.length, 4);

});

test('captures stay out of Git: outside the checkout or in its ignored captures folder', async () => {
  const { repository } = await import('./settings.mjs');
  const inside = path.join(repository, 'captures', 'parity-baselines');
  assert.equal(cache.outsideRepository(inside), path.resolve(inside));
  assert.throws(() => cache.outsideRepository(path.join(repository, 'ui', 'captures')), /outside Git/);
  assert.throws(() => cache.outsideRepository(repository), /outside Git/);
  assert.equal(cache.outsideRepository(tmpdir()), path.resolve(tmpdir()));
});

test('incomplete instrumentation and mismatched installed dependencies refuse', async () => {
  await assert.rejects(app.validateHook({ evaluate: async fn => { globalThis.window = { __hoi4Parity: { lists() {} } }; try { await fn(); } finally { delete globalThis.window; } } }), /capabilities/);
  const root = await mkdtemp(path.join(tmpdir(), 'parity-lock-check-'));
  try { await symlink(new URL('../../node_modules', import.meta.url).pathname.replace(/^\/(.:)/, '$1'), path.join(root, 'node_modules'), 'junction'); await writeFile(path.join(root, 'package-lock.json'), '{}'); await assert.rejects(cache.validateDependencyLock(root), /dependency lock/); }
  finally { await rm(root, { recursive: true, force: true }); }
});

test('disposable adapter refuses debug input and its teardown removes it', async () => {
  const { startUI, removeAdapter } = await import('./server.mjs');
  const root = await mkdtemp(path.join(tmpdir(), 'parity-adapter-'));
  try {
    await assert.rejects(startUI({}, { bridge: path.join(root, 'debug/dashboard-bridge.exe') }, root), /debug/);
    const adapter = path.join(root, 'bridge'); await mkdir(path.join(adapter, '.cargo-target/debug'), { recursive: true });
    await writeFile(path.join(adapter, '.cargo-target/debug/dxcompiler.dll'), 'copy');
    await removeAdapter(adapter);
    assert.equal(await stat(adapter).catch(() => null), null);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('fresh owned Edge contexts preserve exact pixels and 5,000 rows, captured once loading clears', () => {
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
        await context.clock.install({ time: 0 });
        await context.clock.pauseAt(1_000);
        const page = await context.newPage();
        await page.setContent(`<img src="${source}" width="${index ? 85 : 16}">`); await page.locator('img').evaluate(img => img.decode());
        await page.setContent(`<style>body{margin:0}.dash-widget{width:350px;height:1400px;background:#234567}img{width:18.5px;height:18.5px}</style>
          <article class="dash-widget"><header class="dash-widget-header"><h2>Synthetic</h2></header><div class="dash-widget-scaled"><img src="${source}"><span role="status" data-parity-loading>Loading widget?</span></div></article>`);
        await page.locator('img').evaluate(img => img.decode());
        const lists = [{ complete: true, total: 5000, rows: Array.from({ length: 5000 }, (_, index) => ({ index, key: String(index), value: index * 2 })) }];
        const real = app.createApp(page, {});
        let settled = false, settling;
        const started = new Promise(resolve => { settling = resolve; });
        const settle = async locator => { settling(); await real.settle(locator); settled = true; };
        const driver = { ...real, settle, prepare: async () => {}, install: async () => {}, widget: () => page.locator('.dash-widget'), prepareFullRows: async () => {}, fullRows: async () => lists };
        await page.evaluate(() => {
          let samples = 0;
          Object.defineProperty(window, '__hoi4ParityBrowserErrors', { configurable: true, get: () => { samples++; return []; } });
          window.__settleSamples = () => samples;
        });
        const capturing = captureWidget({ page, app: driver, kind: 'synthetic', directory });
        await started;
        const samples = () => page.evaluate(() => window.__settleSamples());
        for (let tick = 0; await samples() === 0; tick++) {
          assert.ok(tick < 10_000, 'the capture never sampled the widget');
          await page.clock.runFor(16);
        }
        const before = await samples();
        await page.clock.runFor(1_000);
        await page.evaluate(() => 0);
        assert.equal(settled, false, 'a loading marker holds the capture past a full stability interval');
        assert.ok(await samples() - before >= 50, 'the capture sampled the widget through a full stability interval');
        await page.evaluate(() => {
          const fallback = document.querySelector('[data-parity-loading]');
          fallback.removeAttribute('data-parity-loading'); fallback.textContent = 'Rows';
        });
        await page.clock.runFor(256);
        await page.clock.resume();
        const item = await capturing;
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
  } finally { await rm(root, { recursive: true, force: true }); }
}

test('a stalled capture fails when its budget runs out, closes its page and publishes the error', async t => {
  const root = await mkdtemp(path.join(tmpdir(), 'parity-stall-'));
  t.mock.timers.enable({ apis: ['setTimeout'] });
  try {
    let closed = false, settled = false;
    const pending = captureWidget({ page: { close: async () => { closed = true; } }, app: { prepare: () => new Promise(() => {}) }, kind: 'stall', directory: root, budgetMs: 20 });
    void pending.then(() => { settled = true; });
    t.mock.timers.tick(19);
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual([settled, closed], [false, false], 'inside its budget the capture waits');
    t.mock.timers.tick(1);
    const failure = await pending;
    assert.equal(failure.status, 'error'); assert.match(failure.error, /budget exceeded/); assert.equal(closed, true);
    assert.equal(JSON.parse(await readFile(path.join(root, 'stall.json'))).status, 'error');
  } finally {
    t.mock.timers.reset();
    await rm(root, { recursive: true, force: true });
  }
});

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
  const commands = [];
  const child = new EventEmitter(); child.stdin = Object.assign(new EventEmitter(), { write: line => commands.push(JSON.parse(line)), end() {} });
  child.stdout = new EventEmitter(); child.stderr = new EventEmitter(); child.exitCode = child.signalCode = null;
  const removed = [];
  let killed = false, middleware;
  child.kill = () => { killed = true; return true; };
  const source = stripTypeScriptTypes(await readFile(new URL('../browserBridge.ts', import.meta.url), 'utf8')).replace(/^import .*;$/gm, '').replace(/\bexport /g, '');
  const factory = runInNewContext(source + '\nbrowserBridge', { promisify, execFile() {}, path, spawn: () => child,
    savedReplayFolder: async folder => folder, process: { pid: 1, env: {} }, tmpdir, randomUUID: () => 'synthetic',
    readdir: async () => [], rm: async target => { removed.push(target); },
    createInterface: () => new EventEmitter(), MAX_MESSAGE_BYTES: 1024 * 1024, BROWSER_ARTWORK_BASE: '/api/artwork/',
    asFailure: error => error, failureWire: error => ({ message: error.message }),
    Buffer, URL, AbortController, setTimeout, clearTimeout, console });
  const plugin = factory('synthetic', 'synthetic', '', '');
  const server = new EventEmitter();
  t.after(() => { server.emit('close'); child.emit('close'); });
  plugin.configureServer({ httpServer: server, middlewares: { use(route, handler) { if (route === '/api/recordings') middleware = handler; } } });
  const response = Object.assign(new EventEmitter(), { setHeader() {}, end() { this.writableEnded = true; } });
  const request = { method: 'POST', url: '/', headers: { 'content-type': 'application/json' }, async *[Symbol.asyncIterator]() { yield Buffer.from('{"type":"product","request":{}}'); } };
  const sending = middleware(request, response);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(commands[0]?.type, 'product');
  let stopped = false;
  const stopping = plugin.closeBundle().then(() => { stopped = true; });
  server.emit('close');
  await sending;
  assert.equal(killed, true);
  assert.equal(stopped, false);
  assert.deepEqual(removed, []);
  child.emit('close');
  await stopping;
  assert.deepEqual(removed, [path.join(tmpdir(), 'battleplan-imports', '1-synthetic')]);
});

test('adapter preserves source-change timestamps and private native verification writes', async () => {
  const { startUI } = await import('./server.mjs');
  const { repository } = await import('./settings.mjs');
  const { execFileSync } = await import('node:child_process');
  const { utimes } = await import('node:fs/promises');
  const root = await mkdtemp(path.join(tmpdir(), 'parity-source-time-'));
  const previousLocal = process.env.LOCALAPPDATA;
  const hashInput = path.join(root, 'input/BattlePlan/viewer/content-hashes-v2.bin');
  let server;
  try {
    await mkdir(path.dirname(hashInput), { recursive: true }); await writeFile(hashInput, 'synthetic hash cache');
    process.env.LOCALAPPDATA = path.join(root, 'input');
    const marker = 'crates/dashboard-core/src/map_settings.rs';
    const git = args => execFileSync('git', args, { cwd: repository, encoding: 'utf8' }).trim();
    const sourceTime = git(['status', '--porcelain', '--', marker])
      ? (await stat(path.join(repository, marker))).mtimeMs
      : Number(git(['log', '-1', '--format=%ct', '--', marker])) * 1000;
    assert.ok(sourceTime > 0);
    const release = path.join(root, 'release');
    const binary = path.join(release, 'dashboard-bridge.exe');
    await mkdir(release);
    await writeFile(binary, 'synthetic release');
    for (const name of ['dxcompiler.dll', 'dxil.dll']) await writeFile(path.join(release, name), name);
    const buildTime = sourceTime / 1000 + 60.123456;
    await utimes(binary, buildTime, buildTime);
    const historyInput = path.join(root, 'input/BattlePlan/viewer/history/recording/reply.zst');
    await mkdir(path.dirname(historyInput), { recursive: true }); await writeFile(historyInput, 'synthetic reply');
    const config = { bridge: binary, recording: path.join(root, 'synthetic.battleplan'), gameDir: '', userDir: '' };
    server = await startUI({ root: repository, commit: 'synthetic' }, config, path.join(root, 'runs', 'capture'));
    const adapter = path.join(root, 'runs/capture/bridge');
    const debug = path.join(adapter, '.cargo-target/debug');
    assert.equal((await stat(path.join(adapter, marker))).mtimeMs, sourceTime);
    assert.equal((await stat(path.join(debug, 'dashboard-bridge.exe'), { bigint: true })).mtimeNs,
      (await stat(binary, { bigint: true })).mtimeNs, 'release build identity preserves fractional timestamp precision');
    for (const name of ['dxcompiler.dll', 'dxil.dll']) assert.equal(await readFile(path.join(debug, name), 'utf8'), name);
    const hashCopy = path.join(adapter, 'localappdata/BattlePlan/viewer/content-hashes-v2.bin');
    assert.equal(await readFile(hashCopy, 'utf8'), 'synthetic hash cache');
    await writeFile(hashCopy, 'private update'); assert.equal(await readFile(hashInput, 'utf8'), 'synthetic hash cache');
    const historyCopy = path.join(adapter, 'localappdata/BattlePlan/viewer/history/recording/reply.zst');
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
  const context = { process: { env: { BATTLEPLAN_OWNED_JOB: 'owned' }, argv: ['node', 'run', 'compare', 'base', 'head'], exit: code => exits.push(code) },
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
