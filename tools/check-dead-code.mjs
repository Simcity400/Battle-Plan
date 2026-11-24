import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { extname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseAst } from 'rolldown/parseAst';
import { relPosix, walkImportGraph } from './ui-import-graph.mjs';

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
const TEXT_EXTENSIONS = new Set(['.ts', '.tsx', '.js', '.mjs', '.cjs', '.rs', '.py', '.ps1', '.json', '.toml', '.html', '.css']);
const NON_CALLER_UI_FILES = new Set([
  'ui/src/dashboard/pages.ts',
  'ui/src/widgets/manifest.ts',
  'ui/src/widgets/legacyManifest.ts',
  'ui/src/widgets/familyManifest.ts',
]);
const NON_PRODUCT_PROPERTY_NAMES = new Set(['kind', 'view', 'owner', 'className', 'icon', 'category', 'family']);

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

export function discoverHtmlEntryPoints(root) {
  const uiDir = resolve(root, 'ui');
  if (!existsSync(uiDir)) return [];
  return readdirSync(uiDir, { withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith('.html'))
    .map(entry => `ui/${entry.name}`)
    .sort();
}

export function checkUiModules(root) {
  const allFiles = walkDirectory(
    resolve(root, 'ui/src'),
    root,
    rel => UI_CHECKED_EXTENSIONS.has(extname(rel).toLowerCase()),
  ).sort((a, b) => a.rel.localeCompare(b.rel));

  const roleByRel = new Map();
  for (const file of allFiles) roleByRel.set(file.rel, classifyUiFile(file.rel));

  const cache = new Map();
  const entryPoints = discoverHtmlEntryPoints(root);
  const prodWalk = walkImportGraph(entryPoints, root, {
    includeTypes: true,
    includeDynamic: true,
    includeWorkerUrls: true,
    cache,
  });

  const liveProd = new Set();
  for (const file of prodWalk.visited) {
    const rel = relPosix(root, file);
    if (roleByRel.has(rel)) liveProd.add(rel);
  }

  const deadProduction = [];
  for (const file of allFiles) {
    if (roleByRel.get(file.rel) === 'production' && !liveProd.has(file.rel)) {
      deadProduction.push(file.rel);
    }
  }
  const deadProdSet = new Set(deadProduction);

  const testFiles = allFiles.filter(file => roleByRel.get(file.rel) === 'test').map(file => file.rel);
  const deadTests = [];
  const liveTestReach = new Set();

  for (const testRel of testFiles) {
    const graph = walkImportGraph([testRel], root, {
      includeTypes: true,
      includeDynamic: true,
      includeWorkerUrls: true,
      includeTestStringRefs: true,
      cache,
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
    if ((role === 'fixture' || role === 'test-support') && !liveTestReach.has(file.rel) && !liveProd.has(file.rel)) {
      deadFixturesAndSupport.push({ file: file.rel, role });
    }
  }

  return {
    entryPoints,
    liveProd,
    deadProduction,
    deadTests,
    deadFixturesAndSupport,
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

const QUOTED_PRODUCT_TOKEN = /['"]([a-z][a-z0-9:-]{1,63})['"]/g;

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
  if (ext !== '.ts' && ext !== '.tsx' && ext !== '.js' && ext !== '.mjs') return found;
  const source = readFileSync(filePath, 'utf8');
  if (!sourceMightMentionProduct(source, productSet, headNames)) return found;
  const lang = ext === '.tsx' ? 'tsx' : ext === '.ts' ? 'ts' : 'js';
  const ast = parseAst(source, { lang });

  const stack = [{ node: ast, parent: null, key: null }];
  while (stack.length > 0) {
    const { node, parent, key } = stack.pop();
    if (!node || typeof node !== 'object') continue;
    if (node.type === 'Literal' && typeof node.value === 'string') {
      const val = node.value;
      const propName = parent?.type === 'Property' && key === 'value'
        ? (parent.key?.name ?? parent.key?.value ?? null)
        : null;
      const isObjKey = parent?.type === 'Property' && key === 'key';
      if (propName === 'head' && headNames.has(val)) {
        found.add(`head:${val}`);
      } else if (!isObjKey && !NON_PRODUCT_PROPERTY_NAMES.has(propName) && productSet.has(val)) {
        found.add(val);
      }
    }
    for (const [childKey, childVal] of Object.entries(node)) {
      if (Array.isArray(childVal)) {
        for (let i = childVal.length - 1; i >= 0; i -= 1) {
          const item = childVal[i];
          if (item && typeof item === 'object' && typeof item.type === 'string') {
            stack.push({ node: item, parent: node, key: childKey });
          }
        }
      } else if (childVal && typeof childVal === 'object' && typeof childVal.type === 'string') {
        stack.push({ node: childVal, parent: node, key: childKey });
      }
    }
  }
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
    if (role === 'production' && uiResult.liveProd.has(file.rel)) {
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
    rel => !rel.startsWith('tools/check-dead-code') && TEXT_EXTENSIONS.has(extname(rel).toLowerCase()),
  );

  const nonDashboardCrateFiles = walkDirectory(
    resolve(root, 'crates'),
    root,
    rel => {
      const crateName = rel.split('/')[1] ?? '';
      return !crateName.startsWith('dashboard') && extname(rel).toLowerCase() === '.rs';
    },
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

function countLegacyManifestEntries(legacyPath) {
  const source = readFileSync(legacyPath, 'utf8');
  const ast = parseAst(source, { lang: 'ts' });
  for (const node of ast.body) {
    if (node.type !== 'ExportNamedDeclaration' || node.declaration?.type !== 'VariableDeclaration') continue;
    for (const decl of node.declaration.declarations) {
      if (decl.id?.type === 'Identifier' && decl.id.name === 'LEGACY_WIDGETS' && decl.init?.type === 'ArrayExpression') {
        return decl.init.elements.length;
      }
    }
  }
  throw new Error('Could not find exported LEGACY_WIDGETS array in ui/src/widgets/legacyManifest.ts');
}

export function checkLegacyRatchet(root) {
  const ratchetPath = resolve(root, 'tools/legacy-widgets-ratchet.json');
  const ratchetData = JSON.parse(readFileSync(ratchetPath, 'utf8'));
  const maxLegacyWidgets = ratchetData.maxLegacyWidgets;
  if (typeof maxLegacyWidgets !== 'number' || !Number.isInteger(maxLegacyWidgets) || maxLegacyWidgets < 0) {
    throw new Error('tools/legacy-widgets-ratchet.json must set integer maxLegacyWidgets >= 0');
  }

  const legacyPath = resolve(root, 'ui/src/widgets/legacyManifest.ts');
  const legacyFileExists = existsSync(legacyPath);
  const count = legacyFileExists ? countLegacyManifestEntries(legacyPath) : 0;

  const errors = [];
  const notices = [];

  if (count > maxLegacyWidgets) {
    errors.push(`ui/src/widgets/legacyManifest.ts has ${count} entries, above ratchet ${maxLegacyWidgets}`);
  } else if (count > 0 && count < maxLegacyWidgets) {
    notices.push(`ui/src/widgets/legacyManifest.ts now has ${count} entries (ratchet is ${maxLegacyWidgets}); lower maxLegacyWidgets in tools/legacy-widgets-ratchet.json to ${count}`);
  }

  if (count === 0) {
    if (maxLegacyWidgets !== 0) {
      errors.push(`legacy widgets are gone; lower maxLegacyWidgets in tools/legacy-widgets-ratchet.json from ${maxLegacyWidgets} to 0`);
    }
    if (legacyFileExists) {
      errors.push('ui/src/widgets/legacyManifest.ts is empty and must be deleted');
    }
    const familyManifestPath = resolve(root, 'ui/src/families/manifest.ts');
    if (existsSync(familyManifestPath) && /\bLegacyWidgetEntry\b/.test(readFileSync(familyManifestPath, 'utf8'))) {
      errors.push('LegacyWidgetEntry still exists in ui/src/families/manifest.ts after legacy list reached 0');
    }
    const manifestPath = resolve(root, 'ui/src/widgets/manifest.ts');
    if (existsSync(manifestPath)) {
      const text = readFileSync(manifestPath, 'utf8');
      for (const token of ['LEGACY_WIDGETS', 'legacyEntriesForKind', 'manifestLegacyEntry']) {
        if (new RegExp(`\\b${token}\\b`).test(text)) {
          errors.push(`${token} still exists in ui/src/widgets/manifest.ts after legacy list reached 0`);
        }
      }
    }
    const loaderPath = resolve(root, 'ui/src/widgets/rendererLoaders.ts');
    if (existsSync(loaderPath)) {
      const text = readFileSync(loaderPath, 'utf8');
      for (const token of ['LEGACY_WIDGETS', 'LegacyWidgetEntry']) {
        if (new RegExp(`\\b${token}\\b`).test(text)) {
          errors.push(`${token} still exists in ui/src/widgets/rendererLoaders.ts after legacy list reached 0`);
        }
      }
    }
  }

  return { count, maxLegacyWidgets, errors, notices };
}

export function runDeadCodeCheck(root) {
  const ui = checkUiModules(root);
  const rust = checkRustProducts(root, ui);
  const legacy = checkLegacyRatchet(root);
  const totalIssues = ui.deadProduction.length
    + ui.deadTests.length
    + ui.deadFixturesAndSupport.length
    + rust.deadProducts.length
    + rust.unwiredRegistrations.length
    + legacy.errors.length;
  return { ui, rust, legacy, totalIssues };
}

export function formatReport(result) {
  const lines = [];
  if (result.ui.deadProduction.length > 0) {
    lines.push(`Dead UI production modules (${result.ui.deadProduction.length}):`);
    for (const file of result.ui.deadProduction) lines.push(`  - ${file}`);
  }
  if (result.ui.deadTests.length > 0) {
    lines.push(`Dead UI tests targeting dead modules (${result.ui.deadTests.length}):`);
    for (const entry of result.ui.deadTests) {
      lines.push(`  - ${entry.test} -> ${entry.targets.join(', ')}`);
    }
  }
  if (result.ui.deadFixturesAndSupport.length > 0) {
    lines.push(`Dead UI fixtures / test-support modules (${result.ui.deadFixturesAndSupport.length}):`);
    for (const entry of result.ui.deadFixturesAndSupport) {
      lines.push(`  - ${entry.file} (${entry.role})`);
    }
  }
  if (result.rust.deadProducts.length > 0) {
    lines.push(`Dead Rust dashboard products (${result.rust.deadProducts.length}):`);
    for (const entry of result.rust.deadProducts) {
      const suffix = entry.onlyIn.length > 0 ? ` (only referenced by dead/test UI: ${entry.onlyIn.join(', ')})` : '';
      lines.push(`  - ${entry.product}${suffix}`);
    }
  }
  if (result.rust.unwiredRegistrations.length > 0) {
    lines.push(`Unwired Rust dashboard registrations (${result.rust.unwiredRegistrations.length}):`);
    for (const item of result.rust.unwiredRegistrations) lines.push(`  - ${item}`);
  }
  if (result.legacy.errors.length > 0) {
    lines.push(`Legacy widget ratchet errors (${result.legacy.errors.length}):`);
    for (const err of result.legacy.errors) lines.push(`  - ${err}`);
  }
  for (const notice of result.legacy.notices) {
    lines.push(`dead-code notice: ${notice}`);
  }
  return lines.join('\n');
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const reportOnly = process.argv.includes('--report-only');
  const root = fileURLToPath(new URL('../', import.meta.url));
  const result = runDeadCodeCheck(root);
  const body = formatReport(result);
  if (body) console.log(body);
  const summary = `dead-code: ${result.ui.deadProduction.length} dead UI module(s), `
    + `${result.ui.deadTests.length} dead test(s), `
    + `${result.ui.deadFixturesAndSupport.length} dead fixture/support file(s), `
    + `${result.rust.deadProducts.length} dead Rust product(s), `
    + `${result.rust.unwiredRegistrations.length} unwired registration(s), `
    + `${result.legacy.count}/${result.legacy.maxLegacyWidgets} legacy widget(s)`;
  if (result.totalIssues > 0) {
    if (reportOnly) {
      console.log(`${summary} (report-only)`);
      process.exit(0);
    }
    console.error(`${summary} (FAILED)`);
    process.exit(1);
  }
  console.log(`${summary} (OK)`);
}
