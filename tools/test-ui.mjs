import { readFile, readdir, stat } from 'node:fs/promises';
import { dirname, extname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { run } from 'node:test';
import { spec } from 'node:test/reporters';
import { parseAst } from 'rolldown/parseAst';

export const repositoryRoot = fileURLToPath(new URL('../', import.meta.url));
const tabFullTest = 'ui/src/components/VirtualList.dom.test.ts';

// Add every browser fixture here. The dependency check below refuses omissions.
export const browserTests = [
  'ui/src/charts/shareDrawing.test.ts',
  'ui/src/charts/table/Table.dom.test.ts',
  'ui/src/widgets/dockyardSizes.dom.test.ts',
  'ui/src/components/CountryFlag.dom.test.ts',
  'ui/src/components/GameWindow.dom.test.ts',
  'ui/src/components/GameWindow.keyboard.dom.test.ts',
  'ui/src/components/Select.dom.test.ts',
  'ui/src/shell/appearance/WidgetIconMenu.dom.test.ts',
  'ui/src/widgets/economySizes.dom.test.ts',
  'ui/src/widgets/politicsSizes.dom.test.ts',
  tabFullTest,
];
const toolTests = ['tools/canvas-page.test.ts', 'tools/test-ui.test.mjs'];
const pureBrowserSupportTests = new Set(['tools/canvas-page.test.ts']);

export async function discoverTests(root = repositoryRoot) {
  async function visit(directory) {
    const entries = await readdir(resolve(root, directory), { withFileTypes: true });
    const nested = await Promise.all(entries.map(entry => {
      const path = `${directory}/${entry.name}`;
      return entry.isDirectory() ? visit(path) : path.endsWith('.test.ts') ? [path] : [];
    }));
    return nested.flat();
  }
  return [...await visit('ui/src'), ...toolTests].sort();
}

/** Inspect actual module dependencies rather than text inside synthetic fixtures. */
export async function checkBrowserManifest(files, manifest = browserTests, root = repositoryRoot) {
  const known = new Set(files);
  if (new Set(manifest).size !== manifest.length) throw new Error('Duplicate browser test in the serial manifest');
  for (const file of manifest) if (!known.has(file)) throw new Error(`Missing browser test from discovery: ${file}`);
  const modules = new Map();
  const browserPackage = /^(?:@playwright\/|@puppeteer\/|playwright(?:-core)?(?:\/|$)|puppeteer(?:-core)?(?:\/|$))/;
  async function localModule(base, reference) {
    if (!reference.startsWith('.')) return null;
    const path = resolve(dirname(base), reference);
    const candidates = extname(path) ? [path] : [path, ...['.ts', '.tsx', '.js', '.mjs'].map(extension => path + extension),
      ...['.ts', '.tsx', '.js', '.mjs'].map(extension => resolve(path, `index${extension}`))];
    for (const candidate of candidates) {
      if (!/\.(?:[cm]?js|tsx?)$/.test(candidate)) continue;
      try { if ((await stat(candidate)).isFile()) return candidate; } catch (error) {
        if (error.code !== 'ENOENT' && error.code !== 'ENOTDIR') throw error;
      }
    }
    return null;
  }
  async function inspect(path) {
    if (!modules.has(path)) modules.set(path, (async () => {
      const lang = path.endsWith('.tsx') ? 'tsx' : path.endsWith('.ts') ? 'ts' : 'js';
      const source = parseAst(await readFile(path, 'utf8'), { lang }, path);
      const references = new Set();
      let browser = path === resolve(root, 'tools/canvas-page.ts');
      function visit(node) {
        if ((node.type === 'ImportDeclaration' && node.importKind !== 'type')
          || (node.type.startsWith('Export') && node.exportKind !== 'type') || node.type === 'ImportExpression') {
          if (typeof node.source?.value === 'string') references.add(node.source.value);
        } else if (node.type === 'CallExpression' && node.callee.type === 'Identifier' && node.callee.name === 'require') {
          if (typeof node.arguments[0]?.value === 'string') references.add(node.arguments[0].value);
        }
        if (node.type === 'Literal' && typeof node.value === 'string'
          && /^(?:--headless(?:=|$)|--remote-debugging-port(?:=|$)|(?:msedge|chrome)\.exe$)/.test(node.value)) browser = true;
        for (const value of Object.values(node)) {
          if (Array.isArray(value)) for (const child of value) { if (child?.type) visit(child); }
          else if (value?.type) visit(value);
        }
      }
      visit(source);
      if ([...references].some(reference => browserPackage.test(reference))) browser = true;
      const dependencies = (await Promise.all([...references].map(reference => localModule(path, reference)))).filter(Boolean);
      return { browser, dependencies };
    })());
    return modules.get(path);
  }
  for (const file of files) {
    if (manifest.includes(file) || pureBrowserSupportTests.has(file)) continue;
    const pending = [resolve(root, file)], visited = new Set();
    while (pending.length) {
      const path = pending.pop();
      if (visited.has(path)) continue;
      visited.add(path);
      const module = await inspect(path);
      if (module.browser) throw new Error(`Browser test missing from the serial manifest: ${file} (via ${path})`);
      pending.push(...module.dependencies);
    }
  }
}

export function testGroups(files, manifest = browserTests, { tabFull = false } = {}) {
  if (tabFull) {
    if (!files.includes(tabFullTest)) throw new Error(`Missing native Tab fixture: ${tabFullTest}`);
    if (!manifest.includes(tabFullTest)) throw new Error(`Native Tab fixture is not in the serial manifest: ${tabFullTest}`);
    return [{ name: 'Full native Tab sweep', files: [tabFullTest], concurrency: 1, tabFull: true }];
  }
  const serial = new Set(manifest);
  return [
    { name: 'UI tests', files: files.filter(file => !serial.has(file)), concurrency: true },
    { name: 'Browser tests', files: files.filter(file => serial.has(file)), concurrency: 1 },
  ];
}

export async function executeGroup(group, root = repositoryRoot, output = process.stdout) {
  if (!group.files.length) return 0;
  const flags = ['BATTLE_PLAN_VIRTUAL_TAB_ONLY', 'BATTLE_PLAN_VIRTUAL_TAB_FULL'];
  const previous = flags.map(flag => process.env[flag]);
  // Node's test runner inherits its environment while spawning each file. Keep
  // the mode scoped until the entire serial group settles, then restore callers.
  for (const flag of flags) {
    if (group.tabFull) process.env[flag] = '1';
    else delete process.env[flag];
  }
  try {
    let failed = false;
    const tests = run({ files: group.files.map(file => resolve(root, file)), concurrency: group.concurrency,
      execArgv: ['--experimental-strip-types'] });
    tests.on('test:fail', () => { failed = true; });
    for await (const chunk of tests.compose(spec)) output.write(chunk);
    return failed ? 1 : 0;
  } finally {
    for (let i = 0; i < flags.length; i++) {
      if (previous[i] === undefined) delete process.env[flags[i]];
      else process.env[flags[i]] = previous[i];
    }
  }
}

/** Complete both groups, preserving a failure from either one. */
export async function runGroups(groups, execute = executeGroup) {
  let failed = false;
  for (const group of groups) {
    try { if (await execute(group)) failed = true; }
    catch (error) { console.error(error); failed = true; }
  }
  return failed ? 1 : 0;
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  const files = await discoverTests();
  await checkBrowserManifest(files);
  process.exitCode = await runGroups(testGroups(files, browserTests, { tabFull: process.argv.includes('--tab-full') }));
}
