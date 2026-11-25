import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, extname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseAst } from 'rolldown/parseAst';
import { astLang, relPosix, staticStringLiteral, walkAst, walkImportGraph } from './ui-import-graph.mjs';
import { analyzeUiSymbolGraph } from './ui-ts-graph.mjs';

export const UI_FILE_RULES = [
  {
    role: 'ambient',
    pattern: /^ui\/src\/.+\.d\.ts$/,
    reason: 'TypeScript ambient declaration files loaded by ui/tsconfig.json without explicit imports',
  },
  {
    role: 'test',
    pattern: /^ui\/src\/.+\.test\.tsx?$/,
    reason: 'UI test entry points executed by tools/test-ui.mjs',
  },
  {
    role: 'fixture',
    pattern: /^ui\/src\/.+\.fixtures?\.tsx?$/,
    reason: 'UI test fixture modules loaded by UI tests',
  },
  {
    role: 'test-support',
    pattern: /^ui\/src\/test-support\/.+$/,
    reason: 'Shared UI test harness modules imported by UI tests',
  },
];

const UI_CHECKED_EXTENSIONS = new Set(['.ts', '.tsx', '.css']);
const SCRIPT_EXTENSIONS = new Set(['.ts', '.tsx', '.mts', '.js', '.mjs', '.cjs']);
const TEXT_EXTENSIONS = new Set(['.ts', '.tsx', '.js', '.mjs', '.cjs', '.rs', '.py', '.ps1', '.json', '.toml', '.html', '.css']);
const TOOL_CALLER_EXTENSIONS = new Set(['.mjs', '.ts', '.js', '.cjs', '.py', '.ps1', '.json', '.toml']);
const NON_CALLER_UI_FILES = new Set([
  'ui/src/dashboard/pages.ts',
  'ui/src/widgets/manifest.ts',
  'ui/src/widgets/familyManifest.ts',
]);
const NON_PRODUCT_PROPERTY_NAMES = new Set(['kind', 'view', 'owner', 'className', 'icon', 'category', 'family']);

function isNonDashboardRustCaller(rel) {
  if (!rel.startsWith('crates/') || extname(rel).toLowerCase() !== '.rs') return false;
  const crateName = rel.split('/')[1] ?? '';
  return !crateName.startsWith('dashboard');
}

const BASE_SNAPSHOT_RULES = [
  { pathspec: 'ui/src', match: rel => rel.startsWith('ui/src/') },
  { pathspec: 'ui/tsconfig.json', match: rel => rel === 'ui/tsconfig.json' },
  { pathspec: 'ui/vite.config.ts', match: rel => rel === 'ui/vite.config.ts' },
  { pathspec: 'ui/browserBridge.ts', match: rel => rel === 'ui/browserBridge.ts' },
  { pathspec: 'ui/replayLibrary.ts', match: rel => rel === 'ui/replayLibrary.ts' },
  { pathspec: ':(glob)ui/*.html', match: rel => /^ui\/[^/]+\.html$/.test(rel) },
  { pathspec: 'crates/dashboard/src/products/mod.rs', match: rel => rel === 'crates/dashboard/src/products/mod.rs' },
  { pathspec: ':(glob)crates/dashboard-*/src/registrations.rs', match: rel => /^crates\/dashboard-[^/]+\/src\/registrations\.rs$/.test(rel) },
  { pathspec: ':(glob)crates/**/*.rs', match: isNonDashboardRustCaller },
  { pathspec: ':(glob)tools/**/*.mjs', match: rel => rel.startsWith('tools/') && rel.endsWith('.mjs') },
  { pathspec: ':(glob)tools/**/*.ts', match: rel => rel.startsWith('tools/') && rel.endsWith('.ts') },
  { pathspec: ':(glob)tools/**/*.js', match: rel => rel.startsWith('tools/') && rel.endsWith('.js') },
  { pathspec: ':(glob)tools/**/*.cjs', match: rel => rel.startsWith('tools/') && rel.endsWith('.cjs') },
  { pathspec: ':(glob)tools/**/*.py', match: rel => rel.startsWith('tools/') && rel.endsWith('.py') },
  { pathspec: ':(glob)tools/**/*.ps1', match: rel => rel.startsWith('tools/') && rel.endsWith('.ps1') },
  { pathspec: ':(glob)tools/**/*.json', match: rel => rel.startsWith('tools/') && rel.endsWith('.json') },
  { pathspec: ':(glob)tools/**/*.toml', match: rel => rel.startsWith('tools/') && rel.endsWith('.toml') },
];

export const BASE_SNAPSHOT_PATHSPECS = BASE_SNAPSHOT_RULES.map(rule => rule.pathspec);

function walkDirectory(dirPath, root, predicate, out = []) {
  if (!existsSync(dirPath)) return out;
  for (const entry of readdirSync(dirPath, { withFileTypes: true })) {
    if (entry.name === 'node_modules' || entry.name === 'dist' || entry.name === 'target' || entry.name.startsWith('.')) continue;
    const full = resolve(dirPath, entry.name);
    if (entry.isDirectory()) {
      walkDirectory(full, root, predicate, out);
    } else {
      const rel = relPosix(root, full);
      if (predicate(rel, full)) out.push({ rel, full });
    }
  }
  return out;
}

export function classifyUiFile(relPath) {
  for (const rule of UI_FILE_RULES) {
    if (rule.pattern.test(relPath)) return rule.role;
  }
  return 'production';
}

function unwrapExpression(node) {
  let cur = node;
  while (cur && (cur.type === 'TSAsExpression' || cur.type === 'TSSatisfiesExpression' || cur.type === 'ParenthesizedExpression' || cur.type === 'TSNonNullExpression')) {
    cur = cur.expression;
  }
  return cur;
}

export function discoverHtmlEntryPoints(root) {
  const viteConfigPath = resolve(root, 'ui/vite.config.ts');
  if (existsSync(viteConfigPath)) {
    const ast = parseAst(readFileSync(viteConfigPath, 'utf8'), { lang: 'ts' });
    const found = new Set();
    walkAst(ast, node => {
      if (node.type !== 'Property') return;
      const keyName = node.key?.name ?? node.key?.value;
      if (keyName !== 'rolldownOptions' && keyName !== 'rollupOptions') return;
      const optObj = unwrapExpression(node.value);
      if (optObj?.type !== 'ObjectExpression') return;
      for (const prop of optObj.properties) {
        if (prop.type !== 'Property') continue;
        const propName = prop.key?.name ?? prop.key?.value;
        if (propName !== 'input') continue;
        walkAst(prop.value, inner => {
          const lit = staticStringLiteral(inner);
          if (lit && lit.endsWith('.html')) {
            const candidate = resolve(root, 'ui', lit);
            if (existsSync(candidate)) found.add(relPosix(root, candidate));
          }
        });
      }
    });
    if (found.size > 0) return [...found].sort();
  }
  const indexHtml = resolve(root, 'ui/index.html');
  return existsSync(indexHtml) ? ['ui/index.html'] : [];
}

function readRatchetFile(root, { allowMissing = false } = {}) {
  const ratchetPath = resolve(root, 'tools/legacy-widgets-ratchet.json');
  if (!existsSync(ratchetPath)) {
    if (allowMissing) {
      return { missing: true, maxUnusedExports: 0 };
    }
    throw new Error('Missing tools/legacy-widgets-ratchet.json');
  }
  const data = JSON.parse(readFileSync(ratchetPath, 'utf8'));
  if (!Number.isInteger(data.maxUnusedExports) || data.maxUnusedExports < 0) {
    throw new Error('tools/legacy-widgets-ratchet.json must set integer maxUnusedExports >= 0');
  }
  return { ...data, missing: false };
}

export function checkUiAndExports(root, ratchetData = null) {
  const data = ratchetData ?? readRatchetFile(root);
  const maxUnusedExports = data.maxUnusedExports;

  const allFiles = walkDirectory(
    resolve(root, 'ui/src'),
    root,
    rel => UI_CHECKED_EXTENSIONS.has(extname(rel).toLowerCase()),
  ).sort((a, b) => a.rel.localeCompare(b.rel));

  const roleByRel = new Map();
  for (const file of allFiles) roleByRel.set(file.rel, classifyUiFile(file.rel));

  const entryPoints = discoverHtmlEntryPoints(root);
  const {
    liveProd,
    runtimeLive,
    typeOnlyLive,
    productionTestImports,
    unusedExports,
  } = analyzeUiSymbolGraph(root, allFiles, roleByRel, entryPoints);

  const deadProduction = [];
  for (const file of allFiles) {
    if (roleByRel.get(file.rel) === 'production' && !liveProd.has(file.rel)) {
      deadProduction.push(file.rel);
    }
  }
  const deadProdSet = new Set(deadProduction);

  const testCache = new Map();
  const testFiles = allFiles.filter(file => roleByRel.get(file.rel) === 'test').map(file => file.rel);
  const deadTests = [];
  const liveTestReach = new Set();

  for (const testRel of testFiles) {
    const graph = walkImportGraph([testRel], root, {
      includeTypes: true,
      includeDynamic: true,
      includeWorkerUrls: true,
      includeTestStringRefs: true,
      cache: testCache,
      shouldTraverse: file => {
        const rel = relPosix(root, file);
        return !liveProd.has(rel);
      },
    });
    const deadTargets = [];
    for (const file of graph.visited) {
      const rel = relPosix(root, file);
      if (deadProdSet.has(rel)) deadTargets.push(rel);
    }
    if (deadTargets.length > 0) {
      deadTests.push({ test: testRel, targets: deadTargets.sort() });
    } else {
      for (const file of graph.visited) {
        const rel = relPosix(root, file);
        if (roleByRel.has(rel)) liveTestReach.add(rel);
      }
    }
  }

  const deadFixturesAndSupport = [];
  for (const file of allFiles) {
    const role = roleByRel.get(file.rel);
    if ((role === 'fixture' || role === 'test-support') && !liveTestReach.has(file.rel)) {
      deadFixturesAndSupport.push({ file: file.rel, role });
    }
  }

  const exportErrors = [];
  const exportNotices = [];
  if (unusedExports.length > maxUnusedExports) {
    exportErrors.push(`${unusedExports.length} unused export(s) in ui/src, above ratchet ${maxUnusedExports}`);
  } else if (unusedExports.length > 0 && unusedExports.length < maxUnusedExports) {
    exportNotices.push(`unused exports in ui/src dropped to ${unusedExports.length} (ratchet is ${maxUnusedExports}); lower maxUnusedExports in tools/legacy-widgets-ratchet.json to ${unusedExports.length}`);
  } else if (unusedExports.length === 0 && maxUnusedExports !== 0) {
    exportErrors.push(`no unused exports remain in ui/src; lower maxUnusedExports in tools/legacy-widgets-ratchet.json from ${maxUnusedExports} to 0`);
  }

  return {
    ui: {
      entryPoints,
      liveProd,
      runtimeLive,
      typeOnlyLive,
      deadProduction,
      deadTests,
      deadFixturesAndSupport,
      productionTestImports,
    },
    exports: {
      maxUnusedExports,
      unusedExports,
      errors: exportErrors,
      notices: exportNotices,
    },
  };
}

function parseContractProducts(root) {
  const contractPath = resolve(root, 'ui/src/generated/dashboard.ts');
  if (!existsSync(contractPath)) return [];
  const text = readFileSync(contractPath, 'utf8');
  const start = text.indexOf('export const REQUEST_PATHS');
  if (start < 0) return [];
  const open = text.indexOf('{', start);
  const end = text.indexOf('\n};', open);
  if (open < 0 || end < 0) return [];
  const block = text.slice(open + 1, end);
  const keys = [];
  for (const match of block.matchAll(/^\s*"([^"]+)":\s*\{/gm)) {
    keys.push(match[1]);
  }
  return keys.sort();
}

function findUnwiredRegistrations(root) {
  const modPath = resolve(root, 'crates/dashboard/src/products/mod.rs');
  if (!existsSync(modPath)) return [];
  const modText = readFileSync(modPath, 'utf8');
  const cratesDir = resolve(root, 'crates');
  if (!existsSync(cratesDir)) return [];

  const unwired = [];
  for (const entry of readdirSync(cratesDir, { withFileTypes: true })) {
    if (!entry.isDirectory() || !entry.name.startsWith('dashboard-')) continue;
    const regPath = resolve(cratesDir, entry.name, 'src/registrations.rs');
    if (!existsSync(regPath)) continue;
    const crateIdent = entry.name.replace(/-/g, '_');
    const regText = readFileSync(regPath, 'utf8');
    const macroPattern = /register(?:_hourly)?_products!\s*\{([\s\S]*?)\}/g;
    for (const block of regText.matchAll(macroPattern)) {
      for (const item of block[1].matchAll(/\b([A-Z][A-Z0-9_]+)\s*=>/g)) {
        const constName = item[1];
        const qualified = `${crateIdent}::registrations::${constName}`;
        if (!modText.includes(qualified)) {
          unwired.push(`${relPosix(root, regPath)}: ${constName}`);
        }
      }
    }
  }
  return unwired.sort();
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function textProductMatcher(key) {
  const escaped = escapeRegExp(key);
  if (key.startsWith('head:')) {
    const bare = key.slice('head:'.length);
    const bareEscaped = escapeRegExp(bare);
    const pattern = new RegExp(`['"\`]${escaped}['"\`]|\\bhead\\s*:\\s*['"\`]${bareEscaped}['"\`]`);
    return text => text.includes(bare) && pattern.test(text);
  }
  const pattern = new RegExp(`['"\`]${escaped}['"\`]`);
  return text => text.includes(key) && pattern.test(text);
}

const QUOTED_PRODUCT_TOKEN = /['"`]([a-z][a-z0-9:-]{1,63})['"`]/g;

function sourceMightMentionProduct(source, productSet, headNames) {
  QUOTED_PRODUCT_TOKEN.lastIndex = 0;
  let match;
  while ((match = QUOTED_PRODUCT_TOKEN.exec(source)) !== null) {
    const token = match[1];
    if (productSet.has(token) || headNames.has(token)) return true;
  }
  return false;
}

function collectUiProductCalls(filePath, relPath, productSet, headNames) {
  const found = new Set();
  if (NON_CALLER_UI_FILES.has(relPath) || relPath.startsWith('ui/src/widgets/manifest/')) return found;
  const ext = extname(filePath).toLowerCase();
  if (!SCRIPT_EXTENSIONS.has(ext)) return found;
  const source = readFileSync(filePath, 'utf8');
  if (!sourceMightMentionProduct(source, productSet, headNames)) return found;
  const ast = parseAst(source, { lang: astLang(filePath) });

  walkAst(ast, (node, parent, key) => {
    const val = staticStringLiteral(node);
    if (val === null) return;
    const propName = parent?.type === 'Property' && key === 'value'
      ? (parent.key?.name ?? parent.key?.value ?? null)
      : null;
    const isObjKey = parent?.type === 'Property' && key === 'key';
    if (propName === 'head' && headNames.has(val)) {
      found.add(`head:${val}`);
    } else if (!isObjKey && !NON_PRODUCT_PROPERTY_NAMES.has(propName) && productSet.has(val)) {
      found.add(val);
    }
  });
  return found;
}

export function checkRustProducts(root, uiResult) {
  const contractProducts = parseContractProducts(root);
  const productSet = new Set(contractProducts);
  const headNames = new Set(contractProducts.filter(key => key.startsWith('head:')).map(key => key.slice('head:'.length)));
  const unwiredRegistrations = findUnwiredRegistrations(root);

  const liveUiCalls = new Set();
  const deadOrTestUiFiles = [];
  const uiFiles = walkDirectory(
    resolve(root, 'ui/src'),
    root,
    rel => !rel.startsWith('ui/src/generated/') && TEXT_EXTENSIONS.has(extname(rel).toLowerCase()),
  );
  for (const file of uiFiles) {
    const role = classifyUiFile(file.rel);
    if (role === 'production' && uiResult.runtimeLive.has(file.rel)) {
      for (const hit of collectUiProductCalls(file.full, file.rel, productSet, headNames)) {
        liveUiCalls.add(hit);
      }
    } else {
      deadOrTestUiFiles.push(file);
    }
  }

  const toolCallerFiles = walkDirectory(
    resolve(root, 'tools'),
    root,
    rel => !rel.startsWith('tools/check-dead-code') && TOOL_CALLER_EXTENSIONS.has(extname(rel).toLowerCase()),
  );

  const nonDashboardCrateFiles = walkDirectory(
    resolve(root, 'crates'),
    root,
    isNonDashboardRustCaller,
  );

  const nonUiLiveBlob = [...toolCallerFiles, ...nonDashboardCrateFiles]
    .map(file => readFileSync(file.full, 'utf8'))
    .join('\n');
  const deadOrTestCorpus = deadOrTestUiFiles
    .map(file => ({ rel: file.rel, text: readFileSync(file.full, 'utf8') }));

  const deadProducts = [];
  for (const product of contractProducts) {
    if (liveUiCalls.has(product)) continue;
    const matches = textProductMatcher(product);
    if (matches(nonUiLiveBlob)) continue;
    const onlyIn = deadOrTestCorpus
      .filter(file => matches(file.text))
      .map(file => file.rel)
      .sort();
    deadProducts.push({ product, onlyIn });
  }

  return {
    totalContractProducts: contractProducts.length,
    deadProducts,
    unwiredRegistrations,
  };
}

export async function runDeadCodeCheck(root, options = {}) {
  const ratchetData = readRatchetFile(root, { allowMissing: Boolean(options.allowMissingRatchet) });
  const { ui, exports } = checkUiAndExports(root, ratchetData);
  const rust = checkRustProducts(root, ui);
  const totalIssues = ui.deadProduction.length
    + ui.deadTests.length
    + ui.deadFixturesAndSupport.length
    + ui.productionTestImports.length
    + rust.deadProducts.length
    + rust.unwiredRegistrations.length
    + exports.errors.length;
  return { ui, rust, exports, totalIssues, ratchetMissing: Boolean(ratchetData.missing) };
}

export function classifyAgainstBase(result, baseResult, baseSha = 'base') {
  const shortBase = baseSha.slice(0, 9);
  const baseRatchetMissing = Boolean(baseResult.ratchetMissing);
  const baseDeadProd = new Set(baseResult.ui.deadProduction);
  const baseDeadTestTargets = new Map(baseResult.ui.deadTests.map(entry => [entry.test, new Set(entry.targets)]));
  const baseDeadFixtures = new Set(baseResult.ui.deadFixturesAndSupport.map(entry => entry.file));
  const baseProdTestImports = new Set((baseResult.ui.productionTestImports ?? []).map(e => `${e.from}->${e.to}`));
  const baseDeadProducts = new Set(baseResult.rust.deadProducts.map(entry => entry.product));
  const baseUnwired = new Set(baseResult.rust.unwiredRegistrations);
  const baseUnusedExportKeys = baseRatchetMissing ? new Set() : new Set((baseResult.exports?.unusedExports ?? []).map(item => item.key));
  const baseExportErrors = baseRatchetMissing ? new Set() : new Set(baseResult.exports?.errors ?? []);
  const baseExportsOver = !baseRatchetMissing && (baseResult.exports?.unusedExports?.length ?? 0) > baseResult.exports.maxUnusedExports;

  const addedUnusedExports = result.exports.unusedExports.filter(item => !baseUnusedExportKeys.has(item.key));

  const ratchetRaises = [];
  if (!baseRatchetMissing && result.exports.maxUnusedExports > baseResult.exports.maxUnusedExports) {
    ratchetRaises.push(`maxUnusedExports raised from ${baseResult.exports.maxUnusedExports} to ${result.exports.maxUnusedExports}`);
  }

  let newIssues = 0;
  let inheritedIssues = 0;
  const tag = inherited => {
    if (inherited) inheritedIssues += 1;
    else newIssues += 1;
    return inherited ? `[inherited from ${shortBase}]` : '[new on this branch]';
  };

  const deadProduction = result.ui.deadProduction.map(file => ({
    file,
    inherited: baseDeadProd.has(file),
    tag: tag(baseDeadProd.has(file)),
  }));

  const deadTests = result.ui.deadTests.map(entry => {
    const baseTargets = baseDeadTestTargets.get(entry.test);
    const inherited = Boolean(baseTargets && entry.targets.every(target => baseTargets.has(target)));
    return { ...entry, inherited, tag: tag(inherited) };
  });

  const deadFixturesAndSupport = result.ui.deadFixturesAndSupport.map(entry => ({
    ...entry,
    inherited: baseDeadFixtures.has(entry.file),
    tag: tag(baseDeadFixtures.has(entry.file)),
  }));

  const productionTestImports = (result.ui.productionTestImports ?? []).map(entry => {
    const inherited = baseProdTestImports.has(`${entry.from}->${entry.to}`);
    return { ...entry, inherited, tag: tag(inherited) };
  });

  const deadProducts = result.rust.deadProducts.map(entry => ({
    ...entry,
    inherited: baseDeadProducts.has(entry.product),
    tag: tag(baseDeadProducts.has(entry.product)),
  }));

  const unwiredRegistrations = result.rust.unwiredRegistrations.map(item => ({
    item,
    inherited: baseUnwired.has(item),
    tag: tag(baseUnwired.has(item)),
  }));

  const ratchetErrors = ratchetRaises.map(err => ({
    error: err,
    inherited: false,
    tag: tag(false),
  }));

  let exportsAddedReported = false;
  const unusedExportErrors = result.exports.errors.map(err => {
    if (err.includes('above ratchet')) {
      exportsAddedReported = true;
      const addedText = addedUnusedExports.length > 0
        ? ` (added: ${addedUnusedExports.slice(0, 10).map(item => item.key).join(', ')}${addedUnusedExports.length > 10 ? `, +${addedUnusedExports.length - 10} more` : ''})`
        : '';
      const inherited = baseExportsOver
        && addedUnusedExports.length === 0
        && result.exports.unusedExports.length <= (baseResult.exports?.unusedExports?.length ?? 0);
      return { error: `${err}${addedText}`, inherited, tag: tag(inherited) };
    }
    const inherited = baseExportErrors.has(err);
    return { error: err, inherited, tag: tag(inherited) };
  });
  if (!exportsAddedReported && addedUnusedExports.length > 0) {
    unusedExportErrors.push({
      error: `added ${addedUnusedExports.length} unused export(s) vs ${shortBase}: ${addedUnusedExports.slice(0, 10).map(item => item.key).join(', ')}${addedUnusedExports.length > 10 ? `, +${addedUnusedExports.length - 10} more` : ''}`,
      inherited: false,
      tag: tag(false),
    });
  }

  const unusedExports = result.exports.unusedExports.map(item => ({
    ...item,
    inherited: baseUnusedExportKeys.has(item.key),
    tag: baseUnusedExportKeys.has(item.key) ? `[inherited from ${shortBase}]` : '[new on this branch]',
  }));

  return {
    shortBase,
    addedUnusedExports,
    ratchetRaises,
    newIssues,
    inheritedIssues,
    deadProduction,
    deadTests,
    deadFixturesAndSupport,
    productionTestImports,
    deadProducts,
    unwiredRegistrations,
    ratchetErrors,
    unusedExportErrors,
    unusedExports,
  };
}

export function formatReport(result, comparison = null, options = {}) {
  const { showAllInFile = false } = options;
  const lines = [];
  if (result.ui.deadProduction.length > 0) {
    lines.push(`Dead UI production modules (${result.ui.deadProduction.length}):`);
    if (comparison) {
      for (const entry of comparison.deadProduction) lines.push(`  - ${entry.file} ${entry.tag}`);
    } else {
      for (const file of result.ui.deadProduction) lines.push(`  - ${file}`);
    }
  }
  if (result.ui.deadTests.length > 0) {
    lines.push(`Dead UI tests targeting dead modules (${result.ui.deadTests.length}):`);
    const list = comparison ? comparison.deadTests : result.ui.deadTests;
    for (const entry of list) {
      const suffix = entry.tag ? ` ${entry.tag}` : '';
      lines.push(`  - ${entry.test} -> ${entry.targets.join(', ')}${suffix}`);
    }
  }
  if (result.ui.deadFixturesAndSupport.length > 0) {
    lines.push(`Dead UI fixtures / test-support modules (${result.ui.deadFixturesAndSupport.length}):`);
    const list = comparison ? comparison.deadFixturesAndSupport : result.ui.deadFixturesAndSupport;
    for (const entry of list) {
      const suffix = entry.tag ? ` ${entry.tag}` : '';
      lines.push(`  - ${entry.file} (${entry.role})${suffix}`);
    }
  }
  if ((result.ui.productionTestImports?.length ?? 0) > 0) {
    lines.push(`Production UI modules importing test/fixture/support files (${result.ui.productionTestImports.length}):`);
    const list = comparison ? comparison.productionTestImports : result.ui.productionTestImports;
    for (const entry of list) {
      const suffix = entry.tag ? ` ${entry.tag}` : '';
      lines.push(`  - ${entry.from} -> ${entry.to} (${entry.role})${suffix}`);
    }
  }
  if (result.rust.deadProducts.length > 0) {
    lines.push(`Dead Rust dashboard products (${result.rust.deadProducts.length}):`);
    const list = comparison ? comparison.deadProducts : result.rust.deadProducts;
    for (const entry of list) {
      const onlyIn = entry.onlyIn.length > 0 ? ` (only referenced by dead/test UI: ${entry.onlyIn.join(', ')})` : '';
      const suffix = entry.tag ? ` ${entry.tag}` : '';
      lines.push(`  - ${entry.product}${onlyIn}${suffix}`);
    }
  }
  if (result.rust.unwiredRegistrations.length > 0) {
    lines.push(`Unwired Rust dashboard registrations (${result.rust.unwiredRegistrations.length}):`);
    if (comparison) {
      for (const entry of comparison.unwiredRegistrations) lines.push(`  - ${entry.item} ${entry.tag}`);
    } else {
      for (const item of result.rust.unwiredRegistrations) lines.push(`  - ${item}`);
    }
  }
  if (comparison && comparison.ratchetErrors.length > 0) {
    lines.push(`Ratchet ceiling regressions (${comparison.ratchetErrors.length}):`);
    for (const entry of comparison.ratchetErrors) lines.push(`  - ${entry.error} ${entry.tag}`);
  }
  const expErrors = comparison ? comparison.unusedExportErrors : result.exports.errors.map(error => ({ error, tag: '' }));
  if (expErrors.length > 0 || (showAllInFile && result.exports.unusedExports.length > 0)) {
    lines.push(`Unused exports in ui/src (${result.exports.unusedExports.length}/${result.exports.maxUnusedExports}):`);
    for (const entry of expErrors) lines.push(`  - ${entry.error}${entry.tag ? ` ${entry.tag}` : ''}`);
    const list = comparison ? comparison.unusedExports : result.exports.unusedExports;
    for (const entry of list) {
      if (!showAllInFile && comparison && entry.inherited && result.exports.errors.length === 0) continue;
      const kind = entry.isReexport ? ' [re-export]' : entry.isType ? ' [type]' : '';
      const suffix = entry.tag ? ` ${entry.tag}` : '';
      lines.push(`  - ${entry.file}: ${entry.name}${kind}${suffix}`);
    }
  }
  for (const notice of result.exports.notices) {
    lines.push(`dead-code notice: ${notice}`);
  }
  return lines.join('\n');
}

function gitText(root, args) {
  try {
    return execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  } catch {
    return '';
  }
}

export function resolveBaseCommit(root, explicitRef) {
  if (explicitRef !== null && explicitRef !== undefined) {
    const baseSha = gitText(root, ['rev-parse', '--verify', '--quiet', `${explicitRef}^{commit}`]);
    if (!baseSha) {
      throw new Error(`Cannot resolve --against ref "${explicitRef}" to a commit`);
    }
    const headSha = gitText(root, ['rev-parse', '--verify', '--quiet', 'HEAD']);
    const dirty = gitText(root, ['status', '--porcelain']);
    if (baseSha === headSha && !dirty) return null;
    return baseSha;
  }
  const baseSha = gitText(root, ['merge-base', 'HEAD', 'origin/main']) || gitText(root, ['merge-base', 'HEAD', 'main']);
  if (!baseSha) return null;
  const headSha = gitText(root, ['rev-parse', '--verify', '--quiet', 'HEAD']);
  const dirty = gitText(root, ['status', '--porcelain']);
  if (baseSha === headSha && !dirty) return null;
  return baseSha;
}

export function materializeCommitSnapshot(root, sha, destDir) {
  const treeOut = execFileSync(
    'git',
    ['ls-tree', '-r', '-z', sha],
    { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] },
  );
  const entries = [];
  for (const record of treeOut.split('\0')) {
    if (!record) continue;
    const tab = record.indexOf('\t');
    if (tab < 0) continue;
    const meta = record.slice(0, tab).split(' ');
    if (meta[1] !== 'blob') continue;
    const rel = record.slice(tab + 1);
    if (BASE_SNAPSHOT_RULES.some(rule => rule.match(rel))) {
      entries.push({ oid: meta[2], rel });
    }
  }
  if (entries.length === 0) return;
  const input = `${entries.map(e => e.oid).join('\n')}\n`;
  const raw = execFileSync(
    'git',
    ['cat-file', '--batch'],
    { cwd: root, input, maxBuffer: 256 * 1024 * 1024, stdio: ['pipe', 'pipe', 'pipe'] },
  );
  const createdDirs = new Set();
  let offset = 0;
  for (const entry of entries) {
    const nl = raw.indexOf(0x0a, offset);
    if (nl < 0) throw new Error(`Unexpected end of git cat-file output at ${entry.rel}`);
    const header = raw.subarray(offset, nl).toString('utf8').split(' ');
    if (header[1] !== 'blob') {
      throw new Error(`Expected blob from git cat-file for ${entry.rel}, got ${header.join(' ')}`);
    }
    const size = Number(header[2]);
    const bodyStart = nl + 1;
    const bodyEnd = bodyStart + size;
    const target = join(destDir, entry.rel);
    const dir = dirname(target);
    if (!createdDirs.has(dir)) {
      mkdirSync(dir, { recursive: true });
      createdDirs.add(dir);
    }
    writeFileSync(target, raw.subarray(bodyStart, bodyEnd));
    offset = bodyEnd + 1;
  }
}

export async function runCheckAtCommit(root, sha) {
  const baseDir = mkdtempSync(join(tmpdir(), 'bp-dead-code-base-'));
  const nodeModulesLink = join(baseDir, 'node_modules');
  const repoNodeModules = resolve(root, 'node_modules');
  let linkedNodeModules = false;
  try {
    materializeCommitSnapshot(root, sha, baseDir);
    if (existsSync(repoNodeModules)) {
      symlinkSync(repoNodeModules, nodeModulesLink, 'junction');
      linkedNodeModules = true;
    }
    return await runDeadCodeCheck(baseDir, { allowMissingRatchet: true });
  } finally {
    if (linkedNodeModules) {
      try { unlinkSync(nodeModulesLink); } catch {}
    }
    rmSync(baseDir, { recursive: true, force: true });
  }
}

export function parseAgainstArg(argv) {
  const index = argv.indexOf('--against');
  if (index >= 0) {
    const value = argv[index + 1];
    if (!value || value.startsWith('--')) {
      throw new Error('--against requires a commit or ref');
    }
    return value;
  }
  const prefixed = argv.find(arg => arg.startsWith('--against='));
  if (prefixed !== undefined) {
    const value = prefixed.slice('--against='.length);
    if (!value) throw new Error('--against requires a commit or ref');
    return value;
  }
  return null;
}

export function compareWithBase(result, baseResult, baseSha = 'base') {
  if (baseResult.ratchetMissing) {
    return {
      comparison: null,
      notice: `base ${baseSha.slice(0, 9)} has no tools/legacy-widgets-ratchet.json; checking HEAD without base comparison`,
    };
  }
  return {
    comparison: classifyAgainstBase(result, baseResult, baseSha),
    notice: null,
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const reportOnly = process.argv.includes('--report-only');
  const noBase = process.argv.includes('--no-base');
  const listInFile = process.argv.includes('--list-in-file');
  const againstArg = parseAgainstArg(process.argv);
  const root = fileURLToPath(new URL('../', import.meta.url));
  const result = await runDeadCodeCheck(root);
  let comparison = null;
  let baseNotice = null;
  if (!noBase) {
    const baseSha = resolveBaseCommit(root, againstArg);
    if (baseSha) {
      const baseResult = await runCheckAtCommit(root, baseSha);
      ({ comparison, notice: baseNotice } = compareWithBase(result, baseResult, baseSha));
    }
  }
  const body = formatReport(result, comparison, { showAllInFile: listInFile });
  if (body) console.log(body);
  if (baseNotice) console.log(`dead-code notice: ${baseNotice}`);
  const summary = `dead-code: ${result.ui.deadProduction.length} dead UI module(s), `
    + `${result.ui.deadTests.length} dead test(s), `
    + `${result.ui.deadFixturesAndSupport.length} dead fixture/support file(s), `
    + `${result.rust.deadProducts.length} dead Rust product(s), `
    + `${result.rust.unwiredRegistrations.length} unwired registration(s), `
    + `${result.exports.unusedExports.length}/${result.exports.maxUnusedExports} unused export(s)`;
  const failed = comparison ? comparison.newIssues > 0 : result.totalIssues > 0;
  if (failed) {
    if (reportOnly) {
      console.log(`${summary} (report-only)`);
      process.exit(0);
    }
    const detail = comparison
      ? `FAILED: ${comparison.newIssues} new on this branch, ${comparison.inheritedIssues} inherited from ${comparison.shortBase}`
      : 'FAILED';
    console.error(`${summary} (${detail})`);
    process.exit(1);
  }
  if (comparison && comparison.inheritedIssues > 0) {
    console.log(`${summary} (OK: 0 new on this branch; ${comparison.inheritedIssues} inherited from ${comparison.shortBase})`);
    process.exit(0);
  }
  console.log(`${summary} (OK)`);
}
