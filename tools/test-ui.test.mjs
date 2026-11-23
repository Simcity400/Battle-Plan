import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { browserTests, checkBrowserManifest, checkNodeImports, discoverTests, repositoryRoot, plannedGroups, runGroups, testArguments, testGroups } from './test-ui.mjs';

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
  assert.deepEqual(groups.map(group => group.deadlineMinutes), [20, undefined], 'a pure test file is stopped at a deadline; a browser fixture is never killed with Edge open');
  assert.deepEqual(groups[1].files.slice().sort(), browserTests.slice().sort());
  const scheduled = groups.flatMap(group => group.files);
  assert.equal(new Set(scheduled).size, files.length, 'no test is scheduled twice');
  assert.deepEqual(scheduled.slice().sort(), files, 'no test is omitted');
});

test('test:ui runs only the named files, in their own groups, and refuses names it cannot run', async () => {
  const files = await checkedTestFiles();
  const pure = 'tools/test-ui.test.mjs', browser = browserTests[0];
  const here = { root: repositoryRoot, cwd: repositoryRoot, caseless: false };
  const plan = (argv, location = here) => plannedGroups(files, testArguments(argv), location).map(group => group.files);
  assert.deepEqual(plan([pure, browser.split('/').join('\\'), resolve(repositoryRoot, pure)]), [[pure], [browser]],
    'separators and absolute paths name the same file once, in its own group');
  assert.deepEqual(plan(['test-ui.test.mjs'], { ...here, cwd: resolve(repositoryRoot, 'tools') }), [[pure], []]);
  assert.deepEqual(plan(['--group', 'browser', browser]), [[browser]]);
  assert.deepEqual(plan([browser.toUpperCase()], { ...here, caseless: true }), [[], [browser]], 'a caseless file system names the discovered spelling');
  assert.throws(() => plan([browser.toUpperCase()]), /Not a discovered UI test/);
  assert.throws(() => plan([pure, 'ui/src/missing.test.ts']), /Not a discovered UI test: ui\/src\/missing\.test\.ts$/);
  assert.throws(() => plan(['--group', 'browser', pure, browser]), /Not in the browser group: tools\/test-ui\.test\.mjs$/);
  assert.throws(() => testArguments(['--tab-full', pure]), /takes no test files/);
  assert.throws(() => testArguments(['--check-imports', pure]), /takes no test files/);
  assert.throws(() => testArguments(['--files', pure]), /Unknown option: --files/);
  assert.throws(() => testArguments(['--group', 'all']), /--group takes node or browser, not all/);
  assert.equal(plan([]).flat().length, files.length, 'no names runs every test');
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
  assert.equal((process.env.BATTLEPLAN_VIRTUAL_TAB_ONLY ?? process.env.HOI4_VIRTUAL_TAB_ONLY), (process.env.BATTLEPLAN_EXPECT_TAB_ONLY ?? process.env.HOI4_EXPECT_TAB_ONLY));
  assert.equal((process.env.BATTLEPLAN_VIRTUAL_TAB_FULL ?? process.env.HOI4_VIRTUAL_TAB_FULL), (process.env.BATTLEPLAN_EXPECT_TAB_FULL ?? process.env.HOI4_EXPECT_TAB_FULL));
});
`);
    const runner = pathToFileURL(resolve(repositoryRoot, 'tools/test-ui.mjs')).href;
    await writeFile(resolve(root, 'driver.mjs'), `
import { executeGroup } from ${JSON.stringify(runner)};
const original = [(process.env.BATTLEPLAN_VIRTUAL_TAB_ONLY ?? process.env.HOI4_VIRTUAL_TAB_ONLY), (process.env.BATTLEPLAN_VIRTUAL_TAB_FULL ?? process.env.HOI4_VIRTUAL_TAB_FULL)];
const results = [], restored = [];
const output = { write() {} };
for (const tabFull of [false, true, false]) {
  if (tabFull) {
    process.env.BATTLEPLAN_EXPECT_TAB_ONLY = '1';
    process.env.BATTLEPLAN_EXPECT_TAB_FULL = '1';
  } else {
    delete process.env.BATTLEPLAN_EXPECT_TAB_ONLY;
    delete process.env.BATTLEPLAN_EXPECT_TAB_FULL;
  }
  results.push(await executeGroup({ files: ['flags.test.mjs'], concurrency: 1, tabFull }, ${JSON.stringify(root)}, output));
  restored.push([(process.env.BATTLEPLAN_VIRTUAL_TAB_ONLY ?? process.env.HOI4_VIRTUAL_TAB_ONLY), (process.env.BATTLEPLAN_VIRTUAL_TAB_FULL ?? process.env.HOI4_VIRTUAL_TAB_FULL)]);
}
console.log(JSON.stringify({ results, original, restored }));
`);
    const env = { ...process.env, BATTLEPLAN_VIRTUAL_TAB_ONLY: 'inherited-only', BATTLEPLAN_VIRTUAL_TAB_FULL: 'inherited-full' };
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

test('the production group runner reports actual Node test failures, and a group stopped at its deadline names its unfinished files', async () => {
  const root = await mkdtemp(resolve(tmpdir(), 'hoi4-test-ui-results-'));
  try {
    await writeFile(resolve(root, 'pass.test.mjs'), `import assert from 'node:assert/strict'; import test from 'node:test';
test('synthetic pass', () => { assert.equal(process.cwd(), ${JSON.stringify(root)}, 'tests run from the root, wherever the runner was started'); });`);
    await writeFile(resolve(root, 'fail.test.mjs'), "import test from 'node:test'; test('synthetic failure', () => { throw new Error('expected synthetic failure'); });");
    await writeFile(resolve(root, 'leak.test.mjs'), `import test from 'node:test';
test('synthetic leak', () => {
  const { port1 } = new MessageChannel();
  port1.on('message', () => {});
  process.stdin.on('end', () => port1.close()).resume();
});`);
    const runner = pathToFileURL(resolve(repositoryRoot, 'tools/test-ui.mjs')).href;
    await writeFile(resolve(root, 'driver.mjs'), `
import { executeGroup } from ${JSON.stringify(runner)};
let reported = '';
const output = { write(chunk) { reported += chunk; } };
const passed = await executeGroup({ files: ['pass.test.mjs'], concurrency: true }, ${JSON.stringify(root)}, output);
const caller = process.cwd();
process.chdir(${JSON.stringify(dirname(root))});
const relativePassed = await executeGroup({ files: ['pass.test.mjs'], concurrency: true }, ${JSON.stringify(basename(root))}, output);
process.chdir(caller);
const passReport = reported;
reported = '';
const failed = await executeGroup({ files: ['fail.test.mjs'], concurrency: 1 }, ${JSON.stringify(root)}, output);
const failureReport = reported;
reported = '';
const controller = new AbortController();
const stopping = { write(chunk) { reported += chunk; if (chunk.includes('synthetic leak')) controller.abort(); } };
const stopped = await executeGroup({ name: 'Synthetic group', files: ['leak.test.mjs', 'pass.test.mjs'], concurrency: 1 }, ${JSON.stringify(root)}, stopping, controller.signal);
console.log(JSON.stringify({ passed, failed, passReport, failureReport, stopped, stopReport: reported, cwd: process.cwd(), relativePassed }));
`);
    const env = { ...process.env };
    delete env.NODE_TEST_CONTEXT;
    const { stdout } = await promisify(execFile)(process.execPath, [resolve(root, 'driver.mjs')], { env, cwd: repositoryRoot, timeout: driverTimeoutMs, windowsHide: true });
    const result = JSON.parse(stdout);
    assert.equal(resolve(result.cwd), resolve(repositoryRoot), 'the caller gets its own directory back');
    assert.equal(result.passed, 0, result.passReport);
    assert.equal(result.relativePassed, 0, 'a relative root names the same folder before and after changing into it');
    assert.match(result.passReport, /synthetic pass/);
    assert.equal(result.failed, 1);
    assert.match(result.failureReport, /expected synthetic failure/);
    assert.equal(result.stopped, 1, 'a group stopped at its deadline fails');
    assert.match(result.stopReport, /^Synthetic group hit its deadline while these files were still running: leak\.test\.mjs$/m,
      'the file holding its process open is named, and the queued one behind it is not');
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
  return new Error(['Node test modules must use imports and syntax Node can load with type stripping:', ...violations].join('\n'));
}

test('Node modules refuse parameter properties with their own file and line through imports and re-exports', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': "import './observer.mts';\nexport * from './barrel.ts';",
    'barrel.ts': "export * from './observer.mts';\nexport { Other } from './other.cts';",
    'observer.mts': 'export class Observer {\r\n  constructor(readonly callback: () => void) {}\r\n}',
    'other.cts': 'export class Other {\n  constructor(public value = 1, protected name: string) {}\n}',
  });
  await assert.rejects(checkNodeImports(['entry.test.ts', 'entry.test.ts'], root), nodeImportError(
    'observer.mts:2: parameter properties require transformation',
    'other.cts:2: parameter properties require transformation',
  ));
});

test('Node modules refuse runtime enums, namespaces and import/export aliases', async t => {
  const sources = {
    'enum': 'enum State { Ready }',
    'const-enum': 'const enum State { Ready }',
    'namespace': 'namespace State { export const ready = true; }',
    'nested-namespace': 'namespace State.Types { export class Ready {} }',
    'namespace-declare': 'namespace State { export declare const ready: boolean; }',
    'namespace-empty-statement': 'namespace State { ; }',
    'namespace-type-export': 'namespace State { export type { Ready }; }',
    'namespace-import-alias': 'namespace State { import Ready = Other.Ready; }',
    'module': 'module Types { export type Ready = boolean; }',
    'require-alias': "import value = require('./value.cjs');",
    'qualified-alias': 'import Ready = State.Ready;',
    'export-assignment': 'export = value;',
    'namespace-export': 'export as namespace State;',
  };
  const root = await importFixtures(t, Object.fromEntries([
    ['entry.test.ts', Object.keys(sources).map(name => `import './${name}.ts';`).join('\n')],
    ...Object.entries(sources).map(([name, source]) => [`${name}.ts`, `\n${source}`]),
  ]));
  await assert.rejects(checkNodeImports(['entry.test.ts'], root), nodeImportError(
    'const-enum.ts:2: enums require transformation',
    'enum.ts:2: enums require transformation',
    'export-assignment.ts:2: export assignments require transformation',
    'module.ts:2: module declarations are not supported by Node',
    'namespace-declare.ts:2: namespaces with values require transformation',
    'namespace-empty-statement.ts:2: namespaces with values require transformation',
    'namespace-export.ts:2: namespace exports are not supported by Node',
    'namespace-import-alias.ts:2: import aliases require transformation',
    'namespace-import-alias.ts:2: namespaces with values require transformation',
    'namespace-type-export.ts:2: namespaces with values require transformation',
    'namespace.ts:2: namespaces with values require transformation',
    'nested-namespace.ts:2: namespaces with values require transformation',
    'qualified-alias.ts:2: import aliases require transformation',
    'require-alias.ts:2: import aliases require transformation',
  ));
});

test('Node modules refuse decorators and auto-accessors that stripping leaves for the JavaScript parser', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': "import './decorated.ts';\nimport './accessor.mjs';",
    'decorated.ts': [
      '@decorate',
      'export class Model {',
      '  @decorate value: string;',
      '  constructor(@inject value: string) {}',
      '  accessor count: number = 0;',
      '}',
    ].join('\n'),
    'accessor.mjs': 'export class Model {\n  accessor count = 0;\n}',
  });
  await assert.rejects(checkNodeImports(['entry.test.ts'], root), nodeImportError(
    'accessor.mjs:2: auto-accessors are not supported by Node',
    'decorated.ts:1: decorators are not supported by Node',
    'decorated.ts:3: decorators are not supported by Node',
    'decorated.ts:4: decorators are not supported by Node',
    'decorated.ts:5: auto-accessors are not supported by Node',
  ));
});

test('Node modules accept erasable declarations, type-only namespaces and ordinary typed constructor parameters', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': [
      'declare enum State { Ready }',
      'declare namespace Ambient { enum State { Ready } }',
      'declare module "external" { export enum State { Ready } }',
      'namespace Types { export type Ready = boolean; export interface Item { ready: Ready } }',
      'namespace Nested.Types { export interface Item {} }',
      'namespace Empty {}',
      'import type External = require("external");',
      'abstract class Model { abstract accessor count: number; }',
      'class Observer { readonly callback: () => void; constructor(callback: () => void) { this.callback = callback; } }',
    ].join('\n'),
  });
  await checkNodeImports(['entry.test.ts'], root);
});

test('Node syntax checks leave Vite-only modules, type-only dependencies and synthetic source strings unconstrained', async t => {
  const root = await importFixtures(t, {
    'entry.test.ts': [
      "import type { Model } from './types.ts';",
      "const load = vite.ssrLoadModule('./view.ts');",
      "const fixture = 'enum State { Ready }; class C { constructor(readonly value: string) {} }';",
      '// namespace State { export const ready = true; }',
    ].join('\n'),
    'types.ts': 'export class Model { constructor(readonly value: string) {} }',
    'view.ts': 'enum State { Ready }; namespace Values { export const ready = State.Ready; }',
  });
  await checkNodeImports(['entry.test.ts'], root);
});

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

test('Node imports check every discovered root, including browser tests and any tools test by its name', async t => {
  const root = await importFixtures(t, {
    'ui/src/components/VirtualList.dom.test.ts': "import './missing';",
    'tools/canvas-page.test.ts': "import './missing.ts';",
    'tools/test-ui.test.mjs': "export * from './missing';",
    'tools/e2e/verdicts.test.mjs': '',
    'tools/e2e/playbackVerdicts.test.mjs': '',
    'tools/e2e/allWidgets.test.mjs': '',
    'tools/cargo-target.test.mjs': '',
    'tools/e2e/newVerdicts.test.mjs': "import './missing.mjs';",
    'tools/e2e/helper.mjs': "import './missing.mjs';",
    'tools/node_modules/package/a.test.mjs': "import './missing.mjs';",
  });
  await assert.rejects(checkNodeImports(await discoverTests(root), root), nodeImportError(
    "tools/canvas-page.test.ts:1: './missing.ts' (no such file)",
    "tools/e2e/newVerdicts.test.mjs:1: './missing.mjs' (no such file)",
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
