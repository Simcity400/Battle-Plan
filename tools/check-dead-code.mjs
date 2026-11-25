import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, extname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseAst } from 'rolldown/parseAst';
import { astLang, extractImportSpecifiers, relPosix, resolveLocalImport, staticStringLiteral, walkAst, walkImportGraph } from './ui-import-graph.mjs';
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
  'ui/src/widgets/legacyManifest.ts',
  'ui/src/widgets/familyManifest.ts',
]);
const LEGACY_DISPATCH_MODULES = new Set([
  'ui/src/widgets/legacyManifest.ts',
  'ui/src/widgets/lazyWidget.tsx',
  'ui/src/widgets/index.ts',
]);
const MIXED_MANIFEST_MODULE = 'ui/src/widgets/manifest.ts';
const MIXED_LOADER_MODULE = 'ui/src/widgets/rendererLoaders.ts';
const LEGACY_MANIFEST_EXPORTS = new Set([
  'LEGACY_WIDGETS',
  'legacyEntriesForKind',
  'manifestLegacyEntry',
]);
const SAFE_LOADER_EXPORTS = new Set([
  'preloadStatus',
  'whenPreloadComplete',
  'isPlaybackActive',
  'setPlaybackActive',
  'rendererKinds',
]);
const SAFE_REGISTRY_STOPS = new Set([
  'ui/src/widgets/familyManifest.ts',
  'ui/src/widgets/definitions.ts',
]);
const GLOBAL_REGISTRIES = new Set([
  ...LEGACY_DISPATCH_MODULES,
  MIXED_MANIFEST_MODULE,
  MIXED_LOADER_MODULE,
  ...SAFE_REGISTRY_STOPS,
]);
const REACT_ELEMENT_RENDER_APIS = new Set([
  'createElement',
  'cloneElement',
  'createRoot',
  'hydrateRoot',
  'createPortal',
  'jsx',
  'jsxs',
  'jsxDEV',
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
      return { missing: true, maxLegacyViews: 0, maxLegacyRendererViews: 0, maxUnusedExports: 0 };
    }
    throw new Error('Missing tools/legacy-widgets-ratchet.json');
  }
  const data = JSON.parse(readFileSync(ratchetPath, 'utf8'));
  const required = ['maxLegacyViews', 'maxLegacyRendererViews', 'maxUnusedExports'];
  for (const field of required) {
    const val = data[field];
    if (typeof val !== 'number' || !Number.isInteger(val) || val < 0) {
      throw new Error(`tools/legacy-widgets-ratchet.json must set integer ${field} >= 0`);
    }
  }
  return { ...data, missing: false };
}

function collectAstBindings(ast) {
  const bindings = new Map();
  for (const stmt of ast.body) {
    const declNode = stmt.type === 'ExportNamedDeclaration' ? stmt.declaration : stmt;
    if (declNode?.type === 'VariableDeclaration') {
      for (const decl of declNode.declarations) {
        if (decl.id?.type === 'Identifier' && decl.init) {
          bindings.set(decl.id.name, decl.init);
        }
      }
    } else if (declNode?.type === 'FunctionDeclaration' && declNode.id?.name) {
      bindings.set(declNode.id.name, declNode);
    }
  }
  return bindings;
}

function collectAllImportsIn(node, fromFile, root, localBindings = null, visiting = new Set()) {
  const targets = [];
  const seen = new Set();
  let hasUnresolvedImport = false;

  const visit = rootNode => {
    walkAst(rootNode, (inner, parent, key) => {
      if (inner.type === 'ImportExpression') {
        const spec = staticStringLiteral(inner.source);
        if (spec === null) {
          hasUnresolvedImport = true;
          return;
        }
        const resolved = resolveLocalImport(fromFile, spec, root);
        if (!resolved) {
          hasUnresolvedImport = true;
          return;
        }
        const rel = relPosix(root, resolved);
        if (!seen.has(rel)) {
          seen.add(rel);
          targets.push(rel);
        }
        return;
      }
      if (localBindings && inner.type === 'Identifier' && localBindings.has(inner.name) && !visiting.has(inner.name)) {
        const isPropKey = (parent?.type === 'Property' && key === 'key' && !parent.computed && !parent.shorthand)
          || (parent?.type === 'MemberExpression' && key === 'property' && !parent.computed);
        if (!isPropKey) {
          visiting.add(inner.name);
          visit(localBindings.get(inner.name));
        }
      }
    });
  };

  visit(node);
  return { targets, hasUnresolvedImport };
}

function resolveImportsFromFunction(fn, fromFile, root) {
  if (typeof fn !== 'function') return { targets: [], hasUnresolvedImport: false };
  try {
    const ast = parseAst(`(${fn.toString()})`, { lang: 'ts' });
    return collectAllImportsIn(ast, fromFile, root);
  } catch {
    return { targets: [], hasUnresolvedImport: true };
  }
}

function parseLegacyManifestAstFallback(legacyPath, root) {
  const byKey = new Map();
  if (!existsSync(legacyPath)) return byKey;
  const ast = parseAst(readFileSync(legacyPath, 'utf8'), { lang: 'ts' });
  const localBindings = collectAstBindings(ast);
  walkAst(ast, node => {
    if (node.type !== 'ObjectExpression') return;
    let kind = null;
    let view = '';
    let collected = null;
    for (const prop of node.properties) {
      if (prop.type !== 'Property') continue;
      const propName = prop.key?.name ?? prop.key?.value;
      if (propName === 'kind') kind = staticStringLiteral(prop.value);
      else if (propName === 'view') view = staticStringLiteral(prop.value) ?? '';
      else if (propName === 'load') {
        collected = collectAllImportsIn(prop.value, legacyPath, root, localBindings);
      }
    }
    if (kind && collected) byKey.set(`${kind}\0${view}`, collected);
  });
  return byKey;
}

async function loadManifestData(root) {
  const metaByKind = new Map();
  const familyViews = [];
  const familyByKind = new Map();
  const legacyEntries = [];
  const unresolvedNativeLoaders = [];
  const familyViewUiEntries = new Map();
  const familyViewUiViews = new Map();
  const stamp = `${Date.now()}-${Math.random().toString(36).slice(2)}`;

  const definitionsPath = resolve(root, 'ui/src/widgets/definitions.ts');
  if (existsSync(definitionsPath)) {
    const mod = await import(`${pathToFileURL(definitionsPath).href}?t=${stamp}`);
    for (const meta of mod.widgetMetas ?? []) {
      if (meta && typeof meta.kind === 'string') metaByKind.set(meta.kind, meta);
    }
  }

  const familyManifestPath = resolve(root, 'ui/src/widgets/familyManifest.ts');
  if (existsSync(familyManifestPath)) {
    const mod = await import(`${pathToFileURL(familyManifestPath).href}?t=${stamp}`);
    for (const entry of mod.FAMILY_VIEWS ?? []) {
      if (!entry || typeof entry.kind !== 'string') continue;
      familyViews.push(entry);
      const view = typeof entry.view === 'string' && entry.view.length > 0 ? entry.view : 'default';
      let set = familyByKind.get(entry.kind);
      if (!set) familyByKind.set(entry.kind, set = new Set());
      set.add(view);
    }
  }

  const legacyPath = resolve(root, 'ui/src/widgets/legacyManifest.ts');
  const legacyFileExists = existsSync(legacyPath);
  if (legacyFileExists) {
    const mod = await import(`${pathToFileURL(legacyPath).href}?t=${stamp}`);
    const astFallback = parseLegacyManifestAstFallback(legacyPath, root);
    const evaluated = mod.LEGACY_WIDGETS;
    if (!Array.isArray(evaluated)) {
      throw new Error('Could not find exported LEGACY_WIDGETS array in ui/src/widgets/legacyManifest.ts');
    }
    for (const entry of evaluated) {
      if (!entry || typeof entry.kind !== 'string') continue;
      const view = typeof entry.view === 'string' && entry.view.length > 0 ? entry.view : null;
      const fromFn = resolveImportsFromFunction(entry.load, legacyPath, root);
      const fromAst = astFallback.get(`${entry.kind}\0${view ?? ''}`);
      const chosen = fromFn.targets.length > 0 || fromFn.hasUnresolvedImport ? fromFn : (fromAst ?? fromFn);
      legacyEntries.push({
        kind: entry.kind,
        view,
        loadTarget: chosen.targets[0] ?? null,
        loadTargets: chosen.targets,
        hasUnresolvedImport: chosen.hasUnresolvedImport,
        hasLoader: typeof entry.load === 'function',
      });
    }
  }

  const manifestPath = resolve(root, 'ui/src/widgets/manifest.ts');
  if (existsSync(manifestPath)) {
    const ast = parseAst(readFileSync(manifestPath, 'utf8'), { lang: 'ts' });
    const localBindings = collectAstBindings(ast);
    const astUiTargets = new Map();
    for (const node of ast.body) {
      if (node.type !== 'ExportNamedDeclaration' || node.declaration?.type !== 'VariableDeclaration') continue;
      for (const decl of node.declaration.declarations) {
        if (decl.id?.type !== 'Identifier' || decl.id.name !== 'FAMILY_VIEW_UI') continue;
        const init = unwrapExpression(decl.init);
        if (init?.type !== 'ObjectExpression') continue;
        for (const prop of init.properties) {
          if (prop.type !== 'Property') continue;
          const kind = prop.key?.value ?? prop.key?.name;
          if (typeof kind !== 'string') continue;
          astUiTargets.set(kind, collectAllImportsIn(prop.value, manifestPath, root, localBindings));
        }
      }
    }
    try {
      const mod = await import(`${pathToFileURL(manifestPath).href}?t=${stamp}`);
      for (const entry of mod.NATIVE_WIDGETS ?? []) {
        const { targets, hasUnresolvedImport } = resolveImportsFromFunction(entry.load, manifestPath, root);
        if (targets.length === 0 || hasUnresolvedImport) unresolvedNativeLoaders.push(entry.kind);
      }
      if (mod.FAMILY_VIEW_UI && typeof mod.FAMILY_VIEW_UI === 'object') {
        for (const [kind, loader] of Object.entries(mod.FAMILY_VIEW_UI)) {
          const fromFn = resolveImportsFromFunction(loader?.load, manifestPath, root);
          const fromAst = astUiTargets.get(kind);
          const chosen = fromFn.targets.length > 0 || fromFn.hasUnresolvedImport ? fromFn : (fromAst ?? fromFn);
          familyViewUiEntries.set(kind, chosen);
          if (Array.isArray(loader?.views)) familyViewUiViews.set(kind, loader.views);
        }
      }
    } catch {
      for (const [kind, collected] of astUiTargets) familyViewUiEntries.set(kind, collected);
    }
  }

  return {
    metaByKind,
    familyViews,
    familyByKind,
    legacyFileExists,
    legacyEntries,
    unresolvedNativeLoaders,
    familyViewUiEntries,
    familyViewUiViews,
  };
}

function selectLegacyViewChoices(meta) {
  const viewField = meta?.options?.find(item => item.key === 'view' && item.kind === 'choice');
  if (viewField?.choices?.length) return viewField.choices;
  if (meta?.viewSwitch?.choices?.length) return meta.viewSwitch.choices;
  if (meta?.viewSwitch?.option) {
    const switchField = meta.options?.find(item => item.key === meta.viewSwitch.option && item.kind === 'choice');
    if (switchField?.choices?.length) return switchField.choices;
  }
  return null;
}

function expandLegacyViewPairs(legacyEntries, metaByKind, familyByKind) {
  const pairs = [];
  const seen = new Set();
  const staleEntries = [];
  const activeLegacyTargets = new Set();
  const unresolvedLoaders = [];

  const addPair = (kind, view) => {
    const pair = `${kind}:${view}`;
    if (!seen.has(pair)) {
      seen.add(pair);
      pairs.push(pair);
    }
  };

  for (const entry of legacyEntries) {
    const explicitView = typeof entry.view === 'string' && entry.view.length > 0 && entry.view !== 'default'
      ? entry.view
      : null;
    const label = explicitView ? `${entry.kind}:${explicitView}` : `${entry.kind}:default`;
    const targets = entry.loadTargets ?? (entry.loadTarget ? [entry.loadTarget] : []);

    if (entry.hasLoader && (entry.hasUnresolvedImport || targets.length === 0)) {
      unresolvedLoaders.push(label);
    }

    const covered = familyByKind.get(entry.kind);
    if (explicitView) {
      if (covered?.has(explicitView)) {
        staleEntries.push(label);
        continue;
      }
      addPair(entry.kind, explicitView);
      for (const target of targets) activeLegacyTargets.add(target);
      continue;
    }

    const meta = metaByKind.get(entry.kind);
    const choices = selectLegacyViewChoices(meta);
    if (choices && choices.length > 0) {
      let addedAny = false;
      for (const choice of choices) {
        if (!covered?.has(choice.value)) {
          addPair(entry.kind, choice.value);
          addedAny = true;
        }
      }
      if (!addedAny) {
        staleEntries.push(label);
      } else {
        for (const target of targets) activeLegacyTargets.add(target);
      }
    } else {
      if (covered?.has('default')) {
        staleEntries.push(label);
      } else {
        addPair(entry.kind, 'default');
        for (const target of targets) activeLegacyTargets.add(target);
      }
    }
  }

  return {
    pairs: pairs.sort(),
    staleEntries: staleEntries.sort(),
    unresolvedLoaders: unresolvedLoaders.sort(),
    activeLegacyTargets,
  };
}

export function checkUiAndExports(root, activeLegacyTargets, ratchetData = null) {
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
  } = analyzeUiSymbolGraph(root, allFiles, roleByRel, entryPoints, activeLegacyTargets);

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

function parseFamilyManifestSpecs(root, runtimeFamilyViews) {
  const dir = resolve(root, 'ui/src/widgets/manifest');
  const byPair = new Map();
  const runtimeViewsByKindFamily = new Map();
  for (const entry of runtimeFamilyViews) {
    const view = typeof entry.view === 'string' && entry.view.length > 0 ? entry.view : 'default';
    const key = `${entry.kind}\0${entry.family}`;
    let list = runtimeViewsByKindFamily.get(key);
    if (!list) runtimeViewsByKindFamily.set(key, list = []);
    list.push(view);
  }

  const pushEntry = (kind, view, family, collected) => {
    const pair = `${kind}:${view}`;
    if (!byPair.has(pair)) {
      byPair.set(pair, {
        kind,
        view,
        family,
        specTargets: collected.targets,
        hasUnresolvedImport: collected.hasUnresolvedImport,
      });
    }
  };

  if (existsSync(dir)) {
    const files = readdirSync(dir).filter(name => name.endsWith('.ts') && !name.endsWith('.test.ts')).sort();

    for (const file of files) {
      const full = resolve(dir, file);
      const family = file.slice(0, -'.ts'.length);
      const ast = parseAst(readFileSync(full, 'utf8'), { lang: 'ts' });
      const localBindings = collectAstBindings(ast);

      const localStringArrays = new Map();
      for (const stmt of ast.body) {
        const declNode = stmt.type === 'ExportNamedDeclaration' ? stmt.declaration : stmt;
        if (declNode?.type !== 'VariableDeclaration') continue;
        for (const decl of declNode.declarations) {
          if (decl.id?.type !== 'Identifier') continue;
          const init = unwrapExpression(decl.init);
          if (init?.type === 'ArrayExpression') {
            const values = init.elements.map(el => staticStringLiteral(unwrapExpression(el)));
            if (values.length > 0 && values.every(v => v !== null)) {
              localStringArrays.set(decl.id.name, values);
            }
          }
        }
      }

      const record = (kind, view, collected) => {
        if (!kind || !collected) return;
        if (view === '*') {
          const concrete = runtimeViewsByKindFamily.get(`${kind}\0${family}`) ?? [];
          for (const v of concrete) pushEntry(kind, v, family, collected);
        } else {
          pushEntry(kind, view, family, collected);
        }
      };

      const resolveStringExpr = (expr, env) => {
        const unwrapped = unwrapExpression(expr);
        const lit = staticStringLiteral(unwrapped);
        if (lit !== null) return lit;
        if (unwrapped?.type === 'Identifier') {
          if (env && env.has(unwrapped.name)) return env.get(unwrapped.name);
          if (unwrapped.name === 'DEFAULT_VIEW') return 'default';
        }
        return null;
      };

      const visitEntryNode = (node, env = null) => {
        const unwrapped = unwrapExpression(node);
        if (!unwrapped) return;
        if (unwrapped.type === 'ObjectExpression') {
          let kind = null;
          let view = 'default';
          let collected = null;
          for (const prop of unwrapped.properties) {
            if (prop.type !== 'Property') continue;
            const propName = prop.key?.name ?? prop.key?.value;
            if (propName === 'kind') {
              kind = resolveStringExpr(prop.value, env);
            } else if (propName === 'view') {
              const resolvedView = resolveStringExpr(prop.value, env);
              view = resolvedView !== null ? resolvedView : '*';
            } else if (propName === 'loadSpec') {
              collected = collectAllImportsIn(prop.value, full, root, localBindings);
            }
          }
          record(kind, view, collected);
        } else if (unwrapped.type === 'CallExpression' && unwrapped.arguments.length >= 2) {
          const kind = resolveStringExpr(unwrapped.arguments[0], env);
          if (!kind) return;
          const collected = collectAllImportsIn(unwrapped, full, root, localBindings);
          const resolvedView = resolveStringExpr(unwrapped.arguments[1], env);
          const view = resolvedView !== null ? resolvedView : 'default';
          record(kind, view, collected);
        }
      };

      const extractArrayStrings = expr => {
        const unwrapped = unwrapExpression(expr);
        if (unwrapped?.type === 'ArrayExpression') {
          const vals = unwrapped.elements.map(el => staticStringLiteral(unwrapExpression(el)));
          return vals.length > 0 && vals.every(v => v !== null) ? vals : null;
        }
        if (unwrapped?.type === 'Identifier' && localStringArrays.has(unwrapped.name)) {
          return localStringArrays.get(unwrapped.name);
        }
        return null;
      };

      const visitSpread = arg => {
        const unwrapped = unwrapExpression(arg);
        if (unwrapped?.type === 'CallExpression'
          && unwrapped.callee?.type === 'MemberExpression'
          && !unwrapped.callee.computed
          && unwrapped.callee.property?.name === 'map'
          && unwrapped.arguments.length >= 1) {
          const cb = unwrapExpression(unwrapped.arguments[0]);
          const paramName = cb && (cb.type === 'ArrowFunctionExpression' || cb.type === 'FunctionExpression')
            && cb.params[0]?.type === 'Identifier'
            ? cb.params[0].name
            : null;
          const items = extractArrayStrings(unwrapped.callee.object);
          if (paramName && items) {
            for (const item of items) {
              const env = new Map([[paramName, item]]);
              walkAst(cb.body, inner => {
                if (inner.type === 'ObjectExpression') visitEntryNode(inner, env);
              });
            }
            return;
          }
        }
        walkAst(arg, inner => {
          if (inner.type === 'ObjectExpression') visitEntryNode(inner);
        });
      };

      for (const stmt of ast.body) {
        if (stmt.type !== 'ExportNamedDeclaration' || stmt.declaration?.type !== 'VariableDeclaration') continue;
        for (const decl of stmt.declaration.declarations) {
          const init = unwrapExpression(decl.init);
          if (init?.type !== 'ArrayExpression') continue;
          for (const el of init.elements) {
            if (!el) continue;
            if (el.type === 'SpreadElement') visitSpread(el.argument);
            else visitEntryNode(el);
          }
        }
      }
    }
  }

  const resolved = [];
  const unresolved = [];
  const seenPairs = new Set();

  for (const entry of runtimeFamilyViews) {
    const view = typeof entry.view === 'string' && entry.view.length > 0 ? entry.view : 'default';
    const pair = `${entry.kind}:${view}`;
    if (seenPairs.has(pair)) continue;
    seenPairs.add(pair);
    const hit = byPair.get(pair);
    if (hit && !hit.hasUnresolvedImport && hit.specTargets.length > 0) {
      resolved.push(hit);
    } else if (hit?.hasUnresolvedImport) {
      unresolved.push({ pair, kind: entry.kind, view, family: entry.family ?? 'unknown' });
    } else {
      const fromFile = entry.family ? resolve(root, 'ui/src/widgets/manifest', `${entry.family}.ts`) : resolve(root, 'ui/src/widgets/familyManifest.ts');
      const fromFn = resolveImportsFromFunction(entry.loadSpec, fromFile, root);
      if (!fromFn.hasUnresolvedImport && fromFn.targets.length > 0) {
        resolved.push({ kind: entry.kind, view, family: entry.family ?? 'unknown', specTargets: fromFn.targets });
      } else {
        unresolved.push({ pair, kind: entry.kind, view, family: entry.family ?? 'unknown' });
      }
    }
  }

  resolved.sort((a, b) => `${a.kind}:${a.view}`.localeCompare(`${b.kind}:${b.view}`));
  unresolved.sort((a, b) => a.pair.localeCompare(b.pair));
  return { resolved, unresolved };
}

export function checkLegacyRatchet(root, loaded, ratchetData = null) {
  const data = ratchetData ?? readRatchetFile(root);
  const maxLegacyViews = data.maxLegacyViews;

  const { pairs, staleEntries, unresolvedLoaders, activeLegacyTargets } = expandLegacyViewPairs(
    loaded.legacyEntries,
    loaded.metaByKind,
    loaded.familyByKind,
  );
  const count = pairs.length;

  const errors = loaded.unresolvedNativeLoaders.map(kind => `NATIVE_WIDGETS entry ${kind} has an unresolvable load target`);
  const notices = [];

  for (const stale of staleEntries) {
    errors.push(`stale LEGACY_WIDGETS entry ${stale} is already covered by FAMILY_VIEWS`);
  }
  for (const bad of unresolvedLoaders) {
    errors.push(`LEGACY_WIDGETS entry ${bad} has an unresolvable load target`);
  }

  if (count > maxLegacyViews) {
    errors.push(`legacy renderer has ${count} (kind, view) pair(s), above ratchet ${maxLegacyViews}`);
  } else if (count > 0 && count < maxLegacyViews) {
    notices.push(`legacy renderer now has ${count} (kind, view) pair(s) (ratchet is ${maxLegacyViews}); lower maxLegacyViews in tools/legacy-widgets-ratchet.json to ${count}`);
  }

  if (count === 0) {
    if (maxLegacyViews !== 0) {
      errors.push(`legacy views are gone; lower maxLegacyViews in tools/legacy-widgets-ratchet.json from ${maxLegacyViews} to 0`);
    }
    if (loaded.legacyFileExists) {
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

  return {
    count,
    maxLegacyViews,
    maxLegacyWidgets: maxLegacyViews,
    pairs,
    staleEntries,
    unresolvedLoaders,
    activeLegacyTargets,
    legacyEntries: loaded.legacyEntries,
    errors,
    notices,
  };
}

function rendersReactElements(root, relPath) {
  const full = resolve(root, relPath);
  if (!existsSync(full)) return false;
  const ext = extname(full).toLowerCase();
  if (!SCRIPT_EXTENSIONS.has(ext)) return false;
  const ast = parseAst(readFileSync(full, 'utf8'), { lang: astLang(full) });
  const reactNamespaces = new Set();
  for (const stmt of ast.body) {
    if (stmt.type !== 'ImportDeclaration' || stmt.importKind === 'type') continue;
    const spec = staticStringLiteral(stmt.source);
    if (spec !== 'react' && spec !== 'react-dom' && spec !== 'react-dom/client' && !spec?.startsWith('react/')) continue;
    for (const item of stmt.specifiers) {
      if (item.importKind === 'type') continue;
      if (item.type === 'ImportSpecifier') {
        const name = item.imported?.name ?? item.imported?.value;
        if (REACT_ELEMENT_RENDER_APIS.has(name)) return true;
      } else if (item.type === 'ImportDefaultSpecifier' || item.type === 'ImportNamespaceSpecifier') {
        reactNamespaces.add(item.local.name);
      }
    }
  }
  if (reactNamespaces.size === 0) return false;
  let found = false;
  walkAst(ast, node => {
    if (found) return;
    if (node.type === 'MemberExpression' && !node.computed
      && node.object?.type === 'Identifier' && reactNamespaces.has(node.object.name)
      && node.property?.type === 'Identifier' && REACT_ELEMENT_RENDER_APIS.has(node.property.name)) {
      found = true;
    }
  });
  return found;
}

function importsLegacyFromMixedModule(root, fromRel, targetModule, isForbiddenExport, cache) {
  const cacheKey = `${fromRel}\0${targetModule}`;
  if (cache.has(cacheKey)) return cache.get(cacheKey);
  const full = resolve(root, fromRel);
  if (!existsSync(full) || !SCRIPT_EXTENSIONS.has(extname(full).toLowerCase())) {
    cache.set(cacheKey, false);
    return false;
  }
  const ast = parseAst(readFileSync(full, 'utf8'), { lang: astLang(full) });
  let usesLegacy = false;
  for (const stmt of ast.body) {
    if (stmt.type === 'ImportDeclaration' && stmt.importKind !== 'type') {
      const spec = staticStringLiteral(stmt.source);
      const target = spec !== null ? resolveLocalImport(full, spec, root) : null;
      if (!target || relPosix(root, target) !== targetModule) continue;
      if (stmt.specifiers.length === 0) {
        usesLegacy = true;
        break;
      }
      for (const item of stmt.specifiers) {
        if (item.importKind === 'type') continue;
        if (item.type === 'ImportNamespaceSpecifier' || item.type === 'ImportDefaultSpecifier') {
          usesLegacy = true;
          break;
        }
        if (item.type === 'ImportSpecifier') {
          const imported = item.imported?.name ?? item.imported?.value;
          if (isForbiddenExport(imported)) {
            usesLegacy = true;
            break;
          }
        }
      }
    }
    if (usesLegacy) break;
  }
  if (!usesLegacy) {
    const parents = new Map();
    walkAst(ast, (node, parent, key) => { parents.set(node, { parent, key }); });
    walkAst(ast, node => {
      if (usesLegacy || node.type !== 'ImportExpression') return;
      const spec = staticStringLiteral(node.source);
      const target = spec !== null ? resolveLocalImport(full, spec, root) : null;
      if (!target || relPosix(root, target) !== targetModule) return;
      let cur = node;
      let info = parents.get(cur);
      while (info?.parent && (info.parent.type === 'AwaitExpression' || info.parent.type === 'ParenthesizedExpression')) {
        cur = info.parent;
        info = parents.get(cur);
      }
      if (info?.parent?.type === 'MemberExpression' && info.key === 'object' && !info.parent.computed && info.parent.property?.type === 'Identifier') {
        if (isForbiddenExport(info.parent.property.name)) usesLegacy = true;
      } else {
        usesLegacy = true;
      }
    });
  }
  cache.set(cacheKey, usesLegacy);
  return usesLegacy;
}

function importsCssDirectly(root, relPath) {
  const full = resolve(root, relPath);
  if (!existsSync(full)) return false;
  const specs = extractImportSpecifiers(full, { includeTypes: false, includeDynamic: false, includeWorkerUrls: false });
  return specs.some(spec => spec.specifier.endsWith('.css'));
}

export function checkConvertedLegacyReach(root, uiResult, legacyResult, loaded, ratchetData = null) {
  const data = ratchetData ?? readRatchetFile(root);
  const maxLegacyRendererViews = data.maxLegacyRendererViews;

  const { resolved: familyViews, unresolved: unresolvedFamilyViews } = parseFamilyManifestSpecs(root, loaded.familyViews);
  const familyViewUi = loaded.familyViewUiEntries;

  const unresolvedUiLoaders = [];
  for (const [kind, entry] of familyViewUi) {
    if (!entry || entry.hasUnresolvedImport || entry.targets.length === 0) unresolvedUiLoaders.push(kind);
  }
  unresolvedUiLoaders.sort();

  const legacyStarts = [...new Set(
    legacyResult.legacyEntries.flatMap(entry => entry.loadTargets ?? (entry.loadTarget ? [entry.loadTarget] : [])).filter(Boolean),
  )];
  const legacyGraph = walkImportGraph(legacyStarts, root, {
    includeTypes: false,
    includeDynamic: true,
    includeWorkerUrls: true,
    shouldTraverse: file => {
      const rel = relPosix(root, file);
      return !rel.startsWith('ui/src/families/') && !GLOBAL_REGISTRIES.has(rel);
    },
  });
  const legacyReach = new Set([...legacyGraph.visited].map(file => relPosix(root, file)));

  const dashShellPath = resolve(root, 'ui/src/dashboard/Dashboard.tsx');
  const shellReach = new Set();
  if (existsSync(dashShellPath)) {
    const shellGraph = walkImportGraph(['ui/src/dashboard/Dashboard.tsx'], root, {
      includeTypes: false,
      includeDynamic: true,
      includeWorkerUrls: true,
      shouldTraverse: file => {
        const rel = relPosix(root, file);
        return !rel.startsWith('ui/src/widgets/') && !rel.startsWith('ui/src/families/') && !GLOBAL_REGISTRIES.has(rel);
      },
    });
    for (const file of shellGraph.visited) shellReach.add(relPosix(root, file));
  }

  const oldWidgetRenderers = new Set(legacyStarts);
  for (const file of legacyReach) {
    if (file.startsWith('ui/src/widgets/') && file.endsWith('.tsx')) {
      oldWidgetRenderers.add(file);
    }
  }

  const oldCharts = new Set();
  for (const file of uiResult.runtimeLive) {
    if (!file.startsWith('ui/src/charts/')) continue;
    if (file.endsWith('.tsx') || rendersReactElements(root, file)) {
      oldCharts.add(file);
    }
  }

  const oldDash = new Set();
  for (const file of legacyReach) {
    if (!file.startsWith('ui/src/dashboard/') || shellReach.has(file)) continue;
    if (file.endsWith('.tsx') && importsCssDirectly(root, file)) {
      oldDash.add(file);
    }
  }

  const oldRenderer = new Set([...oldWidgetRenderers, ...oldCharts, ...oldDash, ...LEGACY_DISPATCH_MODULES]);

  const edgeCache = new Map();
  const manifestLegacyCache = new Map();
  const shortestChain = startFiles => {
    const queue = startFiles.map(file => [file]);
    const seen = new Set(startFiles);
    while (queue.length > 0) {
      const chain = queue.shift();
      const cur = chain[chain.length - 1];
      if (oldRenderer.has(cur)) return chain;
      if (SAFE_REGISTRY_STOPS.has(cur)) continue;
      const full = resolve(root, cur);
      if (!existsSync(full)) continue;
      let targets = edgeCache.get(full);
      if (!targets) {
        targets = [...new Set(extractImportSpecifiers(full, { includeTypes: false, includeDynamic: true, includeWorkerUrls: true })
          .map(spec => resolveLocalImport(full, spec.specifier, root))
          .filter(Boolean)
          .map(target => relPosix(root, target)))].sort();
        edgeCache.set(full, targets);
      }
      for (const next of targets) {
        if (next === MIXED_MANIFEST_MODULE) {
          if (importsLegacyFromMixedModule(root, cur, MIXED_MANIFEST_MODULE, name => LEGACY_MANIFEST_EXPORTS.has(name), manifestLegacyCache)) {
            return [...chain, next];
          }
          continue;
        }
        if (next === MIXED_LOADER_MODULE) {
          if (importsLegacyFromMixedModule(root, cur, MIXED_LOADER_MODULE, name => !SAFE_LOADER_EXPORTS.has(name), manifestLegacyCache)) {
            return [...chain, next];
          }
          continue;
        }
        if (seen.has(next)) continue;
        seen.add(next);
        queue.push([...chain, next]);
      }
    }
    return null;
  };

  const violations = [];
  for (const entry of familyViews) {
    const starts = [...entry.specTargets];
    const uiViews = loaded.familyViewUiViews.get(entry.kind);
    const uiEntry = !uiViews || uiViews.includes(entry.view) ? familyViewUi.get(entry.kind) : undefined;
    if (uiEntry && !uiEntry.hasUnresolvedImport) {
      for (const target of uiEntry.targets) {
        if (!starts.includes(target)) starts.push(target);
      }
    }
    const chain = shortestChain(starts);
    if (chain) {
      violations.push({
        pair: `${entry.kind}:${entry.view}`,
        kind: entry.kind,
        view: entry.view,
        family: entry.family,
        target: chain[chain.length - 1],
        chain,
      });
    }
  }

  const errors = [];
  const notices = [];
  for (const item of unresolvedFamilyViews) {
    errors.push(`unresolved FAMILY_VIEWS entry ${item.pair} (${item.family}): could not resolve loadSpec target`);
  }
  for (const kind of unresolvedUiLoaders) {
    errors.push(`unresolved FAMILY_VIEW_UI loader for ${kind}`);
  }
  if (violations.length > maxLegacyRendererViews) {
    errors.push(`${violations.length} converted view(s) reach old-renderer modules, above ratchet ${maxLegacyRendererViews}`);
  } else if (violations.length > 0 && violations.length < maxLegacyRendererViews) {
    notices.push(`converted views reaching old-renderer modules dropped to ${violations.length} (ratchet is ${maxLegacyRendererViews}); lower maxLegacyRendererViews in tools/legacy-widgets-ratchet.json to ${violations.length}`);
  } else if (violations.length === 0 && maxLegacyRendererViews !== 0) {
    errors.push(`no converted view reaches old-renderer modules; lower maxLegacyRendererViews in tools/legacy-widgets-ratchet.json from ${maxLegacyRendererViews} to 0`);
  }

  return {
    totalConvertedViews: familyViews.length,
    unresolvedFamilyViews,
    unresolvedUiLoaders,
    oldWidgetCount: oldWidgetRenderers.size,
    oldChartCount: oldCharts.size,
    oldDashboardCount: oldDash.size,
    maxLegacyRendererViews,
    violations,
    errors,
    notices,
  };
}

export async function runDeadCodeCheck(root, options = {}) {
  const ratchetData = readRatchetFile(root, { allowMissing: Boolean(options.allowMissingRatchet) });
  const loaded = await loadManifestData(root);
  const legacy = checkLegacyRatchet(root, loaded, ratchetData);
  const { ui, exports } = checkUiAndExports(root, legacy.activeLegacyTargets, ratchetData);
  const rust = checkRustProducts(root, ui);
  const convertedLegacy = checkConvertedLegacyReach(root, ui, legacy, loaded, ratchetData);
  const totalIssues = ui.deadProduction.length
    + ui.deadTests.length
    + ui.deadFixturesAndSupport.length
    + ui.productionTestImports.length
    + rust.deadProducts.length
    + rust.unwiredRegistrations.length
    + legacy.errors.length
    + convertedLegacy.errors.length
    + exports.errors.length;
  return { ui, rust, legacy, convertedLegacy, exports, totalIssues, ratchetMissing: Boolean(ratchetData.missing) };
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
  const baseLegacyPairs = baseRatchetMissing ? new Set() : new Set(baseResult.legacy.pairs);
  const baseLegacyErrors = baseRatchetMissing ? new Set() : new Set(baseResult.legacy.errors);
  const baseOverRatchet = !baseRatchetMissing && baseResult.legacy.count > baseResult.legacy.maxLegacyViews;

  const baseConvertedPairs = baseRatchetMissing ? new Set() : new Set((baseResult.convertedLegacy?.violations ?? []).map(v => v.pair));
  const baseConvertedErrors = baseRatchetMissing ? new Set() : new Set(baseResult.convertedLegacy?.errors ?? []);
  const baseConvertedOver = !baseRatchetMissing && (baseResult.convertedLegacy?.violations?.length ?? 0) > baseResult.convertedLegacy.maxLegacyRendererViews;

  const baseUnusedExportKeys = baseRatchetMissing ? new Set() : new Set((baseResult.exports?.unusedExports ?? []).map(item => item.key));
  const baseExportErrors = baseRatchetMissing ? new Set() : new Set(baseResult.exports?.errors ?? []);
  const baseExportsOver = !baseRatchetMissing && (baseResult.exports?.unusedExports?.length ?? 0) > baseResult.exports.maxUnusedExports;

  const kindOf = pair => pair.slice(0, pair.indexOf(':'));
  const legacyViewsOf = (pairs, kind) => pairs.filter(pair => kindOf(pair) === kind).length;
  const namedDefault = pair => baseLegacyPairs.has(`${kindOf(pair)}:default`) && !result.legacy.pairs.includes(`${kindOf(pair)}:default`)
    && legacyViewsOf(result.legacy.pairs, kindOf(pair)) === 1 && legacyViewsOf([...baseLegacyPairs], kindOf(pair)) === 1;
  const addedLegacyPairs = result.legacy.pairs.filter(pair => !baseLegacyPairs.has(pair) && !namedDefault(pair));
  const addedConvertedViolations = result.convertedLegacy.violations.filter(v => !baseConvertedPairs.has(v.pair));
  const addedUnusedExports = result.exports.unusedExports.filter(item => !baseUnusedExportKeys.has(item.key));

  const ratchetRaises = [];
  if (!baseRatchetMissing) {
    if (result.legacy.maxLegacyViews > baseResult.legacy.maxLegacyViews) {
      ratchetRaises.push(`maxLegacyViews raised from ${baseResult.legacy.maxLegacyViews} to ${result.legacy.maxLegacyViews}`);
    }
    if (result.convertedLegacy.maxLegacyRendererViews > baseResult.convertedLegacy.maxLegacyRendererViews) {
      ratchetRaises.push(`maxLegacyRendererViews raised from ${baseResult.convertedLegacy.maxLegacyRendererViews} to ${result.convertedLegacy.maxLegacyRendererViews}`);
    }
    if (result.exports.maxUnusedExports > baseResult.exports.maxUnusedExports) {
      ratchetRaises.push(`maxUnusedExports raised from ${baseResult.exports.maxUnusedExports} to ${result.exports.maxUnusedExports}`);
    }
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

  let legacyAddedReported = false;
  const legacyErrors = result.legacy.errors.map(err => {
    if (err.includes('above ratchet')) {
      legacyAddedReported = true;
      const text = addedLegacyPairs.length > 0 ? `${err} (added: ${addedLegacyPairs.join(', ')})` : err;
      const inherited = baseOverRatchet && addedLegacyPairs.length === 0 && result.legacy.count <= baseResult.legacy.count;
      return { error: text, inherited, tag: tag(inherited) };
    }
    const inherited = baseLegacyErrors.has(err);
    return { error: err, inherited, tag: tag(inherited) };
  });
  if (!legacyAddedReported && (addedLegacyPairs.length > 0 || result.legacy.count > baseResult.legacy.count)) {
    const detail = addedLegacyPairs.length > 0
      ? `legacy renderer added ${addedLegacyPairs.length} (kind, view) pair(s) vs ${shortBase}: ${addedLegacyPairs.join(', ')}`
      : `legacy renderer count increased from ${baseResult.legacy.count} to ${result.legacy.count}`;
    legacyErrors.push({ error: detail, inherited: false, tag: tag(false) });
  }

  let convertedAddedReported = false;
  const convertedLegacyErrors = result.convertedLegacy.errors.map(err => {
    if (err.includes('above ratchet')) {
      convertedAddedReported = true;
      const addedText = addedConvertedViolations.length > 0
        ? ` (added: ${addedConvertedViolations.map(v => v.pair).join(', ')})`
        : '';
      const inherited = baseConvertedOver
        && addedConvertedViolations.length === 0
        && result.convertedLegacy.violations.length <= (baseResult.convertedLegacy?.violations?.length ?? 0);
      return { error: `${err}${addedText}`, inherited, tag: tag(inherited) };
    }
    const inherited = baseConvertedErrors.has(err);
    return { error: err, inherited, tag: tag(inherited) };
  });
  if (!convertedAddedReported && addedConvertedViolations.length > 0) {
    convertedLegacyErrors.push({
      error: `converted views added ${addedConvertedViolations.length} old-renderer violation(s) vs ${shortBase}: ${addedConvertedViolations.map(v => v.pair).join(', ')}`,
      inherited: false,
      tag: tag(false),
    });
  }

  const convertedViolations = result.convertedLegacy.violations.map(v => ({
    ...v,
    inherited: baseConvertedPairs.has(v.pair),
    tag: baseConvertedPairs.has(v.pair) ? `[inherited from ${shortBase}]` : '[new on this branch]',
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
    addedLegacyPairs,
    addedConvertedViolations,
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
    legacyErrors,
    convertedLegacyErrors,
    convertedViolations,
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
  const legErrors = comparison ? comparison.legacyErrors : result.legacy.errors.map(error => ({ error, tag: '' }));
  if (legErrors.length > 0) {
    lines.push(`Legacy view ratchet errors (${legErrors.length}):`);
    for (const entry of legErrors) lines.push(`  - ${entry.error}${entry.tag ? ` ${entry.tag}` : ''}`);
  }
  const convErrors = comparison ? comparison.convertedLegacyErrors : result.convertedLegacy.errors.map(error => ({ error, tag: '' }));
  if (convErrors.length > 0 || (showAllInFile && result.convertedLegacy.violations.length > 0)) {
    lines.push(`Converted views reaching old-renderer modules (${result.convertedLegacy.violations.length}/${result.convertedLegacy.maxLegacyRendererViews}):`);
    for (const entry of convErrors) lines.push(`  - ${entry.error}${entry.tag ? ` ${entry.tag}` : ''}`);
    const list = comparison ? comparison.convertedViolations : result.convertedLegacy.violations;
    for (const entry of list) {
      if (!showAllInFile && comparison && entry.inherited && result.convertedLegacy.errors.length === 0) continue;
      const suffix = entry.tag ? ` ${entry.tag}` : '';
      lines.push(`  - ${entry.pair} (${entry.family}): ${entry.chain.join(' -> ')}${suffix}`);
    }
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
  for (const notice of [...result.legacy.notices, ...result.convertedLegacy.notices, ...result.exports.notices]) {
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
    + `${result.legacy.count}/${result.legacy.maxLegacyViews} legacy view(s), `
    + `${result.convertedLegacy.violations.length}/${result.convertedLegacy.maxLegacyRendererViews} converted view(s) reaching old renderer, `
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
