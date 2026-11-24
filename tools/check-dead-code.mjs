import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, unlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseAst } from 'rolldown/parseAst';
import { astLang, extractImportSpecifiers, relPosix, resolveLocalImport, walkAst, walkImportGraph } from './ui-import-graph.mjs';

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
const SCRIPT_EXTENSIONS = new Set(['.ts', '.tsx', '.mts', '.js', '.mjs']);
const TEXT_EXTENSIONS = new Set(['.ts', '.tsx', '.js', '.mjs', '.cjs', '.rs', '.py', '.ps1', '.json', '.toml', '.html', '.css']);
const NON_CALLER_UI_FILES = new Set([
  'ui/src/dashboard/pages.ts',
  'ui/src/widgets/manifest.ts',
  'ui/src/widgets/legacyManifest.ts',
  'ui/src/widgets/familyManifest.ts',
]);
const GLOBAL_REGISTRIES = new Set([
  'ui/src/widgets/manifest.ts',
  'ui/src/widgets/familyManifest.ts',
  'ui/src/widgets/legacyManifest.ts',
  'ui/src/widgets/rendererLoaders.ts',
  'ui/src/widgets/definitions.ts',
  'ui/src/widgets/index.ts',
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
  const ast = parseAst(source, { lang: astLang(filePath) });

  walkAst(ast, (node, parent, key) => {
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

function parseLegacyManifestEntries(legacyPath, root) {
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
          let loadTarget = null;
          for (const prop of el.properties) {
            if (prop.type !== 'Property') continue;
            const propName = prop.key?.name ?? prop.key?.value;
            if (propName === 'kind' && prop.value?.type === 'Literal' && typeof prop.value.value === 'string') {
              kind = prop.value.value;
            } else if (propName === 'view' && prop.value?.type === 'Literal' && typeof prop.value.value === 'string') {
              view = prop.value.value;
            } else if (propName === 'load') {
              walkAst(prop.value, inner => {
                if (!loadTarget && inner.type === 'ImportExpression' && inner.source?.type === 'Literal' && typeof inner.source.value === 'string') {
                  const target = resolveLocalImport(legacyPath, inner.source.value, root);
                  if (target) loadTarget = relPosix(root, target);
                }
              });
            }
          }
          if (kind) entries.push({ kind, view, loadTarget });
        }
        return entries;
      }
    }
  }
  throw new Error('Could not find exported LEGACY_WIDGETS array in ui/src/widgets/legacyManifest.ts');
}

function parseFamilyViewUiMap(root) {
  const manifestPath = resolve(root, 'ui/src/widgets/manifest.ts');
  const out = new Map();
  if (!existsSync(manifestPath)) return out;
  const ast = parseAst(readFileSync(manifestPath, 'utf8'), { lang: 'ts' });
  for (const node of ast.body) {
    if (node.type !== 'ExportNamedDeclaration' || node.declaration?.type !== 'VariableDeclaration') continue;
    for (const decl of node.declaration.declarations) {
      if (decl.id?.type !== 'Identifier' || decl.id.name !== 'FAMILY_VIEW_UI' || decl.init?.type !== 'ObjectExpression') continue;
      for (const prop of decl.init.properties) {
        if (prop.type !== 'Property') continue;
        const kind = prop.key?.value ?? prop.key?.name;
        if (typeof kind !== 'string') continue;
        const viewsProp = prop.value?.type === 'ObjectExpression'
          ? prop.value.properties.find(item => item.type === 'Property' && (item.key?.name ?? item.key?.value) === 'views')
          : undefined;
        const views = viewsProp?.value?.type === 'ArrayExpression'
          ? viewsProp.value.elements.filter(item => item?.type === 'Literal' && typeof item.value === 'string').map(item => item.value)
          : null;
        walkAst(prop.value, inner => {
          if (!out.has(kind) && inner.type === 'ImportExpression' && inner.source?.type === 'Literal' && typeof inner.source.value === 'string') {
            const target = resolveLocalImport(manifestPath, inner.source.value, root);
            if (target) out.set(kind, { target: relPosix(root, target), views });
          }
        });
      }
    }
  }
  return out;
}

function parseFamilyManifestSpecs(root, runtimeFamilyViews) {
  const dir = resolve(root, 'ui/src/widgets/manifest');
  if (!existsSync(dir)) return [];
  const files = readdirSync(dir).filter(name => name.endsWith('.ts') && !name.endsWith('.test.ts')).sort();
  const runtimeViewsByKindFamily = new Map();
  for (const entry of runtimeFamilyViews) {
    const view = typeof entry.view === 'string' && entry.view.length > 0 ? entry.view : 'default';
    const key = `${entry.kind}\0${entry.family}`;
    let list = runtimeViewsByKindFamily.get(key);
    if (!list) runtimeViewsByKindFamily.set(key, list = []);
    list.push(view);
  }

  const out = [];
  const seen = new Set();
  const pushEntry = (kind, view, family, specTarget) => {
    const key = `${kind}:${view}`;
    if (seen.has(key)) return;
    seen.add(key);
    out.push({ kind, view, family, specTarget });
  };

  for (const file of files) {
    const full = resolve(dir, file);
    const family = file.slice(0, -'.ts'.length);
    const ast = parseAst(readFileSync(full, 'utf8'), { lang: 'ts' });
    const firstImportIn = node => {
      let target = null;
      walkAst(node, inner => {
        if (!target && inner.type === 'ImportExpression' && inner.source?.type === 'Literal' && typeof inner.source.value === 'string') {
          const resolved = resolveLocalImport(full, inner.source.value, root);
          if (resolved) target = relPosix(root, resolved);
        }
      });
      return target;
    };
    const record = (kind, view, specTarget) => {
      if (!kind || !specTarget) return;
      if (view === '*') {
        const concrete = runtimeViewsByKindFamily.get(`${kind}\0${family}`) ?? ['*'];
        for (const v of concrete) pushEntry(kind, v, family, specTarget);
      } else {
        pushEntry(kind, view, family, specTarget);
      }
    };
    const visitEntryNode = node => {
      if (node.type === 'ObjectExpression') {
        let kind = null;
        let view = 'default';
        let specTarget = null;
        for (const prop of node.properties) {
          if (prop.type !== 'Property') continue;
          const propName = prop.key?.name ?? prop.key?.value;
          if (propName === 'kind' && prop.value?.type === 'Literal' && typeof prop.value.value === 'string') {
            kind = prop.value.value;
          } else if (propName === 'view') {
            if (prop.value?.type === 'Literal' && typeof prop.value.value === 'string') view = prop.value.value;
            else if (prop.value?.type === 'Identifier' && prop.value.name === 'DEFAULT_VIEW') view = 'default';
            else view = '*';
          } else if (propName === 'loadSpec') {
            specTarget = firstImportIn(prop.value);
          }
        }
        record(kind, view, specTarget);
      } else if (node.type === 'CallExpression' && node.arguments.length >= 2 && node.arguments[0]?.type === 'Literal' && typeof node.arguments[0].value === 'string') {
        const specTarget = firstImportIn(node);
        let view = 'default';
        if (node.arguments[1]?.type === 'Literal' && typeof node.arguments[1].value === 'string') {
          view = node.arguments[1].value;
        }
        record(node.arguments[0].value, view, specTarget);
      }
    };

    for (const stmt of ast.body) {
      if (stmt.type !== 'ExportNamedDeclaration' || stmt.declaration?.type !== 'VariableDeclaration') continue;
      for (const decl of stmt.declaration.declarations) {
        if (decl.init?.type !== 'ArrayExpression') continue;
        for (const el of decl.init.elements) {
          if (!el) continue;
          if (el.type === 'SpreadElement') {
            walkAst(el.argument, inner => {
              if (inner.type === 'ObjectExpression') visitEntryNode(inner);
            });
          } else {
            visitEntryNode(el);
          }
        }
      }
    }
  }
  return out.sort((a, b) => `${a.kind}:${a.view}`.localeCompare(`${b.kind}:${b.view}`));
}

async function loadMetaAndFamilyViews(root) {
  const metaByKind = new Map();
  const familyViews = [];
  const familyByKind = new Map();
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
  return { metaByKind, familyViews, familyByKind };
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

function readRatchetFile(root) {
  const ratchetPath = resolve(root, 'tools/legacy-widgets-ratchet.json');
  return JSON.parse(readFileSync(ratchetPath, 'utf8'));
}

export async function checkLegacyRatchet(root, loaded = null, ratchetData = null) {
  const data = ratchetData ?? readRatchetFile(root);
  const maxLegacyViews = data.maxLegacyViews ?? data.maxLegacyWidgets;
  if (typeof maxLegacyViews !== 'number' || !Number.isInteger(maxLegacyViews) || maxLegacyViews < 0) {
    throw new Error('tools/legacy-widgets-ratchet.json must set integer maxLegacyViews >= 0');
  }

  const legacyPath = resolve(root, 'ui/src/widgets/legacyManifest.ts');
  const legacyFileExists = existsSync(legacyPath);
  const legacyEntries = legacyFileExists ? parseLegacyManifestEntries(legacyPath, root) : [];
  const resolved = loaded ?? (legacyEntries.length > 0 ? await loadMetaAndFamilyViews(root) : { metaByKind: new Map(), familyViews: [], familyByKind: new Map() });
  const pairs = expandLegacyViewPairs(legacyEntries, resolved.metaByKind, resolved.familyByKind);
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

  return { count, maxLegacyViews, maxLegacyWidgets: maxLegacyViews, pairs, legacyEntries, errors, notices };
}

export function checkConvertedLegacyReach(root, uiResult, legacyResult, runtimeFamilyViews = [], ratchetData = null) {
  const data = ratchetData ?? readRatchetFile(root);
  const maxLegacyRendererViews = typeof data.maxLegacyRendererViews === 'number' ? data.maxLegacyRendererViews : null;
  if (maxLegacyRendererViews !== null && (!Number.isInteger(maxLegacyRendererViews) || maxLegacyRendererViews < 0)) {
    throw new Error('tools/legacy-widgets-ratchet.json must set integer maxLegacyRendererViews >= 0');
  }

  const familyViews = parseFamilyManifestSpecs(root, runtimeFamilyViews);
  const familyViewUi = parseFamilyViewUiMap(root);

  const familyFiles = [...uiResult.liveProd].filter(file => file.startsWith('ui/src/families/'));
  const familyGraph = walkImportGraph(familyFiles, root, {
    includeTypes: false,
    includeDynamic: true,
    includeWorkerUrls: true,
    shouldTraverse: file => {
      const rel = relPosix(root, file);
      return !rel.startsWith('ui/src/widgets/') && !GLOBAL_REGISTRIES.has(rel);
    },
  });
  const familyReach = new Set([...familyGraph.visited].map(file => relPosix(root, file)));

  const legacyStarts = [...new Set(legacyResult.legacyEntries.map(entry => entry.loadTarget).filter(Boolean))];
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

  const oldCharts = new Set([...uiResult.liveProd].filter(file => file.startsWith('ui/src/charts/') && !familyReach.has(file)));
  const oldDash = new Set();
  const dashCache = new Map();
  for (const file of uiResult.liveProd) {
    if (!file.startsWith('ui/src/dashboard/') || !legacyReach.has(file) || familyReach.has(file)) continue;
    const g = walkImportGraph([file], root, {
      includeTypes: false,
      includeDynamic: true,
      includeWorkerUrls: true,
      cache: dashCache,
      shouldTraverse: cur => !GLOBAL_REGISTRIES.has(relPosix(root, cur)),
    });
    let isUiOrHook = file.endsWith('.tsx') || file.endsWith('.css');
    for (const visited of g.visited) {
      const rel = relPosix(root, visited);
      if (oldCharts.has(rel) || rel.endsWith('.tsx') || rel.endsWith('.css')) isUiOrHook = true;
      const ext = g.externalImports.get(visited);
      if (ext && [...ext].some(spec => spec === 'react' || spec.startsWith('react/'))) isUiOrHook = true;
    }
    if (isUiOrHook) oldDash.add(file);
  }
  const oldRenderer = new Set([...oldCharts, ...oldDash]);

  const edgeCache = new Map();
  const shortestChain = startFiles => {
    const queue = startFiles.map(file => [file]);
    const seen = new Set(startFiles);
    while (queue.length > 0) {
      const chain = queue.shift();
      const cur = chain[chain.length - 1];
      if (oldRenderer.has(cur)) return chain;
      if (cur.startsWith('ui/src/families/') || GLOBAL_REGISTRIES.has(cur)) continue;
      const full = resolve(root, cur);
      let targets = edgeCache.get(full);
      if (!targets) {
        targets = [...new Set(extractImportSpecifiers(full, { includeTypes: false, includeDynamic: true, includeWorkerUrls: true })
          .map(spec => resolveLocalImport(full, spec.specifier, root))
          .filter(Boolean)
          .map(target => relPosix(root, target)))].sort();
        edgeCache.set(full, targets);
      }
      for (const next of targets) {
        if (seen.has(next)) continue;
        seen.add(next);
        queue.push([...chain, next]);
      }
    }
    return null;
  };

  const violations = [];
  for (const entry of familyViews) {
    const starts = [entry.specTarget];
    const ui = familyViewUi.get(entry.kind);
    const uiEntry = ui && (!ui.views || ui.views.includes(entry.view)) ? ui.target : undefined;
    if (uiEntry && !starts.includes(uiEntry)) starts.push(uiEntry);
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
  const limit = maxLegacyRendererViews ?? 0;
  if (maxLegacyRendererViews !== null) {
    if (violations.length > limit) {
      errors.push(`${violations.length} converted view(s) reach old-renderer modules, above ratchet ${limit}`);
    } else if (violations.length > 0 && violations.length < limit) {
      notices.push(`converted views reaching old-renderer modules dropped to ${violations.length} (ratchet is ${limit}); lower maxLegacyRendererViews in tools/legacy-widgets-ratchet.json to ${violations.length}`);
    } else if (violations.length === 0 && limit !== 0) {
      errors.push(`no converted view reaches old-renderer modules; lower maxLegacyRendererViews in tools/legacy-widgets-ratchet.json from ${limit} to 0`);
    }
  }

  return {
    totalConvertedViews: familyViews.length,
    oldChartCount: oldCharts.size,
    oldDashboardCount: oldDash.size,
    maxLegacyRendererViews,
    violations,
    errors,
    notices,
  };
}

function collectBindingNames(pattern, out) {
  if (!pattern) return;
  if (pattern.type === 'Identifier') {
    out.push(pattern.name);
    return;
  }
  if (pattern.type === 'ObjectPattern') {
    for (const prop of pattern.properties) {
      if (prop.type === 'RestElement') collectBindingNames(prop.argument, out);
      else collectBindingNames(prop.value, out);
    }
  } else if (pattern.type === 'ArrayPattern') {
    for (const el of pattern.elements) collectBindingNames(el, out);
  } else if (pattern.type === 'AssignmentPattern') {
    collectBindingNames(pattern.left, out);
  } else if (pattern.type === 'RestElement') {
    collectBindingNames(pattern.argument, out);
  }
}

function leftmostIdent(node) {
  let cur = node;
  while (cur && cur.type === 'TSQualifiedName') cur = cur.left;
  return cur?.type === 'Identifier' ? cur.name : null;
}

function collectParamMemberUsage(fnNode, paramName) {
  const used = new Set();
  let opaque = false;
  walkAst(fnNode.body, (node, parent, key) => {
    if (node.type !== 'Identifier' || node.name !== paramName) return;
    if (parent?.type === 'MemberExpression' && key === 'object') {
      if (!parent.computed && parent.property?.type === 'Identifier') used.add(parent.property.name);
      else opaque = true;
      return;
    }
    if (parent?.type === 'VariableDeclarator' && key === 'init' && parent.id?.type === 'ObjectPattern') {
      for (const prop of parent.id.properties) {
        if (prop.type === 'Property' && !prop.computed && prop.key?.type === 'Identifier') used.add(prop.key.name);
        else opaque = true;
      }
      return;
    }
    if (parent?.type === 'Property' && key === 'key' && !parent.computed) return;
    opaque = true;
  });
  return { used, opaque };
}

function collectStatementRefs(stmtNode) {
  const refs = new Set();
  walkAst(stmtNode, (node, parent, key) => {
    if (node.type !== 'Identifier' && node.type !== 'JSXIdentifier') return;
    if (parent?.type === 'MemberExpression' && key === 'property' && !parent.computed) return;
    if (parent?.type === 'Property' && key === 'key' && !parent.computed && !parent.shorthand) return;
    if (parent?.type === 'TSPropertySignature' && key === 'key' && !parent.computed) return;
    if (parent?.type === 'MethodDefinition' && key === 'key' && !parent.computed) return;
    if (parent?.type === 'PropertyDefinition' && key === 'key' && !parent.computed) return;
    if (parent?.type === 'JSXAttribute' && key === 'name') return;
    if (parent?.type === 'ImportSpecifier' || parent?.type === 'ImportDefaultSpecifier' || parent?.type === 'ImportNamespaceSpecifier') return;
    if (parent?.type === 'ExportSpecifier' && key === 'exported') return;
    if ((parent?.type === 'FunctionDeclaration' || parent?.type === 'ClassDeclaration'
      || parent?.type === 'TSTypeAliasDeclaration' || parent?.type === 'TSInterfaceDeclaration'
      || parent?.type === 'TSEnumDeclaration') && key === 'id') return;
    if (parent?.type === 'VariableDeclarator' && key === 'id') return;
    refs.add(node.name);
  });
  return refs;
}

function analyzeScriptExportsAndImports(root, relPath) {
  const full = resolve(root, relPath);
  const ext = extname(full).toLowerCase();
  if (!SCRIPT_EXTENSIONS.has(ext)) return null;
  const source = readFileSync(full, 'utf8');
  const ast = parseAst(source, { lang: astLang(full) });

  const declaredExports = new Map();
  const reexports = [];
  const starReexports = [];
  const directImports = new Map();
  const nsLocals = new Map();
  const statements = [];

  const addDirect = (targetRel, name) => {
    if (!name) return;
    let set = directImports.get(targetRel);
    if (!set) directImports.set(targetRel, set = new Set());
    set.add(name);
  };

  for (let stmtIndex = 0; stmtIndex < ast.body.length; stmtIndex += 1) {
    const node = ast.body[stmtIndex];
    const bindings = [];
    let isDeclOnly = false;

    if (node.type === 'ImportDeclaration') {
      isDeclOnly = true;
      if (typeof node.source?.value === 'string') {
        const target = resolveLocalImport(full, node.source.value, root);
        if (target) {
          const targetRel = relPosix(root, target);
          for (const spec of node.specifiers) {
            if (spec.type === 'ImportDefaultSpecifier') addDirect(targetRel, 'default');
            else if (spec.type === 'ImportSpecifier') addDirect(targetRel, spec.imported?.name ?? spec.imported?.value);
            else if (spec.type === 'ImportNamespaceSpecifier') nsLocals.set(spec.local.name, targetRel);
          }
        }
      }
    } else if (node.type === 'ExportDefaultDeclaration') {
      isDeclOnly = true;
      if (node.declaration?.id?.name) bindings.push(node.declaration.id.name);
      declaredExports.set('default', { stmtIndex, localName: node.declaration?.id?.name ?? null, isType: false, isReexport: false });
    } else if (node.type === 'ExportNamedDeclaration') {
      isDeclOnly = true;
      if (node.source && typeof node.source.value === 'string') {
        const target = resolveLocalImport(full, node.source.value, root);
        const targetRel = target ? relPosix(root, target) : null;
        for (const spec of node.specifiers) {
          const exported = spec.exported?.name ?? spec.exported?.value;
          const local = spec.local?.name ?? spec.local?.value;
          if (!exported) continue;
          declaredExports.set(exported, {
            stmtIndex,
            localName: null,
            isType: node.exportKind === 'type' || spec.exportKind === 'type',
            isReexport: true,
          });
          if (targetRel && local) reexports.push({ exported, targetRel, targetName: local });
        }
      } else if (node.declaration) {
        const decl = node.declaration;
        const isType = decl.type === 'TSTypeAliasDeclaration' || decl.type === 'TSInterfaceDeclaration';
        if (decl.type === 'VariableDeclaration') {
          const names = [];
          for (const item of decl.declarations) collectBindingNames(item.id, names);
          for (const name of names) {
            bindings.push(name);
            declaredExports.set(name, { stmtIndex, localName: name, isType: false, isReexport: false });
          }
        } else if (decl.id?.name) {
          bindings.push(decl.id.name);
          declaredExports.set(decl.id.name, { stmtIndex, localName: decl.id.name, isType, isReexport: false });
        }
      } else {
        for (const spec of node.specifiers) {
          const exported = spec.exported?.name ?? spec.exported?.value;
          const local = spec.local?.name ?? spec.local?.value;
          if (!exported) continue;
          declaredExports.set(exported, {
            stmtIndex,
            localName: local,
            isType: node.exportKind === 'type' || spec.exportKind === 'type',
            isReexport: false,
            specifierOnly: true,
          });
        }
      }
    } else if (node.type === 'ExportAllDeclaration') {
      isDeclOnly = true;
      if (typeof node.source?.value === 'string') {
        const target = resolveLocalImport(full, node.source.value, root);
        const targetRel = target ? relPosix(root, target) : null;
        if (node.exported) {
          const expName = node.exported.name ?? node.exported.value;
          if (expName) {
            declaredExports.set(expName, { stmtIndex, localName: null, isType: false, isReexport: true });
            if (targetRel) reexports.push({ exported: expName, targetRel, targetName: '*' });
          }
        } else if (targetRel) {
          starReexports.push(targetRel);
        }
      }
    } else if (node.type === 'VariableDeclaration') {
      isDeclOnly = true;
      for (const item of node.declarations) collectBindingNames(item.id, bindings);
    } else if (node.type === 'FunctionDeclaration' || node.type === 'ClassDeclaration'
      || node.type === 'TSTypeAliasDeclaration' || node.type === 'TSInterfaceDeclaration'
      || node.type === 'TSEnumDeclaration') {
      isDeclOnly = true;
      if (node.id?.name) bindings.push(node.id.name);
    }

    const refs = node.type === 'ImportDeclaration' || (node.type === 'ExportNamedDeclaration' && node.source) || node.type === 'ExportAllDeclaration'
      ? new Set()
      : collectStatementRefs(node);
    statements.push({ stmtIndex, bindings, refs, isSideEffect: !isDeclOnly });
  }

  walkAst(ast, (node, parent, key) => {
    if (nsLocals.size > 0 && node.type === 'Identifier' && nsLocals.has(node.name)) {
      if (parent?.type === 'ImportNamespaceSpecifier') return;
      const targetRel = nsLocals.get(node.name);
      if (parent?.type === 'MemberExpression' && key === 'object') {
        if (!parent.computed && parent.property?.type === 'Identifier') addDirect(targetRel, parent.property.name);
        else addDirect(targetRel, '*');
        return;
      }
      if (parent?.type === 'TSQualifiedName' && key === 'left') {
        if (parent.right?.type === 'Identifier') addDirect(targetRel, parent.right.name);
        else addDirect(targetRel, '*');
        return;
      }
      if (parent?.type === 'Property' && key === 'key' && !parent.computed) return;
      if (parent?.type === 'MemberExpression' && key === 'property' && !parent.computed) return;
      addDirect(targetRel, '*');
    }
    if (node.type === 'TSImportType') {
      const spec = node.source?.value ?? node.argument?.literal?.value;
      if (typeof spec === 'string') {
        const target = resolveLocalImport(full, spec, root);
        if (target) {
          const targetRel = relPosix(root, target);
          addDirect(targetRel, leftmostIdent(node.qualifier) ?? '*');
        }
      }
    }
    if (node.type === 'ImportExpression' && node.source?.type === 'Literal' && typeof node.source.value === 'string') {
      const target = resolveLocalImport(full, node.source.value, root);
      if (!target) return;
      const targetRel = relPosix(root, target);
      if (parent?.type === 'MemberExpression' && key === 'object' && parent.property?.name === 'then') return;
      if (parent?.type === 'AwaitExpression') return;
      addDirect(targetRel, 'default');
    }
    if (node.type === 'AwaitExpression' && node.argument?.type === 'ImportExpression'
      && node.argument.source?.type === 'Literal' && typeof node.argument.source.value === 'string') {
      const target = resolveLocalImport(full, node.argument.source.value, root);
      if (!target) return;
      const targetRel = relPosix(root, target);
      if (parent?.type === 'MemberExpression' && key === 'object') {
        if (!parent.computed && parent.property?.type === 'Identifier') addDirect(targetRel, parent.property.name);
        else addDirect(targetRel, '*');
      } else if (parent?.type === 'VariableDeclarator' && key === 'init' && parent.id?.type === 'ObjectPattern') {
        for (const prop of parent.id.properties) {
          if (prop.type === 'Property' && !prop.computed && prop.key?.type === 'Identifier') addDirect(targetRel, prop.key.name);
          else addDirect(targetRel, '*');
        }
      } else {
        addDirect(targetRel, '*');
      }
    }
    if (node.type === 'CallExpression' && node.callee?.type === 'MemberExpression'
      && !node.callee.computed && node.callee.property?.name === 'then'
      && node.callee.object?.type === 'ImportExpression' && node.callee.object.source?.type === 'Literal') {
      const target = resolveLocalImport(full, node.callee.object.source.value, root);
      if (!target) return;
      const targetRel = relPosix(root, target);
      const cb = node.arguments[0];
      if (cb && (cb.type === 'ArrowFunctionExpression' || cb.type === 'FunctionExpression')) {
        const p0 = cb.params[0];
        if (p0?.type === 'Identifier') {
          const { used, opaque } = collectParamMemberUsage(cb, p0.name);
          if (opaque) addDirect(targetRel, '*');
          else for (const name of used) addDirect(targetRel, name);
        } else if (p0?.type === 'ObjectPattern') {
          for (const prop of p0.properties) {
            if (prop.type === 'Property' && !prop.computed && prop.key?.type === 'Identifier') addDirect(targetRel, prop.key.name);
            else addDirect(targetRel, '*');
          }
        } else {
          addDirect(targetRel, '*');
        }
      } else {
        addDirect(targetRel, '*');
      }
    }
    if (node.type === 'CallExpression' && node.arguments.length >= 2) {
      for (let i = 0; i < node.arguments.length - 1; i += 1) {
        const arg = node.arguments[i];
        let importTarget = null;
        if (arg?.type === 'ArrowFunctionExpression' && arg.body?.type === 'ImportExpression' && arg.body.source?.type === 'Literal') {
          importTarget = resolveLocalImport(full, arg.body.source.value, root);
        }
        if (!importTarget) continue;
        const nextArg = node.arguments[i + 1];
        if (nextArg && (nextArg.type === 'ArrowFunctionExpression' || nextArg.type === 'FunctionExpression')) {
          const targetRel = relPosix(root, importTarget);
          const p0 = nextArg.params[0];
          if (p0?.type === 'Identifier') {
            const { used, opaque } = collectParamMemberUsage(nextArg, p0.name);
            if (opaque) addDirect(targetRel, '*');
            else for (const name of used) addDirect(targetRel, name);
          } else {
            addDirect(targetRel, '*');
          }
        }
      }
    }
  });

  return { relPath, declaredExports, reexports, starReexports, directImports, statements };
}

export function checkUnusedExports(root, uiResult, ratchetData = null) {
  const data = ratchetData ?? readRatchetFile(root);
  const maxUnusedExports = typeof data.maxUnusedExports === 'number' ? data.maxUnusedExports : null;
  if (maxUnusedExports !== null && (!Number.isInteger(maxUnusedExports) || maxUnusedExports < 0)) {
    throw new Error('tools/legacy-widgets-ratchet.json must set integer maxUnusedExports >= 0');
  }

  const infoByFile = new Map();
  for (const rel of uiResult.liveProd) {
    const info = analyzeScriptExportsAndImports(root, rel);
    if (info) infoByFile.set(rel, info);
  }

  const entryModules = new Set();
  for (const htmlRel of uiResult.entryPoints) {
    const full = resolve(root, htmlRel);
    for (const spec of extractImportSpecifiers(full)) {
      const target = resolveLocalImport(full, spec.specifier, root);
      if (target) entryModules.add(relPosix(root, target));
    }
  }
  for (const rel of uiResult.liveProd) {
    const full = resolve(root, rel);
    for (const spec of extractImportSpecifiers(full, { includeTypes: false, includeDynamic: false, includeWorkerUrls: true })) {
      if (spec.kind === 'url') {
        const target = resolveLocalImport(full, spec.specifier, root);
        if (target) entryModules.add(relPosix(root, target));
      }
    }
  }

  const demanded = new Map();
  const markDemand = (fileRel, name) => {
    let set = demanded.get(fileRel);
    if (!set) demanded.set(fileRel, set = new Set());
    if (set.has(name)) return false;
    set.add(name);
    return true;
  };
  for (const info of infoByFile.values()) {
    for (const [targetRel, names] of info.directImports) {
      for (const name of names) markDemand(targetRel, name);
    }
  }

  let changed = true;
  while (changed) {
    changed = false;
    for (const [rel, info] of infoByFile) {
      const req = demanded.get(rel);
      if (!req) continue;
      const allWanted = req.has('*');
      for (const re of info.reexports) {
        if (allWanted || req.has(re.exported)) {
          if (markDemand(re.targetRel, re.targetName)) changed = true;
        }
      }
      for (const starTarget of info.starReexports) {
        if (allWanted) {
          if (markDemand(starTarget, '*')) changed = true;
        } else {
          for (const name of req) {
            if (name !== 'default' && !info.declaredExports.has(name)) {
              if (markDemand(starTarget, name)) changed = true;
            }
          }
        }
      }
    }
  }

  const unusedExports = [];
  for (const [rel, info] of [...infoByFile.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
    if (rel.startsWith('ui/src/generated/') || entryModules.has(rel)) continue;
    const req = demanded.get(rel);
    if (req?.has('*')) continue;

    const bindingToStmts = new Map();
    for (const stmt of info.statements) {
      for (const name of stmt.bindings) {
        let list = bindingToStmts.get(name);
        if (!list) bindingToStmts.set(name, list = []);
        list.push(stmt.stmtIndex);
      }
    }

    const liveStmts = new Set();
    const queue = [];
    const markStmt = idx => {
      if (!liveStmts.has(idx)) {
        liveStmts.add(idx);
        queue.push(idx);
      }
    };

    for (const stmt of info.statements) {
      if (stmt.isSideEffect) markStmt(stmt.stmtIndex);
    }
    for (const [expName, meta] of info.declaredExports) {
      if (!req?.has(expName)) continue;
      markStmt(meta.stmtIndex);
      if (meta.specifierOnly && meta.localName) {
        for (const idx of bindingToStmts.get(meta.localName) ?? []) markStmt(idx);
      }
    }

    while (queue.length > 0) {
      const idx = queue.pop();
      const stmt = info.statements[idx];
      if (!stmt) continue;
      for (const ref of stmt.refs) {
        for (const depIdx of bindingToStmts.get(ref) ?? []) markStmt(depIdx);
      }
    }

    for (const [expName, meta] of [...info.declaredExports.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
      if (req?.has(expName)) continue;
      if (meta.isReexport) {
        unusedExports.push({ file: rel, name: expName, key: `${rel}:${expName}`, isType: meta.isType, isReexport: true });
        continue;
      }
      if (meta.specifierOnly) {
        const localLive = Boolean(meta.localName && (bindingToStmts.get(meta.localName) ?? []).some(idx => liveStmts.has(idx)));
        if (!localLive) {
          unusedExports.push({ file: rel, name: expName, key: `${rel}:${expName}`, isType: meta.isType, isReexport: false });
        }
        continue;
      }
      if (!liveStmts.has(meta.stmtIndex)) {
        unusedExports.push({ file: rel, name: expName, key: `${rel}:${expName}`, isType: meta.isType, isReexport: false });
      }
    }
  }

  const errors = [];
  const notices = [];
  if (maxUnusedExports !== null) {
    if (unusedExports.length > maxUnusedExports) {
      errors.push(`${unusedExports.length} unused export(s) in ui/src, above ratchet ${maxUnusedExports}`);
    } else if (unusedExports.length > 0 && unusedExports.length < maxUnusedExports) {
      notices.push(`unused exports in ui/src dropped to ${unusedExports.length} (ratchet is ${maxUnusedExports}); lower maxUnusedExports in tools/legacy-widgets-ratchet.json to ${unusedExports.length}`);
    } else if (unusedExports.length === 0 && maxUnusedExports !== 0) {
      errors.push(`no unused exports remain in ui/src; lower maxUnusedExports in tools/legacy-widgets-ratchet.json from ${maxUnusedExports} to 0`);
    }
  }

  return {
    maxUnusedExports,
    unusedExports,
    errors,
    notices,
  };
}

export async function runDeadCodeCheck(root) {
  const ui = checkUiModules(root);
  const rust = checkRustProducts(root, ui);
  const ratchetData = readRatchetFile(root);
  const loaded = await loadMetaAndFamilyViews(root);
  const legacy = await checkLegacyRatchet(root, loaded, ratchetData);
  const convertedLegacy = checkConvertedLegacyReach(root, ui, legacy, loaded.familyViews, ratchetData);
  const exports = checkUnusedExports(root, ui, ratchetData);
  const totalIssues = ui.deadProduction.length
    + ui.deadTests.length
    + ui.deadFixturesAndSupport.length
    + rust.deadProducts.length
    + rust.unwiredRegistrations.length
    + legacy.errors.length
    + convertedLegacy.errors.length
    + exports.errors.length;
  return { ui, rust, legacy, convertedLegacy, exports, totalIssues };
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

  const baseConvertedPairs = new Set((baseResult.convertedLegacy?.violations ?? []).map(v => v.pair));
  const baseConvertedOver = baseResult.convertedLegacy?.maxLegacyRendererViews !== null
    && (baseResult.convertedLegacy?.violations?.length ?? 0) > baseResult.convertedLegacy.maxLegacyRendererViews;

  const baseUnusedExportKeys = new Set((baseResult.exports?.unusedExports ?? []).map(item => item.key));
  const baseExportsOver = baseResult.exports?.maxUnusedExports !== null
    && (baseResult.exports?.unusedExports?.length ?? 0) > baseResult.exports.maxUnusedExports;

  const addedLegacyPairs = result.legacy.pairs.filter(pair => !baseLegacyPairs.has(pair));
  const addedConvertedViolations = result.convertedLegacy.violations.filter(v => !baseConvertedPairs.has(v.pair));
  const addedUnusedExports = result.exports.unusedExports.filter(item => !baseUnusedExportKeys.has(item.key));

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

  const convertedLegacyErrors = result.convertedLegacy.errors.map(err => {
    if (err.includes('above ratchet')) {
      const addedText = addedConvertedViolations.length > 0
        ? ` (added: ${addedConvertedViolations.map(v => v.pair).join(', ')})`
        : '';
      const inherited = baseConvertedOver
        && addedConvertedViolations.length === 0
        && result.convertedLegacy.violations.length <= (baseResult.convertedLegacy?.violations?.length ?? 0);
      return { error: `${err}${addedText}`, inherited, tag: tag(inherited) };
    }
    const inherited = (baseResult.convertedLegacy?.errors ?? []).includes(err);
    return { error: err, inherited, tag: tag(inherited) };
  });

  const convertedViolations = result.convertedLegacy.violations.map(v => ({
    ...v,
    inherited: baseConvertedPairs.has(v.pair),
    tag: baseConvertedPairs.has(v.pair) ? `[inherited from ${shortBase}]` : '[new on this branch]',
  }));

  const unusedExportErrors = result.exports.errors.map(err => {
    if (err.includes('above ratchet')) {
      const addedText = addedUnusedExports.length > 0
        ? ` (added: ${addedUnusedExports.slice(0, 10).map(item => item.key).join(', ')}${addedUnusedExports.length > 10 ? `, +${addedUnusedExports.length - 10} more` : ''})`
        : '';
      const inherited = baseExportsOver
        && addedUnusedExports.length === 0
        && result.exports.unusedExports.length <= (baseResult.exports?.unusedExports?.length ?? 0);
      return { error: `${err}${addedText}`, inherited, tag: tag(inherited) };
    }
    const inherited = (baseResult.exports?.errors ?? []).includes(err);
    return { error: err, inherited, tag: tag(inherited) };
  });

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
    newIssues,
    inheritedIssues,
    deadProduction,
    deadTests,
    deadFixturesAndSupport,
    deadProducts,
    unwiredRegistrations,
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
  if (result.convertedLegacy.errors.length > 0 || (showAllInFile && result.convertedLegacy.violations.length > 0)) {
    lines.push(`Converted views reaching old-renderer modules (${result.convertedLegacy.violations.length}/${result.convertedLegacy.maxLegacyRendererViews ?? 0}):`);
    if (comparison && result.convertedLegacy.errors.length > 0) {
      for (const entry of comparison.convertedLegacyErrors) lines.push(`  - ${entry.error} ${entry.tag}`);
    } else {
      for (const err of result.convertedLegacy.errors) lines.push(`  - ${err}`);
    }
    const list = comparison ? comparison.convertedViolations : result.convertedLegacy.violations;
    for (const entry of list) {
      const suffix = entry.tag ? ` ${entry.tag}` : '';
      lines.push(`  - ${entry.pair} (${entry.family}): ${entry.chain.join(' -> ')}${suffix}`);
    }
  }
  if (result.exports.errors.length > 0 || (showAllInFile && result.exports.unusedExports.length > 0)) {
    lines.push(`Unused exports in ui/src (${result.exports.unusedExports.length}/${result.exports.maxUnusedExports ?? 0}):`);
    if (comparison && result.exports.errors.length > 0) {
      for (const entry of comparison.unusedExportErrors) lines.push(`  - ${entry.error} ${entry.tag}`);
    } else {
      for (const err of result.exports.errors) lines.push(`  - ${err}`);
    }
    const list = comparison ? comparison.unusedExports : result.exports.unusedExports;
    for (const entry of list) {
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
    execFileSync('tar', ['-xf', '__base.tar'], { cwd: baseDir, stdio: ['ignore', 'ignore', 'pipe'] });
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
  const listInFile = process.argv.includes('--list-in-file');
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
  const body = formatReport(result, comparison, { showAllInFile: listInFile });
  if (body) console.log(body);
  const legacyRendererPart = result.convertedLegacy.maxLegacyRendererViews !== null
    ? `, ${result.convertedLegacy.violations.length}/${result.convertedLegacy.maxLegacyRendererViews} converted view(s) reaching old renderer`
    : '';
  const unusedExportPart = result.exports.maxUnusedExports !== null
    ? `, ${result.exports.unusedExports.length}/${result.exports.maxUnusedExports} unused export(s)`
    : '';
  const summary = `dead-code: ${result.ui.deadProduction.length} dead UI module(s), `
    + `${result.ui.deadTests.length} dead test(s), `
    + `${result.ui.deadFixturesAndSupport.length} dead fixture/support file(s), `
    + `${result.rust.deadProducts.length} dead Rust product(s), `
    + `${result.rust.unwiredRegistrations.length} unwired registration(s), `
    + `${result.legacy.count}/${result.legacy.maxLegacyViews} legacy view(s)`
    + legacyRendererPart
    + unusedExportPart;
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
