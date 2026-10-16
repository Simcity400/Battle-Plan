import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { browserTests, checkBrowserManifest, discoverTests, repositoryRoot, runGroups, testGroups } from './test-ui.mjs';

// These child drivers share CPU and module-loading time with the pure UI group.
const driverTimeoutMs = 30_000;
let checkedFiles;
function checkedTestFiles() {
  return checkedFiles ??= (async () => {
    const files = await discoverTests();
    await checkBrowserManifest(files);
    return files;
  })();
}

test('test:ui runs every file once, with normal pure concurrency and serial browsers', async () => {
  const packageJson = JSON.parse(await readFile(resolve(repositoryRoot, 'package.json'), 'utf8'));
  assert.equal(packageJson.scripts['test:ui'], 'node tools/test-ui.mjs', 'test:ui must use the checked two-group runner');
  const files = await checkedTestFiles();
  const groups = testGroups(files);
  assert.equal(groups[0].concurrency, true, 'pure tests use ordinary Node file concurrency');
  assert.equal(groups[1].concurrency, 1, 'browser fixtures have exclusive process ownership');
  assert.deepEqual(groups[1].files.slice().sort(), browserTests.slice().sort());
  const scheduled = groups.flatMap(group => group.files);
  assert.equal(new Set(scheduled).size, files.length, 'no test is scheduled twice');
  assert.deepEqual(scheduled.slice().sort(), files, 'no test is omitted');
});

test('test:ui:tab-full selects only the native Tab fixture with serial browser ownership', async () => {
  const packageJson = JSON.parse(await readFile(resolve(repositoryRoot, 'package.json'), 'utf8'));
  assert.equal(packageJson.scripts['test:ui:tab-full'], 'node tools/test-ui.mjs --tab-full');
  const files = await checkedTestFiles();
  const groups = testGroups(files, browserTests, { tabFull: true });
  assert.equal(groups.length, 1, 'the full sweep owns the only test group');
  assert.deepEqual(groups[0].files, ['ui/src/components/VirtualList.dom.test.ts']);
  assert.equal(groups[0].concurrency, 1, 'full differential sweep owns its browser serially');
  assert.equal(groups[0].tabFull, true, 'the fixture must receive both sweep flags');
  assert.throws(() => testGroups([], browserTests, { tabFull: true }), /Missing native Tab fixture/);
  assert.throws(() => testGroups(files, [], { tabFull: true }), /Native Tab fixture is not in the serial manifest/);
});

test('group execution isolates full-sweep flags and restores the invoking environment', async () => {
  const root = await mkdtemp(resolve(tmpdir(), 'hoi4-test-ui-tab-flags-'));
  try {
    await writeFile(resolve(root, 'flags.test.mjs'), `
import assert from 'node:assert/strict';
import test from 'node:test';
test('child receives isolated native sweep flags', () => {
  assert.equal((process.env.BATTLE_PLAN_VIRTUAL_TAB_ONLY ?? process.env.HOI4_VIRTUAL_TAB_ONLY), (process.env.BATTLE_PLAN_EXPECT_TAB_ONLY ?? process.env.HOI4_EXPECT_TAB_ONLY));
  assert.equal((process.env.BATTLE_PLAN_VIRTUAL_TAB_FULL ?? process.env.HOI4_VIRTUAL_TAB_FULL), (process.env.BATTLE_PLAN_EXPECT_TAB_FULL ?? process.env.HOI4_EXPECT_TAB_FULL));
});
`);
    const runner = pathToFileURL(resolve(repositoryRoot, 'tools/test-ui.mjs')).href;
    // The driver exercises actual isolated test children without opening a browser.
    await writeFile(resolve(root, 'driver.mjs'), `
import { executeGroup } from ${JSON.stringify(runner)};
const original = [(process.env.BATTLE_PLAN_VIRTUAL_TAB_ONLY ?? process.env.HOI4_VIRTUAL_TAB_ONLY), (process.env.BATTLE_PLAN_VIRTUAL_TAB_FULL ?? process.env.HOI4_VIRTUAL_TAB_FULL)];
const results = [], restored = [];
const output = { write() {} };
for (const tabFull of [false, true, false]) {
  if (tabFull) {
    process.env.BATTLE_PLAN_EXPECT_TAB_ONLY = '1';
    process.env.BATTLE_PLAN_EXPECT_TAB_FULL = '1';
  } else {
    delete process.env.BATTLE_PLAN_EXPECT_TAB_ONLY;
    delete process.env.BATTLE_PLAN_EXPECT_TAB_FULL;
  }
  results.push(await executeGroup({ files: ['flags.test.mjs'], concurrency: 1, tabFull }, ${JSON.stringify(root)}, output));
  restored.push([(process.env.BATTLE_PLAN_VIRTUAL_TAB_ONLY ?? process.env.HOI4_VIRTUAL_TAB_ONLY), (process.env.BATTLE_PLAN_VIRTUAL_TAB_FULL ?? process.env.HOI4_VIRTUAL_TAB_FULL)]);
}
console.log(JSON.stringify({ results, original, restored }));
`);
    const env = { ...process.env, BATTLE_PLAN_VIRTUAL_TAB_ONLY: 'inherited-only', BATTLE_PLAN_VIRTUAL_TAB_FULL: 'inherited-full' };
    delete env.NODE_TEST_CONTEXT;
    const { stdout } = await promisify(execFile)(process.execPath, [resolve(root, 'driver.mjs')], { env, timeout: driverTimeoutMs, windowsHide: true });
    const result = JSON.parse(stdout);
    assert.deepEqual(result.results, [0, 0, 0], 'default, full and default again receive their own flags');
    assert.deepEqual(result.restored, [result.original, result.original, result.original], 'group flags never leak into the caller');
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('both groups complete sequentially and failure of either group fails test:ui', async () => {
  const groups = testGroups(['pure.test.ts', 'browser.test.ts'], ['browser.test.ts']);
  for (const statuses of [[0, 0], [1, 0], [0, 1], [1, 1]]) {
    const completed = [];
    const result = await runGroups(groups, async group => {
      const index = completed.length;
      await Promise.resolve();
      completed.push(group);
      return statuses[index];
    });
    assert.deepEqual(completed, groups, 'the second group runs even if the first fails');
    assert.equal(result, statuses.some(Boolean) ? 1 : 0);
  }
});

test('the production group runner reports actual Node test failures', async () => {
  const root = await mkdtemp(resolve(tmpdir(), 'hoi4-test-ui-results-'));
  try {
    await writeFile(resolve(root, 'pass.test.mjs'), "import test from 'node:test'; test('synthetic pass', () => {});");
    await writeFile(resolve(root, 'fail.test.mjs'), "import test from 'node:test'; test('synthetic failure', () => { throw new Error('expected synthetic failure'); });");
    const runner = pathToFileURL(resolve(repositoryRoot, 'tools/test-ui.mjs')).href;
    // run() intentionally skips recursive use inside a test process. Exercise
    // its normal entry context in a short-lived Node driver, with no browser.
    await writeFile(resolve(root, 'driver.mjs'), `
import { executeGroup } from ${JSON.stringify(runner)};
let reported = '';
const output = { write(chunk) { reported += chunk; } };
const passed = await executeGroup({ files: ['pass.test.mjs'], concurrency: true }, ${JSON.stringify(root)}, output);
const passReport = reported;
reported = '';
const failed = await executeGroup({ files: ['fail.test.mjs'], concurrency: 1 }, ${JSON.stringify(root)}, output);
console.log(JSON.stringify({ passed, failed, passReport, failureReport: reported }));
`);
    const env = { ...process.env };
    delete env.NODE_TEST_CONTEXT;
    const { stdout } = await promisify(execFile)(process.execPath, [resolve(root, 'driver.mjs')], { env, timeout: driverTimeoutMs, windowsHide: true });
    const result = JSON.parse(stdout);
    assert.equal(result.passed, 0);
    assert.match(result.passReport, /synthetic pass/);
    assert.equal(result.failed, 1);
    assert.match(result.failureReport, /expected synthetic failure/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('an added browser test cannot silently enter the concurrent group', async () => {
  const root = await mkdtemp(resolve(tmpdir(), 'hoi4-test-ui-manifest-'));
  async function fixture(file, source) {
    await mkdir(resolve(root, file, '..'), { recursive: true });
    await writeFile(resolve(root, file), source);
  }
  try {
    await fixture('tools/canvas-page.ts', 'export function canvasPage() {}');
    await fixture('ui/src/browser.test.ts', "import { canvasPage } from '../../tools/canvas-page.ts';");
    await assert.rejects(checkBrowserManifest(['ui/src/browser.test.ts'], [], root), /Browser test missing from the serial manifest/);
    await fixture('ui/src/helper.ts', "export { canvasPage } from '../../tools/canvas-page.ts';");
    await fixture('ui/src/indirect.test.ts', "import { canvasPage } from './helper';");
    await assert.rejects(checkBrowserManifest(['ui/src/indirect.test.ts'], [], root), /Browser test missing from the serial manifest/);
    await fixture('ui/src/native.test.ts', "const args = ['--headless', '--user-data-dir=synthetic'];");
    await assert.rejects(checkBrowserManifest(['ui/src/native.test.ts'], [], root), /Browser test missing from the serial manifest/);
    await fixture('ui/src/playwright.test.ts', "import { chromium } from 'playwright';");
    await assert.rejects(checkBrowserManifest(['ui/src/playwright.test.ts'], [], root), /Browser test missing from the serial manifest/);
    await checkBrowserManifest(['ui/src/browser.test.ts'], ['ui/src/browser.test.ts'], root);
    await assert.rejects(checkBrowserManifest([], ['ui/src/browser.test.ts'], root), /Missing browser test from discovery/);
    await assert.rejects(checkBrowserManifest(['ui/src/browser.test.ts'], ['ui/src/browser.test.ts', 'ui/src/browser.test.ts'], root), /Duplicate browser test/);
    await fixture('ui/src/pure.test.ts', 'const fixture = "import browser from \'playwright\'; const args = [\'--headless\'];";');
    await checkBrowserManifest(['ui/src/pure.test.ts'], [], root);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
