import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { browserTests, checkBrowserManifest, checkNodeImports, discoverTests, repositoryRoot, runGroups, testGroups } from './test-ui.mjs';

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
  assert.equal(packageJson.scripts['check:ui'], 'tsc --noEmit -p ui/tsconfig.json && node tools/test-ui.mjs --check-imports');
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

async function importFixtures(t, files) {
  const root = await mkdtemp(resolve(tmpdir(), 'hoi4-test-ui-imports-'));
  t.after(() => rm(root, { recursive: true }));
  await Promise.all(Object.entries(files).map(async ([file, source]) => {
    await mkdir(resolve(root, file, '..'), { recursive: true });
    await writeFile(resolve(root, file), source);
  }));
  return root;
}

function nodeImportError(...violations) {
  return new Error(['Node test imports must name, with its extension, a file Node can load:', ...violations].join('\n'));
}

test('Node imports refuse extensionless static imports with the importing file and line', async t => {
  const root = await importFixtures(t, {
    'ui/entry.test.ts': "const ready = true;\r\nimport { value } from './a';",
    'ui/a.ts': 'export const value = 1;',
  });
  await assert.rejects(checkNodeImports(['ui/entry.test.ts'], root),
    nodeImportError("ui/entry.test.ts:2: './a' (no extension)"));
});

test('Node imports refuse missing files and directories even when they have extensions', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': "import './missing.ts';\nimport './directory.js';",
    'directory.js/index.ts': 'export const value = 1;',
  });
  await assert.rejects(checkNodeImports(['entry.test.ts'], root), nodeImportError(
    "entry.test.ts:1: './missing.ts' (no such file)",
    "entry.test.ts:2: './directory.js' (no such file)",
  ));
});

test('Node imports resolve specifiers as URLs, without query or fragment and percent-decoded', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': "import './helper.mjs?fresh';\nimport './helper.mjs#part';\nimport './with%20space.ts';",
    'helper.mjs': 'export const value = 1;',
    'with space.ts': "import './behind';",
  });
  await assert.rejects(checkNodeImports(['entry.test.ts'], root),
    nodeImportError("with space.ts:1: './behind' (no extension)"));
});

test('Node imports follow .mts modules, and refuse files Node cannot load', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': "import './a.mts';\nimport './styles.css';\nimport './Widget.tsx';",
    'a.mts': "export * from './leaf';",
    'styles.css': '',
    'Widget.tsx': 'export default () => null;',
  });
  await assert.rejects(checkNodeImports(['entry.test.ts'], root), nodeImportError(
    "a.mts:1: './leaf' (no extension)",
    "entry.test.ts:2: './styles.css' (Node cannot load this file type)",
    "entry.test.ts:3: './Widget.tsx' (Node cannot load this file type)",
  ));
});

test('Node imports accept JSON only with exactly its import attribute, and modules only without one', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': [
      "import data from './data.json' with { type: 'json' };",
      "import bare from './data.json';",
      "export { default as again } from './data.json' with { 'type': 'json' };",
      "import extra from './data.json' with { type: 'json', mode: 'x' };",
      "import './helper.mjs' with { type: 'json' };",
      "import './helper.mjs';",
    ].join('\n'),
    'data.json': '{}',
    'helper.mjs': 'export const value = 1;',
  });
  await assert.rejects(checkNodeImports(['entry.test.ts'], root), nodeImportError(
    "entry.test.ts:2: './data.json' (JSON needs exactly with { type: 'json' })",
    "entry.test.ts:4: './data.json' (JSON needs exactly with { type: 'json' })",
    "entry.test.ts:5: './helper.mjs' (a module takes no import attributes)",
  ));
});

test('a require edge resolves as a filesystem path, so the browser manifest still sees what it loads', async () => {
  const root = await mkdtemp(resolve(tmpdir(), 'hoi4-test-ui-require-'));
  try {
    await writeFile(resolve(root, 'entry.test.ts'), "const require = createRequire(import.meta.url);\nrequire('./browser#helper.cjs');");
    await writeFile(resolve(root, 'browser#helper.cjs'), "require('playwright-core');");
    await assert.rejects(checkBrowserManifest(['entry.test.ts'], [], root),
      /Browser test missing from the serial manifest: entry\.test\.ts/);
  } finally {
    await rm(root, { recursive: true });
  }
});

test('Node imports neither refuse nor follow type-only imports and re-exports', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': "import type { Missing } from './missing';\nexport type { X } from './types';\nexport type * from './other-missing.ts';",
    'types.ts': "import './missing-runtime';\nexport type X = string;",
  });
  await checkNodeImports(['entry.test.ts'], root);
});

test('Node imports ignore dynamic import() and require edges', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': "const load = () => import('./lazy');\nconst missing = () => import('./missing.ts');\nconst required = require('./required');",
    'lazy.ts': "import './behind-lazy';",
    'required.ts': "import './behind-require';",
  });
  await checkNodeImports(['entry.test.ts'], root);
});

test('Node imports treat specifier-level type imports as runtime edges', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': "import { type X } from './a';",
    'a.ts': 'export type X = string;',
  });
  await assert.rejects(checkNodeImports(['entry.test.ts'], root),
    nodeImportError("entry.test.ts:1: './a' (no extension)"));
});

test('Node imports refuse extensionless star and named runtime re-exports', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': "export * from './a';\nexport { value } from './a';\nexport { type X } from './a';",
    'a.ts': 'export const value = 1; export type X = string;',
  });
  await assert.rejects(checkNodeImports(['entry.test.ts'], root), nodeImportError(
    "entry.test.ts:1: './a' (no extension)",
    "entry.test.ts:2: './a' (no extension)",
    "entry.test.ts:3: './a' (no extension)",
  ));
});

test('Node imports collect sorted unique violations behind unresolved imports and directory indexes', async t => {
  const root = await importFixtures(t, {
    'z.test.ts': "import './shared';",
    'a.test.ts': "export * from './shared';",
    'shared.ts': "import './nested';",
    'nested/index.tsx': "export * from '../shared.ts';\nimport './missing.ts';",
  });
  await assert.rejects(checkNodeImports(['z.test.ts', 'a.test.ts', 'a.test.ts'], root), nodeImportError(
    "a.test.ts:1: './shared' (no extension)",
    "nested/index.tsx:2: './missing.ts' (no such file)",
    "shared.ts:1: './nested' (no extension)",
    "z.test.ts:1: './shared' (no extension)",
  ));
});

test('Node imports check every discovered root, including browser and tools tests', async t => {
  const root = await importFixtures(t, {
    'ui/src/components/VirtualList.dom.test.ts': "import './missing';",
    'tools/canvas-page.test.ts': "import './missing.ts';",
    'tools/test-ui.test.mjs': "export * from './missing';",
  });
  await assert.rejects(checkNodeImports(await discoverTests(root), root), nodeImportError(
    "tools/canvas-page.test.ts:1: './missing.ts' (no such file)",
    "tools/test-ui.test.mjs:1: './missing' (no extension)",
    "ui/src/components/VirtualList.dom.test.ts:1: './missing' (no extension)",
  ));
});

test('Node imports accept a clean graph with explicit extensions, cycles and external packages', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': "import './a.ts';\nexport * from './b.js';\nexport { value } from './c.mjs';\nimport 'node:fs';\nimport 'external-package';",
    'a.ts': "export * from './b.js';",
    'b.js': "import './a.ts';\nexport const value = 1;",
    'c.mjs': "export { value } from './b.js';",
  });
  await checkNodeImports(['entry.test.ts'], root);
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
    await fixture('ui/src/lazy.test.ts', "const load = () => import('./helper');");
    await assert.rejects(checkBrowserManifest(['ui/src/lazy.test.ts'], [], root), /Browser test missing from the serial manifest/);
    await fixture('ui/src/require.test.ts', "const helper = require('./helper');");
    await assert.rejects(checkBrowserManifest(['ui/src/require.test.ts'], [], root), /Browser test missing from the serial manifest/);
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
