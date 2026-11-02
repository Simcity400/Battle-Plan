import { readFile, readdir, stat } from 'node:fs/promises';
import { dirname, extname, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { run } from 'node:test';
import { spec } from 'node:test/reporters';
import { parseAst } from 'rolldown/parseAst';

export const repositoryRoot = fileURLToPath(new URL('../', import.meta.url));
const tabFullTest = 'ui/src/components/VirtualList.dom.test.ts';

// Add every browser fixture here. The dependency check below refuses omissions.
export const browserTests = [
  'ui/src/charts/heat/HeatMatrix.dom.test.ts',
  'ui/src/charts/shareDrawing.test.ts',
  'ui/src/charts/table/Table.dom.test.ts',
  'ui/src/widgets/dockyardSizes.dom.test.ts',
  'ui/src/components/CountryFlag.dom.test.ts',
  'ui/src/components/GameWindow.dom.test.ts',
  'ui/src/components/GameWindow.keyboard.dom.test.ts',
  'ui/src/components/loadable.dom.test.ts',
  'ui/src/components/Select.dom.test.ts',
  'ui/src/recorder/RecorderPanel.dom.test.ts',
  'ui/src/shell/appearance/WidgetIconMenu.dom.test.ts',
  'ui/src/widgets/diplomacySizes.dom.test.ts',
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

function moduleGraph(root) {
  const modules = new Map();
  const resolutions = new Map();
  const browserPackage = /^(?:@playwright\/|@puppeteer\/|playwright(?:-core)?(?:\/|$)|puppeteer(?:-core)?(?:\/|$))/;
  async function isFile(path) {
    try { return (await stat(path)).isFile(); } catch (error) {
      if (error.code !== 'ENOENT' && error.code !== 'ENOTDIR') throw error;
      return false;
    }
  }
  async function localModule(base, reference, kind) {
    if (!reference.startsWith('.')) return { file: false, dependency: null };
    // As Node resolves each: an import against the importer's URL (no query or fragment, percent-decoded),
    // a require as a filesystem path.
    const path = kind === 'require' ? resolve(dirname(base), reference) : fileURLToPath(new URL(reference, pathToFileURL(base)));
    if (!resolutions.has(path)) resolutions.set(path, (async () => {
      const sourceFile = /\.(?:[cm]?[jt]s|tsx)$/;
      if (await isFile(path)) return { path, file: true, dependency: sourceFile.test(path) ? path : null };
      const extensions = ['.ts', '.tsx', '.mts', '.js', '.mjs'];
      const candidates = [...(extname(path) ? [] : extensions.map(extension => path + extension)),
        ...extensions.map(extension => resolve(path, `index${extension}`))];
      for (const candidate of candidates) {
        if (await isFile(candidate)) return { path, file: false, dependency: candidate };
      }
      return { path, file: false, dependency: null };
    })());
    return resolutions.get(path);
  }
  async function inspect(path) {
    if (!modules.has(path)) modules.set(path, (async () => {
      const lang = path.endsWith('.tsx') ? 'tsx' : /\.[cm]?ts$/.test(path) ? 'ts' : 'js';
      const text = await readFile(path, 'utf8');
      const source = parseAst(text, { lang }, path);
      const references = new Map();
      let browser = path === resolve(root, 'tools/canvas-page.ts');
      function reference(specifier, kind, start, attributes = []) {
        const key = `${kind}\0${specifier}`;
        if (!references.has(key)) references.set(key, { reference: specifier, kind, lines: [] });
        const named = attributes.map(({ key, value }) => `${key.name ?? key.value}=${value.value}`).sort().join(',');
        if (start !== undefined) references.get(key).lines.push({ line: text.slice(0, start).split('\n').length, attributes: named });
      }
      function visit(node) {
        if ((node.type === 'ImportDeclaration' && node.importKind !== 'type')
          || ((node.type === 'ExportNamedDeclaration' || node.type === 'ExportAllDeclaration') && node.exportKind !== 'type')) {
          if (typeof node.source?.value === 'string') reference(node.source.value, 'import', node.start, node.attributes);
        } else if (node.type === 'ImportExpression') {
          if (typeof node.source?.value === 'string') reference(node.source.value, 'import');
        } else if (node.type === 'CallExpression' && node.callee.type === 'Identifier' && node.callee.name === 'require') {
          if (typeof node.arguments[0]?.value === 'string') reference(node.arguments[0].value, 'require');
        }
        if (node.type === 'Literal' && typeof node.value === 'string'
          && /^(?:--headless(?:=|$)|--remote-debugging-port(?:=|$)|(?:msedge|chrome)\.exe$)/.test(node.value)) browser = true;
        for (const value of Object.values(node)) {
          if (Array.isArray(value)) for (const child of value) { if (child?.type) visit(child); }
          else if (value?.type) visit(value);
        }
      }
      visit(source);
      if ([...references.values()].some(({ reference }) => browserPackage.test(reference))) browser = true;
      const edges = await Promise.all([...references.values()].map(async edge =>
        ({ ...edge, ...await localModule(path, edge.reference, edge.kind) })));
      return { browser, edges };
    })());
    return modules.get(path);
  }
  async function* walk(files, staticOnly = false) {
    const pending = files.map(file => resolve(root, file)), visited = new Set();
    while (pending.length) {
      const path = pending.pop();
      if (visited.has(path)) continue;
      visited.add(path);
      const module = await inspect(path);
      yield { path, ...module };
      for (const edge of module.edges) {
        if (edge.dependency && (!staticOnly || edge.lines.length)) pending.push(edge.dependency);
      }
    }
  }
  return { walk };
}

/** Why Node's loader, stripping types, cannot load a relative static import; null when it can. */
function nodeRefusal({ path, file }, attributes) {
  if (!extname(path)) return 'no extension';
  if (!file) return 'no such file';
  if (/\.[cm]?[jt]s$/.test(path)) return attributes ? 'a module takes no import attributes' : null;
  if (path.endsWith('.json')) return attributes === 'type=json' ? null : "JSON needs exactly with { type: 'json' }";
  return 'Node cannot load this file type';
}

export async function checkNodeImports(files, root = repositoryRoot, graph = moduleGraph(root)) {
  const violations = new Set();
  for await (const { path, edges } of graph.walk(files, true)) {
    for (const edge of edges) {
      if (!edge.reference.startsWith('.')) continue;
      const importer = relative(root, path).split(sep).join('/');
      for (const { line, attributes } of edge.lines) {
        const refusal = nodeRefusal(edge, attributes);
        if (refusal) violations.add(`${importer}:${line}: '${edge.reference}' (${refusal})`);
      }
    }
  }
  if (violations.size) throw new Error(`Node test imports must name, with its extension, a file Node can load:\n${[...violations].sort().join('\n')}`);
}

/** Inspect actual module dependencies rather than text inside synthetic fixtures. */
export async function checkBrowserManifest(files, manifest = browserTests, root = repositoryRoot, graph = moduleGraph(root)) {
  const known = new Set(files);
  if (new Set(manifest).size !== manifest.length) throw new Error('Duplicate browser test in the serial manifest');
  for (const file of manifest) if (!known.has(file)) throw new Error(`Missing browser test from discovery: ${file}`);
  for (const file of files) {
    if (manifest.includes(file) || pureBrowserSupportTests.has(file)) continue;
    for await (const { path, browser } of graph.walk([file])) {
      if (browser) throw new Error(`Browser test missing from the serial manifest: ${file} (via ${path})`);
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
  const graph = moduleGraph(repositoryRoot);
  await checkNodeImports(files, repositoryRoot, graph);
  await checkBrowserManifest(files, browserTests, repositoryRoot, graph);
  if (!process.argv.includes('--check-imports')) {
    process.exitCode = await runGroups(testGroups(files, browserTests, { tabFull: process.argv.includes('--tab-full') }));
  }
}
