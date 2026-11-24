import { readFileSync, statSync } from 'node:fs';
import { dirname, extname, relative, resolve } from 'node:path';
import { parseAst } from 'rolldown/parseAst';

const SCRIPT_EXTENSIONS = ['.ts', '.tsx', '.mts', '.js', '.mjs', '.cjs'];
const RESOLVE_EXTENSIONS = ['.ts', '.tsx', '.mts', '.js', '.mjs', '.css'];
const INDEX_FILES = ['index.ts', 'index.tsx', 'index.js', 'index.mjs'];

function isFile(filePath) {
  try {
    return statSync(filePath).isFile();
  } catch {
    return false;
  }
}

export function toPosix(filePath) {
  return filePath.replace(/\\/g, '/');
}

export function relPosix(root, filePath) {
  return toPosix(relative(root, filePath));
}

export function stripSpecifierSuffix(specifier) {
  return specifier.split(/[?#]/, 1)[0];
}

export function resolveLocalImport(fromFile, specifier, root) {
  const clean = stripSpecifierSuffix(specifier);
  if (!clean) return null;
  let base;
  if (clean.startsWith('/ui/src/')) {
    base = resolve(root, clean.slice(1));
  } else if (clean.startsWith('/src/')) {
    base = resolve(root, 'ui', clean.slice(1));
  } else if (clean.startsWith('.')) {
    base = resolve(dirname(fromFile), clean);
  } else {
    return null;
  }
  const candidates = [
    base,
    ...RESOLVE_EXTENSIONS.map(ext => `${base}${ext}`),
    ...INDEX_FILES.map(index => resolve(base, index)),
  ];
  for (const candidate of candidates) {
    if (isFile(candidate)) return candidate;
  }
  return null;
}

function isImportMetaUrl(node) {
  return node?.type === 'MemberExpression'
    && !node.computed
    && node.property?.type === 'Identifier'
    && node.property.name === 'url'
    && node.object?.type === 'MetaProperty'
    && node.object.meta?.name === 'import'
    && node.object.property?.name === 'meta';
}

function extractHtmlImports(source) {
  const out = [];
  const pattern = /<(?:script\b[^>]*\bsrc|link\b[^>]*\bhref)=["']([^"']+)["']/gi;
  for (const match of source.matchAll(pattern)) {
    out.push({ specifier: match[1], kind: 'html', isType: false });
  }
  return out;
}

function extractCssImports(source) {
  const stripped = source.replace(/\/\*[\s\S]*?\*\//g, '');
  const out = [];
  const pattern = /@import\s+(?:url\(\s*)?["']([^"']+)["']/gi;
  for (const match of stripped.matchAll(pattern)) {
    out.push({ specifier: match[1], kind: 'css', isType: false });
  }
  return out;
}

function astLang(filePath) {
  if (filePath.endsWith('.tsx')) return 'tsx';
  if (filePath.endsWith('.ts') || filePath.endsWith('.mts')) return 'ts';
  return 'js';
}

function walkAst(rootNode, visitor) {
  const stack = [rootNode];
  while (stack.length > 0) {
    const node = stack.pop();
    if (!node || typeof node !== 'object') continue;
    visitor(node);
    for (const value of Object.values(node)) {
      if (Array.isArray(value)) {
        for (let i = value.length - 1; i >= 0; i -= 1) {
          const item = value[i];
          if (item && typeof item === 'object' && typeof item.type === 'string') stack.push(item);
        }
      } else if (value && typeof value === 'object' && typeof value.type === 'string') {
        stack.push(value);
      }
    }
  }
}

export function extractImportSpecifiers(filePath, options = {}) {
  const {
    includeTypes = true,
    includeDynamic = true,
    includeWorkerUrls = true,
    includeTestStringRefs = false,
  } = options;
  const ext = extname(filePath).toLowerCase();
  const source = readFileSync(filePath, 'utf8');
  if (ext === '.html') return extractHtmlImports(source);
  if (ext === '.css') return extractCssImports(source);
  if (!SCRIPT_EXTENSIONS.includes(ext)) return [];

  const ast = parseAst(source, { lang: astLang(filePath) });
  const out = [];
  const testRefPattern = /(\/(?:ui\/)?src\/[A-Za-z0-9_./-]+\.(?:tsx?|css))\b/g;

  const visitNode = node => {
    if (node.type === 'ImportDeclaration') {
      const spec = node.source?.value;
      if (typeof spec !== 'string') return;
      const isType = node.importKind === 'type'
        || (node.specifiers.length > 0 && node.specifiers.every(item => item.importKind === 'type'));
      if (includeTypes || !isType) out.push({ specifier: spec, kind: 'import', isType });
      return;
    }
    if (node.type === 'ExportNamedDeclaration' || node.type === 'ExportAllDeclaration') {
      const spec = node.source?.value;
      if (typeof spec !== 'string') return;
      const isType = node.exportKind === 'type'
        || (node.type === 'ExportNamedDeclaration' && node.specifiers.length > 0 && node.specifiers.every(item => item.exportKind === 'type'));
      if (includeTypes || !isType) out.push({ specifier: spec, kind: 'export', isType });
      return;
    }
    if (includeDynamic && node.type === 'ImportExpression') {
      if (node.source?.type === 'Literal' && typeof node.source.value === 'string') {
        out.push({ specifier: node.source.value, kind: 'dynamic', isType: false });
      }
      return;
    }
    if (includeTypes && node.type === 'TSImportType') {
      const spec = node.source?.value ?? node.argument?.literal?.value;
      if (typeof spec === 'string') {
        out.push({ specifier: spec, kind: 'type-import', isType: true });
      }
      return;
    }
    if (includeWorkerUrls && node.type === 'NewExpression'
      && node.callee?.type === 'Identifier' && node.callee.name === 'URL'
      && node.arguments.length >= 2
      && node.arguments[0]?.type === 'Literal' && typeof node.arguments[0].value === 'string'
      && isImportMetaUrl(node.arguments[1])) {
      out.push({ specifier: node.arguments[0].value, kind: 'url', isType: false });
      return;
    }
    if (includeTestStringRefs) {
      let text = null;
      if (node.type === 'Literal' && typeof node.value === 'string') text = node.value;
      else if (node.type === 'TemplateElement' && typeof node.value?.cooked === 'string') text = node.value.cooked;
      if (text && text.includes('src/')) {
        for (const match of text.matchAll(testRefPattern)) {
          out.push({ specifier: match[1], kind: 'test-ref', isType: false });
        }
      }
    }
  };

  const needsDeepWalk = ((includeDynamic || includeTypes) && source.includes('import('))
    || (includeWorkerUrls && source.includes('import.meta.url'))
    || (includeTestStringRefs && source.includes('src/'));

  if (!needsDeepWalk) {
    for (const node of ast.body) visitNode(node);
  } else {
    walkAst(ast, visitNode);
  }
  return out;
}

export function walkImportGraph(entryPaths, root, options = {}) {
  const visited = new Set();
  const edges = new Map();
  const externalImports = new Map();
  const stack = entryPaths.map(entry => resolve(root, entry));
  const shouldTraverse = options.shouldTraverse ?? (() => true);
  const cache = options.cache ?? new Map();
  const cacheTag = `${options.includeTypes !== false ? 1 : 0}${options.includeDynamic !== false ? 1 : 0}${options.includeWorkerUrls !== false ? 1 : 0}${options.includeTestStringRefs ? 1 : 0}`;

  while (stack.length > 0) {
    const file = stack.pop();
    if (!file || visited.has(file) || !isFile(file)) continue;
    visited.add(file);
    if (!shouldTraverse(file)) continue;

    const cacheKey = `${cacheTag}\0${file}`;
    let parsed = cache.get(cacheKey);
    if (!parsed) {
      const specs = extractImportSpecifiers(file, options);
      const resolvedTargets = new Set();
      const externals = new Set();
      for (const entry of specs) {
        const target = resolveLocalImport(file, entry.specifier, root);
        if (target) resolvedTargets.add(target);
        else if (!entry.specifier.startsWith('.') && !entry.specifier.startsWith('/')) externals.add(entry.specifier);
      }
      parsed = { resolvedTargets, externals };
      cache.set(cacheKey, parsed);
    }

    edges.set(file, parsed.resolvedTargets);
    if (parsed.externals.size > 0) externalImports.set(file, parsed.externals);
    for (const target of parsed.resolvedTargets) {
      if (!visited.has(target)) stack.push(target);
    }
  }

  return { visited, edges, externalImports };
}
