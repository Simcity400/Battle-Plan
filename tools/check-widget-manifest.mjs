import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseAst } from 'rolldown/parseAst';
import { DEFAULT_VIEW, normalizeView, viewKey } from '../ui/src/families/manifest.ts';
import { widgetMetas } from '../ui/src/widgets/definitions.ts';
import { FAMILY_VIEWS } from '../ui/src/widgets/familyManifest.ts';
import { LEGACY_WIDGETS } from '../ui/src/widgets/legacyManifest.ts';

const root = fileURLToPath(new URL('../', import.meta.url));
const errors = [];

const metaByKind = new Map();
for (const meta of widgetMetas) {
  if (metaByKind.has(meta.kind)) errors.push(`duplicate WidgetMeta kind: ${meta.kind}`);
  metaByKind.set(meta.kind, meta);
}

const familyByKind = new Map();
const familyPairs = new Set();
for (const entry of FAMILY_VIEWS) {
  if (!entry.kind) {
    errors.push('FAMILY_VIEWS entry missing kind');
    continue;
  }
  if (!metaByKind.has(entry.kind)) errors.push(`FAMILY_VIEWS references unknown kind: ${entry.kind}`);
  if (!entry.family || typeof entry.family !== 'string') errors.push(`FAMILY_VIEWS(${entry.kind}) missing family id`);
  if (typeof entry.loadSpec !== 'function') errors.push(`FAMILY_VIEWS(${entry.kind}) missing loadSpec function`);
  const view = normalizeView(entry.view);
  const key = viewKey(entry.kind, view);
  if (familyPairs.has(key)) errors.push(`duplicate FAMILY_VIEWS entry: (${entry.kind}, ${view})`);
  familyPairs.add(key);
  let list = familyByKind.get(entry.kind);
  if (!list) familyByKind.set(entry.kind, list = []);
  list.push(entry);
  if (entry.options) {
    const optionKeys = new Set();
    for (const field of entry.options) {
      if (optionKeys.has(field.key)) errors.push(`duplicate option key ${field.key} in FAMILY_VIEWS(${entry.kind}, ${view})`);
      optionKeys.add(field.key);
    }
  }
}

const legacyByKind = new Map();
const legacyPairs = new Set();
for (const entry of LEGACY_WIDGETS) {
  if (!entry.kind) {
    errors.push('LEGACY_WIDGETS entry missing kind');
    continue;
  }
  if (!metaByKind.has(entry.kind)) errors.push(`LEGACY_WIDGETS references unknown kind: ${entry.kind}`);
  if (typeof entry.load !== 'function') errors.push(`LEGACY_WIDGETS(${entry.kind}) missing load function`);
  const view = normalizeView(entry.view);
  const key = viewKey(entry.kind, view);
  if (legacyPairs.has(key)) errors.push(`duplicate LEGACY_WIDGETS entry: (${entry.kind}, ${view})`);
  if (familyPairs.has(key)) errors.push(`(${entry.kind}, ${view}) is registered in both FAMILY_VIEWS and LEGACY_WIDGETS`);
  legacyPairs.add(key);
  let list = legacyByKind.get(entry.kind);
  if (!list) legacyByKind.set(entry.kind, list = []);
  list.push(entry);
}

for (const meta of widgetMetas) {
  const familyEntries = familyByKind.get(meta.kind) ?? [];
  const legacyEntries = legacyByKind.get(meta.kind) ?? [];
  if (familyEntries.length === 0 && legacyEntries.length === 0) {
    errors.push(`kind "${meta.kind}" is neither in FAMILY_VIEWS nor in LEGACY_WIDGETS`);
    continue;
  }
  if (familyEntries.length > 0 && legacyEntries.length === 0 && meta.playsBackDated !== undefined) {
    errors.push(`family kind "${meta.kind}" must not set playsBackDated in definition.ts`);
  }
  if (familyEntries.length > 0 && legacyEntries.length > 0) {
    if (familyEntries.some(entry => normalizeView(entry.view) === DEFAULT_VIEW)) {
      errors.push(`partially ported kind "${meta.kind}" cannot use default view in FAMILY_VIEWS`);
    }
    if (legacyEntries.some(entry => normalizeView(entry.view) === DEFAULT_VIEW)) {
      errors.push(`partially ported kind "${meta.kind}" cannot use default view in LEGACY_WIDGETS`);
    }
    const viewOptionKey = familyEntries.find(entry => entry.viewOption)?.viewOption ?? meta.viewSwitch?.option ?? 'view';
    const viewField = meta.options?.find(field => field.key === viewOptionKey);
    if (viewField?.kind === 'choice') {
      const covered = new Set([
        ...familyEntries.map(entry => normalizeView(entry.view)),
        ...legacyEntries.map(entry => normalizeView(entry.view)),
      ]);
      for (const choice of viewField.choices) {
        if (!covered.has(choice.value)) {
          errors.push(`kind "${meta.kind}" view "${choice.value}" is missing from both FAMILY_VIEWS and LEGACY_WIDGETS`);
        }
      }
    }
  }
}

function resolveImportPath(fromFile, specifier) {
  const base = resolve(dirname(fromFile), specifier);
  const candidates = [base, `${base}.ts`, `${base}.tsx`, `${base}.js`, `${base}.mjs`, resolve(base, 'index.ts'), resolve(base, 'index.tsx')];
  return candidates.find(candidate => existsSync(candidate)) ?? null;
}

function checkWorkerSafe(entryRelPaths) {
  const visited = new Set();
  const stack = entryRelPaths.map(rel => resolve(root, rel));
  while (stack.length > 0) {
    const file = stack.pop();
    if (!file || visited.has(file)) continue;
    visited.add(file);
    if (file.endsWith('.tsx') || file.endsWith('.css')) {
      errors.push(`worker-safe manifest graph reaches UI file: ${file}`);
      continue;
    }
    const source = readFileSync(file, 'utf8');
    const ast = parseAst(source, { lang: file.endsWith('.ts') ? 'ts' : 'js' });
    for (const node of ast.body) {
      if (node.type !== 'ImportDeclaration' && node.type !== 'ExportNamedDeclaration' && node.type !== 'ExportAllDeclaration') continue;
      if (node.importKind === 'type' || node.exportKind === 'type') continue;
      const spec = node.source?.value;
      if (!spec) continue;
      if (node.type === 'ImportDeclaration' && node.specifiers.length > 0 && node.specifiers.every(item => item.importKind === 'type')) continue;
      if (spec === 'react' || spec.startsWith('react/') || spec === 'react-dom' || spec.startsWith('react-dom/') || spec.endsWith('.css')) {
        errors.push(`worker-safe manifest graph imports "${spec}" from ${file}`);
        continue;
      }
      if (spec.startsWith('.')) {
        const next = resolveImportPath(file, spec);
        if (next) stack.push(next);
      }
    }
  }
}

checkWorkerSafe([
  'ui/src/widgets/familyManifest.ts',
  'ui/src/families/manifest.ts',
  'ui/src/families/registry.ts',
  'ui/src/families/table/registry.ts',
  'ui/src/families/bars/registry.ts',
  'ui/src/families/time-chart/declaration.ts',
  'ui/src/families/heat/lanesRegistry.ts',
  'ui/src/families/heat/matrixRegistry.ts',
]);

const drawnSource = readFileSync(resolve(root, 'ui/src/draw/views/drawn.ts'), 'utf8');
if (drawnSource.includes('registerAll')) {
  errors.push('ui/src/draw/views/drawn.ts must not eagerly import registerAll');
}

const familyKinds = familyByKind.size;
const familyViews = FAMILY_VIEWS.length;
const legacyCount = LEGACY_WIDGETS.length;

if (errors.length > 0) {
  for (const error of errors) console.error(`widget-manifest: ${error}`);
  console.error(`widget-manifest: FAILED (${errors.length} error(s)); ${familyViews} family view(s) across ${familyKinds} kind(s); ${legacyCount} entry/entries still on old renderer`);
  process.exit(1);
}

console.log(`widget-manifest: OK (${familyViews} family view(s) across ${familyKinds} kind(s); ${legacyCount} still on old renderer)`);
