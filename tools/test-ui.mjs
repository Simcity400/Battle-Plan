import { readFile, readdir, stat } from 'node:fs/promises';
import { dirname, extname, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { run } from 'node:test';
import { spec } from 'node:test/reporters';
import { parseAst } from 'rolldown/parseAst';

export const repositoryRoot = fileURLToPath(new URL('../', import.meta.url));
const tabFullTest = 'ui/src/components/VirtualList.dom.test.ts';

export const browserTests = [
  'ui/src/charts/heat/HeatMatrix.dom.test.ts',
  'ui/src/charts/live.dom.test.ts',
  'ui/src/charts/paintCost.dom.test.ts',
  'ui/src/charts/shareDrawing.test.ts',
  'ui/src/charts/table/Table.dom.test.ts',
  'ui/src/components/CountryFlag.dom.test.ts',
  'ui/src/components/DateTimePicker.dom.test.ts',
  'ui/src/components/loadable.dom.test.ts',
  'ui/src/components/MapControls.dom.test.ts',
  'ui/src/components/PlaybackSpeed.dom.test.ts',
  'ui/src/components/Select.dom.test.ts',
  'ui/src/dashboard/atHead.dom.test.ts',
  'ui/src/dashboard/follow/follow.dom.test.ts',
  'ui/src/dashboard/playbackBars.dom.test.ts',
  'ui/src/test-support/recordedPages.dom.test.ts',
  'ui/src/engine/armyRecorded.dom.test.ts',
  'ui/src/dashboard/prepareLane.dom.test.ts',
  'ui/src/recorder/RecorderPanel.dom.test.ts',
  'ui/src/shell/appearance/WidgetIconMenu.dom.test.ts',
  'ui/src/widgets/raid-log/raidPlayback.dom.test.ts',
  'ui/src/widgets/special-projects/specialProjects.dom.test.ts',
  tabFullTest,
];
const pureBrowserSupportTests = new Set(['tools/canvas-page.test.ts']);

export async function discoverTests(root = repositoryRoot) {
  async function visit(directory, pattern) {
    const entries = await readdir(resolve(root, directory), { withFileTypes: true });
    const nested = await Promise.all(entries.map(entry => {
      const path = `${directory}/${entry.name}`;
      if (entry.isDirectory()) return entry.name === 'node_modules' ? [] : visit(path, pattern);
      return pattern.test(entry.name) ? [path] : [];
    }));
    return nested.flat();
  }
  return [...await visit('ui/src', /\.test\.ts$/), ...await visit('tools', /\.test\.(?:ts|mjs)$/)].sort();
}

function namespaceHasValues(node) {
  if (node.type === 'TSModuleDeclaration') return node.body && namespaceHasValues(node.body);
  if (node.type === 'TSModuleBlock') return node.body.some(namespaceHasValues);
  if (node.type === 'ExportNamedDeclaration' && node.declaration) return namespaceHasValues(node.declaration);
  return node.type !== 'TSTypeAliasDeclaration' && node.type !== 'TSInterfaceDeclaration';
}

function nodeSyntaxRefusal(node) {
  switch (node.type) {
    case 'TSParameterProperty': return 'parameter properties require transformation';
    case 'TSEnumDeclaration': return 'enums require transformation';
    case 'TSModuleDeclaration':
      if (node.kind === 'module') return 'module declarations are not supported by Node';
      return namespaceHasValues(node) ? 'namespaces with values require transformation' : null;
    case 'TSImportEqualsDeclaration': return node.importKind === 'type' ? null : 'import aliases require transformation';
    case 'TSExportAssignment': return 'export assignments require transformation';
    case 'TSNamespaceExportDeclaration': return 'namespace exports are not supported by Node';
    case 'Decorator': return 'decorators are not supported by Node';
    case 'AccessorProperty': return 'auto-accessors are not supported by Node';
    default: return null;
  }
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
      const syntax = [];
      let browser = path === resolve(root, 'tools/canvas-page.ts');
      function reference(specifier, kind, start, attributes = []) {
        const key = `${kind}\0${specifier}`;
        if (!references.has(key)) references.set(key, { reference: specifier, kind, lines: [] });
        const named = attributes.map(({ key, value }) => `${key.name ?? key.value}=${value.value}`).sort().join(',');
        if (start !== undefined) references.get(key).lines.push({ line: text.slice(0, start).split('\n').length, attributes: named });
      }
      function visit(node) {
        if (node.declare || node.type === 'TSModuleDeclaration' && node.global) return;
        const refusal = nodeSyntaxRefusal(node);
        if (refusal) syntax.push({ line: text.slice(0, node.start).split('\n').length, refusal });
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
      return { browser, edges, syntax };
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

function nodeRefusal({ path, file }, attributes) {
  if (!extname(path)) return 'no extension';
  if (!file) return 'no such file';
  if (/\.[cm]?[jt]s$/.test(path)) return attributes ? 'a module takes no import attributes' : null;
  if (path.endsWith('.json')) return attributes === 'type=json' ? null : "JSON needs exactly with { type: 'json' }";
  return 'Node cannot load this file type';
}

export async function checkNodeImports(files, root = repositoryRoot, graph = moduleGraph(root)) {
  const violations = new Set();
  for await (const { path, edges, syntax } of graph.walk(files, true)) {
    const importer = relative(root, path).split(sep).join('/');
    for (const { line, refusal } of syntax) violations.add(`${importer}:${line}: ${refusal}`);
    for (const edge of edges) {
      if (!edge.reference.startsWith('.')) continue;
      for (const { line, attributes } of edge.lines) {
        const refusal = nodeRefusal(edge, attributes);
        if (refusal) violations.add(`${importer}:${line}: '${edge.reference}' (${refusal})`);
      }
    }
  }
  if (violations.size) throw new Error(`Node test modules must use imports and syntax Node can load with type stripping:\n${[...violations].sort().join('\n')}`);
}

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
    const only = process.argv.includes('--group') ? process.argv[process.argv.indexOf('--group') + 1] : null;
    if (only !== null && only !== 'node' && only !== 'browser') throw new Error(`--group takes node or browser, not ${only}`);
    const groups = testGroups(files, browserTests, { tabFull: process.argv.includes('--tab-full') });
    process.exitCode = await runGroups(groups.filter(group => only === null || group.concurrency === (only === 'node' ? true : 1)));
  }
}
