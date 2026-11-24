import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, unlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join, resolve } from 'node:path';
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

function parseLegacyManifestEntries(legacyPath) {
  const source = readFileSync(legacyPath, 'utf8');
  const ast = parseAst(source, { lang: 'ts' });
  for (const node of ast.body) {
    if (node.type !== 'ExportNamedDeclaration' || node.declaration?.type !== 'VariableDeclaration') continue;
    for (const decl of node.declaration.declarations) {
      if (decl.id?.type === 'Identifier' && decl.id.name === 'LEGACY_WIDGETS' && decl.init?.type === 'ArrayExpression') {
        const entries = [];
        for (const el of decl.init.elements) {
          if (!el || el.type !== 'ObjectExpression') continue;
          let kind = null;
          let view = null;
          for (const prop of el.properties) {
            if (prop.type !== 'Property') continue;
            const propName = prop.key?.name ?? prop.key?.value;
            if (propName === 'kind' && prop.value?.type === 'Literal' && typeof prop.value.value === 'string') {
              kind = prop.value.value;
            } else if (propName === 'view' && prop.value?.type === 'Literal' && typeof prop.value.value === 'string') {
              view = prop.value.value;
            }
          }
          if (kind) entries.push({ kind, view });
        }
        return entries;
      }
    }
  }
  throw new Error('Could not find exported LEGACY_WIDGETS array in ui/src/widgets/legacyManifest.ts');
}

async function loadMetaAndFamilyViews(root) {
  const metaByKind = new Map();
  const familyByKind = new Map();
  const definitionsPath = resolve(root, 'ui/src/widgets/definitions.ts');
  if (existsSync(definitionsPath)) {
    const mod = await import(`${pathToFileURL(definitionsPath).href}?t=${Date.now()}`);
    for (const meta of mod.widgetMetas ?? []) {
      if (meta && typeof meta.kind === 'string') metaByKind.set(meta.kind, meta);
    }
  }
  const familyManifestPath = resolve(root, 'ui/src/widgets/familyManifest.ts');
  if (existsSync(familyManifestPath)) {
    const mod = await import(`${pathToFileURL(familyManifestPath).href}?t=${Date.now()}`);
    for (const entry of mod.FAMILY_VIEWS ?? []) {
      if (!entry || typeof entry.kind !== 'string') continue;
      const view = typeof entry.view === 'string' && entry.view.length > 0 ? entry.view : 'default';
      let set = familyByKind.get(entry.kind);
      if (!set) familyByKind.set(entry.kind, set = new Set());
      set.add(view);
    }
  }
  return { metaByKind, familyByKind };
}

function expandLegacyViewPairs(legacyEntries, metaByKind, familyByKind) {
  const pairs = [];
  const seen = new Set();
  const addPair = (kind, view) => {
    const pair = `${kind}:${view}`;
    if (!seen.has(pair)) {
      seen.add(pair);
      pairs.push(pair);
    }
  };
  for (const entry of legacyEntries) {
    if (entry.view && entry.view.length > 0) {
      addPair(entry.kind, entry.view);
      continue;
    }
    const meta = metaByKind.get(entry.kind);
    const optKey = meta?.viewSwitch?.option ?? 'view';
    const field = meta?.options?.find(item => item.key === optKey && item.kind === 'choice');
    const choices = meta?.viewSwitch?.choices ?? field?.choices ?? null;
    if (choices && choices.length > 0) {
      const covered = familyByKind.get(entry.kind);
      for (const choice of choices) {
        if (!covered?.has(choice.value)) addPair(entry.kind, choice.value);
      }
    } else {
      addPair(entry.kind, 'default');
    }
  }
  return pairs.sort();
}

export async function checkLegacyRatchet(root) {
  const ratchetPath = resolve(root, 'tools/legacy-widgets-ratchet.json');
  const ratchetData = JSON.parse(readFileSync(ratchetPath, 'utf8'));
  const maxLegacyViews = ratchetData.maxLegacyViews ?? ratchetData.maxLegacyWidgets;
  if (typeof maxLegacyViews !== 'number' || !Number.isInteger(maxLegacyViews) || maxLegacyViews < 0) {
    throw new Error('tools/legacy-widgets-ratchet.json must set integer maxLegacyViews >= 0');
  }

  const legacyPath = resolve(root, 'ui/src/widgets/legacyManifest.ts');
  const legacyFileExists = existsSync(legacyPath);
  const legacyEntries = legacyFileExists ? parseLegacyManifestEntries(legacyPath) : [];
  const { metaByKind, familyByKind } = legacyEntries.length > 0
    ? await loadMetaAndFamilyViews(root)
    : { metaByKind: new Map(), familyByKind: new Map() };
  const pairs = expandLegacyViewPairs(legacyEntries, metaByKind, familyByKind);
  const count = pairs.length;

  const errors = [];
  const notices = [];

  if (count > maxLegacyViews) {
    errors.push(`legacy renderer has ${count} (kind, view) pair(s), above ratchet ${maxLegacyViews}`);
  } else if (count > 0 && count < maxLegacyViews) {
    notices.push(`legacy renderer now has ${count} (kind, view) pair(s) (ratchet is ${maxLegacyViews}); lower maxLegacyViews in tools/legacy-widgets-ratchet.json to ${count}`);
  }

  if (count === 0) {
    if (maxLegacyViews !== 0) {
      errors.push(`legacy views are gone; lower maxLegacyViews in tools/legacy-widgets-ratchet.json from ${maxLegacyViews} to 0`);
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

  return { count, maxLegacyViews, maxLegacyWidgets: maxLegacyViews, pairs, errors, notices };
}

export async function runDeadCodeCheck(root) {
  const ui = checkUiModules(root);
  const rust = checkRustProducts(root, ui);
  const legacy = await checkLegacyRatchet(root);
  const totalIssues = ui.deadProduction.length
    + ui.deadTests.length
    + ui.deadFixturesAndSupport.length
    + rust.deadProducts.length
    + rust.unwiredRegistrations.length
    + legacy.errors.length;
  return { ui, rust, legacy, totalIssues };
}

export function classifyAgainstBase(result, baseResult, baseSha = 'base') {
  const shortBase = baseSha.slice(0, 9);
  const baseDeadProd = new Set(baseResult.ui.deadProduction);
  const baseDeadTestTargets = new Map(baseResult.ui.deadTests.map(entry => [entry.test, new Set(entry.targets)]));
  const baseDeadFixtures = new Set(baseResult.ui.deadFixturesAndSupport.map(entry => entry.file));
  const baseDeadProducts = new Set(baseResult.rust.deadProducts.map(entry => entry.product));
  const baseUnwired = new Set(baseResult.rust.unwiredRegistrations);
  const baseLegacyPairs = new Set(baseResult.legacy.pairs);
  const baseLegacyErrors = new Set(baseResult.legacy.errors);
  const baseOverRatchet = baseResult.legacy.count > baseResult.legacy.maxLegacyViews;

  const addedLegacyPairs = result.legacy.pairs.filter(pair => !baseLegacyPairs.has(pair));
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

  const legacyErrors = result.legacy.errors.map(err => {
    if (err.includes('above ratchet')) {
      const text = addedLegacyPairs.length > 0 ? `${err} (added: ${addedLegacyPairs.join(', ')})` : err;
      const inherited = baseOverRatchet && addedLegacyPairs.length === 0 && result.legacy.count <= baseResult.legacy.count;
      return { error: text, inherited, tag: tag(inherited) };
    }
    const inherited = baseLegacyErrors.has(err);
    return { error: err, inherited, tag: tag(inherited) };
  });

  return {
    shortBase,
    addedLegacyPairs,
    newIssues,
    inheritedIssues,
    deadProduction,
    deadTests,
    deadFixturesAndSupport,
    deadProducts,
    unwiredRegistrations,
    legacyErrors,
  };
}

export function formatReport(result, comparison = null) {
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
  if (result.legacy.errors.length > 0) {
    lines.push(`Legacy view ratchet errors (${result.legacy.errors.length}):`);
    if (comparison) {
      for (const entry of comparison.legacyErrors) lines.push(`  - ${entry.error} ${entry.tag}`);
    } else {
      for (const err of result.legacy.errors) lines.push(`  - ${err}`);
    }
  }
  for (const notice of result.legacy.notices) {
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

function resolveBaseCommit(root, explicitRef) {
  const baseSha = explicitRef
    ? gitText(root, ['rev-parse', '--verify', '--quiet', `${explicitRef}^{commit}`])
    : (gitText(root, ['merge-base', 'HEAD', 'origin/main']) || gitText(root, ['merge-base', 'HEAD', 'main']));
  if (!baseSha) return null;
  const headSha = gitText(root, ['rev-parse', '--verify', '--quiet', 'HEAD']);
  const dirty = gitText(root, ['status', '--porcelain']);
  if (baseSha === headSha && !dirty) return null;
  return baseSha;
}

async function runCheckAtCommit(root, sha) {
  const baseDir = mkdtempSync(join(tmpdir(), 'bp-dead-code-base-'));
  const tarPath = join(baseDir, '__base.tar');
  const nodeModulesLink = join(baseDir, 'node_modules');
  const repoNodeModules = resolve(root, 'node_modules');
  let linkedNodeModules = false;
  try {
    execFileSync(
      'git',
      [
        'archive',
        '--output',
        tarPath,
        sha,
        '--',
        'ui/src',
        'tools/legacy-widgets-ratchet.json',
        ':(glob)ui/*.html',
        ':(glob)crates/*/src/**/*.rs',
        ':(glob)tools/*.mjs',
        ':(glob)tools/*.py',
        ':(glob)tools/*.ps1',
        ':(glob)tools/*.json',
        ':(glob)tools/*.toml',
      ],
      { cwd: root, stdio: ['ignore', 'ignore', 'pipe'] },
    );
    execFileSync('tar', ['-xf', tarPath, '-C', baseDir], { stdio: ['ignore', 'ignore', 'pipe'] });
    rmSync(tarPath, { force: true });
    if (existsSync(repoNodeModules)) {
      symlinkSync(repoNodeModules, nodeModulesLink, 'junction');
      linkedNodeModules = true;
    }
    return await runDeadCodeCheck(baseDir);
  } finally {
    if (linkedNodeModules) {
      try { unlinkSync(nodeModulesLink); } catch {}
    }
    rmSync(baseDir, { recursive: true, force: true });
  }
}

function parseAgainstArg(argv) {
  const index = argv.indexOf('--against');
  if (index >= 0 && index + 1 < argv.length) return argv[index + 1];
  const prefixed = argv.find(arg => arg.startsWith('--against='));
  return prefixed ? prefixed.slice('--against='.length) : null;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const reportOnly = process.argv.includes('--report-only');
  const noBase = process.argv.includes('--no-base');
  const againstArg = parseAgainstArg(process.argv);
  const root = fileURLToPath(new URL('../', import.meta.url));
  const result = await runDeadCodeCheck(root);
  let comparison = null;
  if (result.totalIssues > 0 && !noBase) {
    const baseSha = resolveBaseCommit(root, againstArg);
    if (baseSha) {
      const baseResult = await runCheckAtCommit(root, baseSha);
      comparison = classifyAgainstBase(result, baseResult, baseSha);
    }
  }
  const body = formatReport(result, comparison);
  if (body) console.log(body);
  const summary = `dead-code: ${result.ui.deadProduction.length} dead UI module(s), `
    + `${result.ui.deadTests.length} dead test(s), `
    + `${result.ui.deadFixturesAndSupport.length} dead fixture/support file(s), `
    + `${result.rust.deadProducts.length} dead Rust product(s), `
    + `${result.rust.unwiredRegistrations.length} unwired registration(s), `
    + `${result.legacy.count}/${result.legacy.maxLegacyViews} legacy view(s)`;
  if (result.totalIssues > 0) {
    if (reportOnly) {
      console.log(`${summary} (report-only)`);
      process.exit(0);
    }
    if (comparison && comparison.newIssues === 0) {
      console.log(`${summary} (OK: 0 new on this branch; ${comparison.inheritedIssues} inherited from ${comparison.shortBase})`);
      process.exit(0);
    }
    const detail = comparison
      ? `FAILED: ${comparison.newIssues} new on this branch, ${comparison.inheritedIssues} inherited from ${comparison.shortBase}`
      : 'FAILED';
    console.error(`${summary} (${detail})`);
    process.exit(1);
  }
  console.log(`${summary} (OK)`);
}
