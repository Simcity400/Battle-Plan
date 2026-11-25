import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseOptionKey, scopedOptionKey } from '../ui/src/dashboard/options.ts';
import { DEFAULT_VIEW, normalizeView, viewKey } from '../ui/src/families/manifest.ts';
import { widgetMetas } from '../ui/src/widgets/definitions.ts';
import { FAMILY_MANIFESTS, FAMILY_VIEWS } from '../ui/src/widgets/familyManifest.ts';
import { NATIVE_WIDGETS } from '../ui/src/widgets/manifest.ts';
import { walkImportGraph } from './ui-import-graph.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const errors = [];

const manifestDir = resolve(root, 'ui/src/widgets/manifest');
const manifestFiles = existsSync(manifestDir)
  ? readdirSync(manifestDir).filter(name => name.endsWith('.ts') && !name.endsWith('.test.ts')).sort()
  : [];

const registeredFamilies = new Set();
for (const mod of FAMILY_MANIFESTS) {
  if (!mod.family || typeof mod.family !== 'string') {
    errors.push('FAMILY_MANIFESTS entry missing family id');
    continue;
  }
  if (registeredFamilies.has(mod.family)) errors.push(`duplicate FAMILY_MANIFESTS family: ${mod.family}`);
  registeredFamilies.add(mod.family);
  if (typeof mod.lane !== 'function') errors.push(`FAMILY_MANIFESTS(${mod.family}) missing its lane loader`);
  if (typeof mod.hostAdapter?.deriveAuditRows !== 'function') errors.push(`FAMILY_MANIFESTS(${mod.family}) host adapter has no deriveAuditRows, so its audit checks nothing`);
  for (const entry of mod.views) {
    if (entry.family !== mod.family) {
      errors.push(`view (${entry.kind}, ${normalizeView(entry.view)}) declares family "${entry.family}" inside "${mod.family}" manifest`);
    }
  }
}

for (const file of manifestFiles) {
  const expectedFamily = file.slice(0, -'.ts'.length);
  if (!registeredFamilies.has(expectedFamily)) {
    errors.push(`ui/src/widgets/manifest/${file} is not registered in FAMILY_MANIFESTS in ui/src/widgets/familyManifest.ts`);
  }
}

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
  if (view !== DEFAULT_VIEW && !parseOptionKey(scopedOptionKey('option', view))) {
    errors.push(`FAMILY_VIEWS(${entry.kind}, ${view}): the view name cannot scope a stored option key`);
  }
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

const nativeKinds = new Set();
for (const { kind, renderer, load } of NATIVE_WIDGETS) {
  if (!metaByKind.has(kind)) errors.push(`NATIVE_WIDGETS references unknown kind: ${kind}`);
  if (renderer !== 'native' || typeof load !== 'function') errors.push(`NATIVE_WIDGETS(${kind}) requires a native renderer and load function`);
  if (nativeKinds.has(kind) || familyByKind.has(kind)) errors.push(`native kind "${kind}" is registered more than once`);
  nativeKinds.add(kind);
}

for (const meta of widgetMetas) {
  if (nativeKinds.has(meta.kind)) continue;
  const familyEntries = familyByKind.get(meta.kind) ?? [];
  if (familyEntries.length === 0) {
    errors.push(`kind "${meta.kind}" is missing from FAMILY_VIEWS and NATIVE_WIDGETS`);
    continue;
  }
  if (familyEntries.length > 1) {
    if (familyEntries.some(entry => normalizeView(entry.view) === DEFAULT_VIEW)) {
      errors.push(`multi-view kind "${meta.kind}" cannot use default view in FAMILY_VIEWS`);
    }
  }
  const named = familyEntries.some(entry => normalizeView(entry.view) !== DEFAULT_VIEW);
  if (named) {
    const viewOptionKey = familyEntries.find(entry => entry.viewOption)?.viewOption ?? meta.viewSwitch?.option ?? 'view';
    const viewField = meta.options?.find(field => field.key === viewOptionKey);
    if (viewField?.kind === 'choice') {
      const covered = new Set(familyEntries.map(entry => normalizeView(entry.view)));
      for (const choice of viewField.choices) {
        if (!covered.has(choice.value)) {
          errors.push(`kind "${meta.kind}" view "${choice.value}" is missing from FAMILY_VIEWS`);
        }
      }
    }
  }
}

function checkWorkerSafe(entryRelPaths) {
  const { visited, externalImports } = walkImportGraph(entryRelPaths, root, {
    includeTypes: false,
    includeDynamic: false,
    includeWorkerUrls: false,
    shouldTraverse: file => {
      if (file.endsWith('.tsx') || file.endsWith('.css')) {
        errors.push(`worker-safe manifest graph reaches UI file: ${file}`);
        return false;
      }
      return true;
    },
  });
  for (const file of visited) {
    const externals = externalImports.get(file);
    if (!externals) continue;
    for (const spec of externals) {
      if (spec === 'react' || spec.startsWith('react/') || spec === 'react-dom' || spec.startsWith('react-dom/') || spec.endsWith('.css')) {
        errors.push(`worker-safe manifest graph imports "${spec}" from ${file}`);
      }
    }
  }
}

checkWorkerSafe([
  'ui/src/widgets/familyManifest.ts',
  ...manifestFiles.map(file => `ui/src/widgets/manifest/${file}`),
  'ui/src/families/manifest.ts',
  'ui/src/families/registry.ts',
]);

const drawnSource = readFileSync(resolve(root, 'ui/src/draw/views/drawn.ts'), 'utf8');
if (drawnSource.includes('registerAll')) {
  errors.push('ui/src/draw/views/drawn.ts must not eagerly import registerAll');
}

const familyKinds = familyByKind.size;
const familyViews = FAMILY_VIEWS.length;

if (errors.length > 0) {
  for (const error of errors) console.error(`widget-manifest: ${error}`);
  console.error(`widget-manifest: FAILED (${errors.length} error(s)); ${familyViews} family view(s) across ${familyKinds} kind(s); ${nativeKinds.size} native kind(s)`);
  process.exit(1);
}

console.log(`widget-manifest: OK (${familyViews} family view(s) across ${familyKinds} kind(s); ${nativeKinds.size} native kind(s))`);
