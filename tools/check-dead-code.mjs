import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, unlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { parseAst } from 'rolldown/parseAst';
import { astLang, extractImportSpecifiers, relPosix, resolveLocalImport, staticStringLiteral, walkAst, walkImportGraph } from './ui-import-graph.mjs';

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
const GLOBAL_REGISTRIES = new Set([
  'ui/src/widgets/manifest.ts',
  'ui/src/widgets/familyManifest.ts',
  'ui/src/widgets/legacyManifest.ts',
  'ui/src/widgets/rendererLoaders.ts',
  'ui/src/widgets/definitions.ts',
  'ui/src/widgets/index.ts',
]);
const NON_PRODUCT_PROPERTY_NAMES = new Set(['kind', 'view', 'owner', 'className', 'icon', 'category', 'family']);

const BASE_SNAPSHOT_RULES = [
  { pathspec: 'ui/src', match: rel => rel.startsWith('ui/src/') },
  { pathspec: 'ui/vite.config.ts', match: rel => rel === 'ui/vite.config.ts' },
  { pathspec: 'ui/browserBridge.ts', match: rel => rel === 'ui/browserBridge.ts' },
  { pathspec: 'ui/replayLibrary.ts', match: rel => rel === 'ui/replayLibrary.ts' },
  { pathspec: ':(glob)ui/*.html', match: rel => /^ui\/[^/]+\.html$/.test(rel) },
  { pathspec: ':(glob)crates/*/src/**/*.rs', match: rel => /^crates\/[^/]+\/src\/.+\.rs$/.test(rel) },
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

function collectBlockLocalBindings(stmts) {
  const names = [];
  for (const stmt of stmts ?? []) {
    if (!stmt) continue;
    if (stmt.type === 'VariableDeclaration') {
      for (const d of stmt.declarations) collectBindingNames(d.id, names);
    } else if ((stmt.type === 'FunctionDeclaration' || stmt.type === 'ClassDeclaration'
      || stmt.type === 'TSTypeAliasDeclaration' || stmt.type === 'TSInterfaceDeclaration'
      || stmt.type === 'TSEnumDeclaration') && stmt.id?.name) {
      names.push(stmt.id.name);
    }
  }
  return names;
}

function collectTypeParamNames(node) {
  const names = [];
  const params = node?.typeParameters?.params;
  if (Array.isArray(params)) {
    for (const p of params) {
      if (p?.name?.type === 'Identifier') names.push(p.name.name);
      else if (typeof p?.name === 'string') names.push(p.name);
    }
  }
  return names;
}

function leftmostIdent(node) {
  let cur = node;
  while (cur && cur.type === 'TSQualifiedName') cur = cur.left;
  return cur?.type === 'Identifier' ? cur.name : null;
}

function collectIdentifierMemberUsage(rootNode, targetName, initialShadowed = new Set()) {
  const used = new Set();
  let opaque = false;

  const walk = (node, parent, key, shadowed) => {
    if (!node || typeof node !== 'object') return;
    let nextShadowed = shadowed;

    if (node.type === 'FunctionDeclaration' || node.type === 'FunctionExpression' || node.type === 'ArrowFunctionExpression') {
      const local = [];
      if (node.type === 'FunctionExpression' && node.id?.name) local.push(node.id.name);
      for (const p of node.params ?? []) collectBindingNames(p, local);
      local.push(...collectTypeParamNames(node));
      if (local.includes(targetName)) return;
      if (local.length > 0) nextShadowed = new Set([...shadowed, ...local]);
    } else if (node.type === 'BlockStatement' && node !== rootNode) {
      const local = collectBlockLocalBindings(node.body);
      if (local.includes(targetName)) return;
      if (local.length > 0) nextShadowed = new Set([...shadowed, ...local]);
    } else if (node.type === 'CatchClause' && node.param) {
      const local = [];
      collectBindingNames(node.param, local);
      if (local.includes(targetName)) return;
      if (local.length > 0) nextShadowed = new Set([...shadowed, ...local]);
    } else if ((node.type === 'ForStatement' || node.type === 'ForInStatement' || node.type === 'ForOfStatement')) {
      const init = node.init ?? node.left;
      if (init?.type === 'VariableDeclaration') {
        const local = [];
        for (const d of init.declarations) collectBindingNames(d.id, local);
        if (local.includes(targetName)) return;
        if (local.length > 0) nextShadowed = new Set([...shadowed, ...local]);
      }
    }

    if (node.type === 'Identifier' && node.name === targetName && !nextShadowed.has(targetName)) {
      if (parent?.type === 'MemberExpression' && key === 'object') {
        if (!parent.computed && parent.property?.type === 'Identifier') used.add(parent.property.name);
        else opaque = true;
        return;
      }
      if (parent?.type === 'JSXMemberExpression' && key === 'object') {
        if (parent.property?.type === 'JSXIdentifier') used.add(parent.property.name);
        else opaque = true;
        return;
      }
      if (parent?.type === 'TSQualifiedName' && key === 'left') {
        if (parent.right?.type === 'Identifier') used.add(parent.right.name);
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
      if (parent?.type === 'Property' && key === 'key' && !parent.computed && !parent.shorthand) return;
      if (parent?.type === 'MemberExpression' && key === 'property' && !parent.computed) return;
      if (parent?.type === 'VariableDeclarator' && key === 'id') return;
      if (parent?.type === 'TSPropertySignature' && key === 'key' && !parent.computed) return;
      if ((parent?.type === 'MethodDefinition' || parent?.type === 'PropertyDefinition') && key === 'key' && !parent.computed) return;
      opaque = true;
      return;
    }

    for (const [childKey, value] of Object.entries(node)) {
      if (Array.isArray(value)) {
        for (const el of value) {
          if (el && typeof el === 'object' && typeof el.type === 'string') walk(el, node, childKey, nextShadowed);
        }
      } else if (value && typeof value === 'object' && typeof value.type === 'string') {
        walk(value, node, childKey, nextShadowed);
      }
    }
  };

  walk(rootNode, null, null, initialShadowed);
  return { used, opaque };
}

function extractCallbackParamUsage(cb) {
  if (!cb || (cb.type !== 'ArrowFunctionExpression' && cb.type !== 'FunctionExpression')) {
    return ['*'];
  }
  const p0 = cb.params[0];
  if (!p0) return ['*'];
  if (p0.type === 'Identifier') {
    const { used, opaque } = collectIdentifierMemberUsage(cb.body, p0.name);
    return opaque ? ['*'] : [...used];
  }
  if (p0.type === 'ObjectPattern') {
    const names = [];
    for (const prop of p0.properties) {
      if (prop.type === 'Property' && !prop.computed && prop.key?.type === 'Identifier') names.push(prop.key.name);
      else return ['*'];
    }
    return names;
  }
  return ['*'];
}

function isLazyCall(node) {
  if (node?.type !== 'CallExpression') return false;
  if (node.callee?.type === 'Identifier' && node.callee.name === 'lazy') return true;
  if (node.callee?.type === 'MemberExpression'
    && !node.callee.computed
    && node.callee.object?.type === 'Identifier' && node.callee.object.name === 'React'
    && node.callee.property?.type === 'Identifier' && node.callee.property.name === 'lazy') {
    return true;
  }
  return false;
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

function findDirectDynamicImport(fnNode) {
  if (!fnNode) return null;
  if (fnNode.type === 'ArrowFunctionExpression') {
    const body = unwrapExpression(fnNode.body);
    if (body?.type === 'ImportExpression') return body;
    if (body?.type === 'BlockStatement' && body.body.length === 1 && body.body[0]?.type === 'ReturnStatement') {
      const arg = unwrapExpression(body.body[0].argument);
      if (arg?.type === 'ImportExpression') return arg;
    }
  } else if ((fnNode.type === 'FunctionExpression' || fnNode.type === 'FunctionDeclaration') && fnNode.body?.type === 'BlockStatement') {
    if (fnNode.body.body.length === 1 && fnNode.body.body[0]?.type === 'ReturnStatement') {
      const arg = unwrapExpression(fnNode.body.body[0].argument);
      if (arg?.type === 'ImportExpression') return arg;
    }
  }
  return null;
}

const TYPE_CHILD_KEYS = new Set([
  'typeAnnotation',
  'returnType',
  'typeParameters',
  'typeArguments',
  'superTypeArguments',
  'implements',
  'extends',
]);

const TYPE_NODE_TYPES = new Set([
  'TSTypeAliasDeclaration',
  'TSInterfaceDeclaration',
  'TSTypeAnnotation',
  'TSTypeReference',
  'TSTypeQuery',
  'TSTypeLiteral',
  'TSFunctionType',
  'TSConstructorType',
  'TSArrayType',
  'TSTupleType',
  'TSUnionType',
  'TSIntersectionType',
  'TSConditionalType',
  'TSMappedType',
  'TSIndexedAccessType',
  'TSInferType',
  'TSTypeOperator',
  'TSImportType',
]);

function analyzeScriptModule(root, relPath, cache) {
  if (cache.has(relPath)) return cache.get(relPath);
  const full = resolve(root, relPath);
  const ext = extname(full).toLowerCase();
  if (!SCRIPT_EXTENSIONS.has(ext) || !existsSync(full)) {
    cache.set(relPath, null);
    return null;
  }
  const source = readFileSync(full, 'utf8');
  const ast = parseAst(source, { lang: astLang(full) });

  const declaredExports = new Map();
  const reexports = [];
  const starReexports = [];
  const importedBindings = new Map();
  const nsBindings = new Map();
  const bareImports = [];
  const workerUrls = [];
  const statements = [];
  const localLoaders = new Map();

  for (let stmtIndex = 0; stmtIndex < ast.body.length; stmtIndex += 1) {
    const node = ast.body[stmtIndex];
    const bindings = [];
    let isDeclOnly = false;
    let isTypeOnly = false;

    if (node.type === 'ImportDeclaration') {
      isDeclOnly = true;
      const spec = staticStringLiteral(node.source);
      if (spec !== null) {
        const target = resolveLocalImport(full, spec, root);
        if (target) {
          const targetRel = relPosix(root, target);
          const declIsType = node.importKind === 'type';
          if (node.specifiers.length === 0) {
            bareImports.push({ targetRel, isType: declIsType });
          } else {
            for (const item of node.specifiers) {
              const itemIsType = declIsType || item.importKind === 'type';
              if (item.type === 'ImportDefaultSpecifier') {
                importedBindings.set(item.local.name, { targetRel, importedName: 'default', isType: itemIsType });
              } else if (item.type === 'ImportSpecifier') {
                const importedName = item.imported?.name ?? item.imported?.value;
                if (importedName) importedBindings.set(item.local.name, { targetRel, importedName, isType: itemIsType });
              } else if (item.type === 'ImportNamespaceSpecifier') {
                nsBindings.set(item.local.name, { targetRel, isType: itemIsType });
              }
            }
          }
        }
      }
    } else if (node.type === 'ExportDefaultDeclaration') {
      isDeclOnly = true;
      if (node.declaration?.id?.name) bindings.push(node.declaration.id.name);
      declaredExports.set('default', { stmtIndex, localName: node.declaration?.id?.name ?? null, isType: false, isReexport: false, specifierOnly: false });
    } else if (node.type === 'ExportNamedDeclaration') {
      isDeclOnly = true;
      const sourceSpec = staticStringLiteral(node.source);
      if (sourceSpec !== null) {
        const target = resolveLocalImport(full, sourceSpec, root);
        const targetRel = target ? relPosix(root, target) : null;
        for (const spec of node.specifiers) {
          const exported = spec.exported?.name ?? spec.exported?.value;
          const local = spec.local?.name ?? spec.local?.value;
          if (!exported) continue;
          const isType = node.exportKind === 'type' || spec.exportKind === 'type';
          declaredExports.set(exported, { stmtIndex, localName: null, isType, isReexport: true, specifierOnly: false });
          if (targetRel && local) reexports.push({ exported, targetRel, targetName: local, isType, stmtIndex });
        }
      } else if (node.declaration) {
        const decl = node.declaration;
        isTypeOnly = decl.type === 'TSTypeAliasDeclaration' || decl.type === 'TSInterfaceDeclaration';
        if (decl.type === 'VariableDeclaration') {
          const names = [];
          for (const item of decl.declarations) {
            collectBindingNames(item.id, names);
            if (item.id?.type === 'Identifier') {
              const imp = findDirectDynamicImport(unwrapExpression(item.init));
              if (imp) {
                const spec = staticStringLiteral(imp.source);
                const resolved = spec !== null ? resolveLocalImport(full, spec, root) : null;
                if (resolved) localLoaders.set(item.id.name, { targetRel: relPosix(root, resolved), exported: true, stmtIndex, impNode: imp });
              }
            }
          }
          for (const name of names) {
            bindings.push(name);
            declaredExports.set(name, { stmtIndex, localName: name, isType: false, isReexport: false, specifierOnly: false });
          }
        } else if (decl.id?.name) {
          bindings.push(decl.id.name);
          declaredExports.set(decl.id.name, { stmtIndex, localName: decl.id.name, isType: isTypeOnly, isReexport: false, specifierOnly: false });
          if (decl.type === 'FunctionDeclaration') {
            const imp = findDirectDynamicImport(decl);
            if (imp) {
              const spec = staticStringLiteral(imp.source);
              const resolved = spec !== null ? resolveLocalImport(full, spec, root) : null;
              if (resolved) localLoaders.set(decl.id.name, { targetRel: relPosix(root, resolved), exported: true, stmtIndex, impNode: imp });
            }
          }
        }
      } else {
        for (const spec of node.specifiers) {
          const exported = spec.exported?.name ?? spec.exported?.value;
          const local = spec.local?.name ?? spec.local?.value;
          if (!exported) continue;
          const isType = node.exportKind === 'type' || spec.exportKind === 'type';
          declaredExports.set(exported, { stmtIndex, localName: local, isType, isReexport: false, specifierOnly: true });
        }
      }
    } else if (node.type === 'ExportAllDeclaration') {
      isDeclOnly = true;
      const sourceSpec = staticStringLiteral(node.source);
      if (sourceSpec !== null) {
        const target = resolveLocalImport(full, sourceSpec, root);
        const targetRel = target ? relPosix(root, target) : null;
        const isType = node.exportKind === 'type';
        if (node.exported) {
          const expName = node.exported.name ?? node.exported.value;
          if (expName) {
            declaredExports.set(expName, { stmtIndex, localName: null, isType, isReexport: true, specifierOnly: false });
            if (targetRel) reexports.push({ exported: expName, targetRel, targetName: '*', isType, stmtIndex });
          }
        } else if (targetRel) {
          starReexports.push({ targetRel, isType, stmtIndex });
        }
      }
    } else if (node.type === 'VariableDeclaration') {
      isDeclOnly = true;
      for (const item of node.declarations) {
        collectBindingNames(item.id, bindings);
        if (item.id?.type === 'Identifier') {
          const imp = findDirectDynamicImport(unwrapExpression(item.init));
          if (imp) {
            const spec = staticStringLiteral(imp.source);
            const resolved = spec !== null ? resolveLocalImport(full, spec, root) : null;
            if (resolved) localLoaders.set(item.id.name, { targetRel: relPosix(root, resolved), exported: false, stmtIndex, impNode: imp });
          }
        }
      }
    } else if (node.type === 'FunctionDeclaration' || node.type === 'ClassDeclaration'
      || node.type === 'TSTypeAliasDeclaration' || node.type === 'TSInterfaceDeclaration'
      || node.type === 'TSEnumDeclaration') {
      isDeclOnly = true;
      isTypeOnly = node.type === 'TSTypeAliasDeclaration' || node.type === 'TSInterfaceDeclaration';
      if (node.id?.name) {
        bindings.push(node.id.name);
        if (node.type === 'FunctionDeclaration') {
          const imp = findDirectDynamicImport(node);
          if (imp) {
            const spec = staticStringLiteral(imp.source);
            const resolved = spec !== null ? resolveLocalImport(full, spec, root) : null;
            if (resolved) localLoaders.set(node.id.name, { targetRel: relPosix(root, resolved), exported: false, stmtIndex, impNode: imp });
          }
        }
      }
    }

    statements.push({
      stmtIndex,
      node,
      bindings,
      refs: new Set(),
      typeRefs: new Set(),
      nsUses: [],
      dynamicUses: [],
      inlineTypeUses: [],
      isSideEffect: !isDeclOnly,
      isTypeOnly,
      isDeclOnly,
    });
  }

  for (const [, meta] of declaredExports) {
    if (meta.specifierOnly && meta.localName && localLoaders.has(meta.localName)) {
      localLoaders.get(meta.localName).exported = true;
    }
  }

  const isLegacyManifestFile = relPath === 'ui/src/widgets/legacyManifest.ts';
  const isWidgetManifestFile = relPath === 'ui/src/widgets/manifest.ts';
  const loaderImpNodes = new Set([...localLoaders.values()].map(item => item.impNode));

  for (const stmt of statements) {
    const node = stmt.node;
    if (node.type === 'ImportDeclaration' || (node.type === 'ExportNamedDeclaration' && node.source) || node.type === 'ExportAllDeclaration') {
      continue;
    }

    const handledImports = new Set();

    const walkStmt = (cur, parent, key, shadowed, inType, enclosingBlock) => {
      if (!cur || typeof cur !== 'object') return;
      const nextInType = inType || TYPE_NODE_TYPES.has(cur.type) || TYPE_CHILD_KEYS.has(key);
      let nextShadowed = shadowed;
      let nextBlock = enclosingBlock;

      if (cur.type === 'FunctionDeclaration' || cur.type === 'FunctionExpression' || cur.type === 'ArrowFunctionExpression') {
        const local = [];
        if (cur.type === 'FunctionExpression' && cur.id?.name) local.push(cur.id.name);
        for (const p of cur.params ?? []) collectBindingNames(p, local);
        local.push(...collectTypeParamNames(cur));
        if (local.length > 0) nextShadowed = new Set([...shadowed, ...local]);
        nextBlock = cur.body ?? cur;
      } else if (cur.type === 'BlockStatement') {
        const local = collectBlockLocalBindings(cur.body);
        if (local.length > 0) nextShadowed = new Set([...shadowed, ...local]);
        nextBlock = cur;
      } else if (cur.type === 'CatchClause' && cur.param) {
        const local = [];
        collectBindingNames(cur.param, local);
        if (local.length > 0) nextShadowed = new Set([...shadowed, ...local]);
      } else if (cur.type === 'ForStatement' || cur.type === 'ForInStatement' || cur.type === 'ForOfStatement') {
        const init = cur.init ?? cur.left;
        if (init?.type === 'VariableDeclaration') {
          const local = [];
          for (const d of init.declarations) collectBindingNames(d.id, local);
          if (local.length > 0) nextShadowed = new Set([...shadowed, ...local]);
        }
      } else if (cur.type === 'TSTypeAliasDeclaration' || cur.type === 'TSInterfaceDeclaration' || cur.type === 'ClassDeclaration' || cur.type === 'ClassExpression') {
        const local = collectTypeParamNames(cur);
        if (local.length > 0) nextShadowed = new Set([...shadowed, ...local]);
      }

      if (cur.type === 'NewExpression'
        && cur.callee?.type === 'Identifier' && cur.callee.name === 'URL'
        && cur.arguments.length >= 2
        && isImportMetaUrl(cur.arguments[1])) {
        const spec = staticStringLiteral(cur.arguments[0]);
        if (spec !== null) {
          const target = resolveLocalImport(full, spec, root);
          if (target) workerUrls.push({ targetRel: relPosix(root, target), stmtIndex: stmt.stmtIndex });
        }
      }

      if (cur.type === 'TSImportType') {
        const spec = staticStringLiteral(cur.source) ?? staticStringLiteral(cur.argument?.literal);
        if (spec !== null) {
          const target = resolveLocalImport(full, spec, root);
          if (target) {
            stmt.inlineTypeUses.push({ targetRel: relPosix(root, target), name: leftmostIdent(cur.qualifier) ?? '*' });
          }
        }
      }

      if (cur.type === 'CallExpression' && isLazyCall(cur) && cur.arguments.length >= 1) {
        const imp = findDirectDynamicImport(unwrapExpression(cur.arguments[0]));
        if (imp) {
          handledImports.add(imp);
          const spec = staticStringLiteral(imp.source);
          const target = spec !== null ? resolveLocalImport(full, spec, root) : null;
          if (target) stmt.dynamicUses.push({ targetRel: relPosix(root, target), names: ['default'], legacyLoad: false });
        }
      }

      if (cur.type === 'CallExpression' && cur.arguments.length >= 2) {
        for (let i = 0; i < cur.arguments.length - 1; i += 1) {
          const imp = findDirectDynamicImport(unwrapExpression(cur.arguments[i]));
          if (!imp) continue;
          const nextArg = unwrapExpression(cur.arguments[i + 1]);
          if (nextArg && (nextArg.type === 'ArrowFunctionExpression' || nextArg.type === 'FunctionExpression')) {
            handledImports.add(imp);
            const spec = staticStringLiteral(imp.source);
            const target = spec !== null ? resolveLocalImport(full, spec, root) : null;
            if (target) {
              stmt.dynamicUses.push({ targetRel: relPosix(root, target), names: extractCallbackParamUsage(nextArg), legacyLoad: false });
            }
          }
        }
      }

      if (cur.type === 'CallExpression'
        && cur.callee?.type === 'MemberExpression'
        && !cur.callee.computed
        && cur.callee.property?.name === 'then'
        && cur.callee.object?.type === 'ImportExpression') {
        const imp = cur.callee.object;
        handledImports.add(imp);
        const spec = staticStringLiteral(imp.source);
        const target = spec !== null ? resolveLocalImport(full, spec, root) : null;
        if (target) {
          stmt.dynamicUses.push({ targetRel: relPosix(root, target), names: extractCallbackParamUsage(cur.arguments[0]), legacyLoad: false });
        }
      }

      if (cur.type === 'AwaitExpression' && cur.argument?.type === 'ImportExpression') {
        const imp = cur.argument;
        handledImports.add(imp);
        const spec = staticStringLiteral(imp.source);
        const target = spec !== null ? resolveLocalImport(full, spec, root) : null;
        if (target) {
          const targetRel = relPosix(root, target);
          let names;
          if (parent?.type === 'MemberExpression' && key === 'object') {
            names = !parent.computed && parent.property?.type === 'Identifier' ? [parent.property.name] : ['*'];
          } else if (parent?.type === 'VariableDeclarator' && key === 'init' && parent.id?.type === 'ObjectPattern') {
            names = [];
            for (const prop of parent.id.properties) {
              if (prop.type === 'Property' && !prop.computed && prop.key?.type === 'Identifier') names.push(prop.key.name);
              else { names = ['*']; break; }
            }
          } else if (parent?.type === 'VariableDeclarator' && key === 'init' && parent.id?.type === 'Identifier') {
            const { used, opaque } = collectIdentifierMemberUsage(nextBlock ?? node, parent.id.name);
            names = opaque ? ['*'] : [...used];
          } else {
            names = ['*'];
          }
          stmt.dynamicUses.push({ targetRel, names, legacyLoad: false });
        }
      }

      if (cur.type === 'ImportExpression' && !handledImports.has(cur) && !loaderImpNodes.has(cur)) {
        const spec = staticStringLiteral(cur.source);
        const target = spec !== null ? resolveLocalImport(full, spec, root) : null;
        if (target) {
          const targetRel = relPosix(root, target);
          const inManifestLoadProp = (isLegacyManifestFile || isWidgetManifestFile)
            && parent?.type === 'ArrowFunctionExpression';
          if (inManifestLoadProp) {
            stmt.dynamicUses.push({ targetRel, names: ['default'], legacyLoad: isLegacyManifestFile });
          } else {
            stmt.dynamicUses.push({ targetRel, names: ['*'], legacyLoad: false });
          }
        }
      }

      if (cur.type === 'Identifier' && localLoaders.has(cur.name) && !nextShadowed.has(cur.name)) {
        const loader = localLoaders.get(cur.name);
        const isDeclId = (parent?.type === 'VariableDeclarator' && key === 'id')
          || (parent?.type === 'FunctionDeclaration' && key === 'id');
        if (!isDeclId && !(parent?.type === 'Property' && key === 'key' && !parent.computed && !parent.shorthand)
          && !(parent?.type === 'MemberExpression' && key === 'property' && !parent.computed)) {
          if (parent?.type === 'CallExpression' && key === 'callee') {
            const grand = null;
            void grand;
          } else {
            loader.opaqueEscape = true;
          }
        }
      }

      if (cur.type === 'CallExpression' && cur.callee?.type === 'Identifier' && localLoaders.has(cur.callee.name) && !nextShadowed.has(cur.callee.name)) {
        const loader = localLoaders.get(cur.callee.name);
        if (parent?.type === 'AwaitExpression') {
          stmt.dynamicUses.push({ targetRel: loader.targetRel, names: [], legacyLoad: false, pendingAwaitLoader: { callNode: cur, awaitNode: parent, block: nextBlock ?? node } });
        } else if (parent?.type === 'MemberExpression' && key === 'object' && !parent.computed && parent.property?.name === 'then') {
          stmt.dynamicUses.push({ targetRel: loader.targetRel, names: [], legacyLoad: false, pendingThenLoader: { memberNode: parent } });
        } else {
          stmt.dynamicUses.push({ targetRel: loader.targetRel, names: ['*'], legacyLoad: false });
        }
      }

      if ((cur.type === 'Identifier' || cur.type === 'JSXIdentifier') && !nextShadowed.has(cur.name)) {
        const skip = (parent?.type === 'MemberExpression' && key === 'property' && !parent.computed)
          || (parent?.type === 'JSXMemberExpression' && key === 'property')
          || (parent?.type === 'Property' && key === 'key' && !parent.computed && !parent.shorthand)
          || (parent?.type === 'TSPropertySignature' && key === 'key' && !parent.computed)
          || (parent?.type === 'MethodDefinition' && key === 'key' && !parent.computed)
          || (parent?.type === 'PropertyDefinition' && key === 'key' && !parent.computed)
          || (parent?.type === 'JSXAttribute' && key === 'name')
          || (parent?.type === 'ImportSpecifier' || parent?.type === 'ImportDefaultSpecifier' || parent?.type === 'ImportNamespaceSpecifier')
          || (parent?.type === 'ExportSpecifier' && key === 'exported')
          || ((parent?.type === 'FunctionDeclaration' || parent?.type === 'ClassDeclaration'
            || parent?.type === 'TSTypeAliasDeclaration' || parent?.type === 'TSInterfaceDeclaration'
            || parent?.type === 'TSEnumDeclaration') && key === 'id')
          || (parent?.type === 'VariableDeclarator' && key === 'id')
          || (parent?.type === 'TSTypeParameter' && key === 'name')
          || (parent?.type === 'LabeledStatement' && key === 'label')
          || ((parent?.type === 'BreakStatement' || parent?.type === 'ContinueStatement') && key === 'label');

        if (!skip) {
          if (cur.type === 'JSXIdentifier' && /^[a-z]/.test(cur.name)) {
          } else if (nextInType) {
            stmt.typeRefs.add(cur.name);
          } else {
            stmt.refs.add(cur.name);
          }

          if (nsBindings.has(cur.name)) {
            const ns = nsBindings.get(cur.name);
            const isType = nextInType || ns.isType;
            if (parent?.type === 'MemberExpression' && key === 'object') {
              const name = !parent.computed && parent.property?.type === 'Identifier' ? parent.property.name : '*';
              stmt.nsUses.push({ targetRel: ns.targetRel, name, isType });
            } else if (parent?.type === 'JSXMemberExpression' && key === 'object') {
              const name = parent.property?.type === 'JSXIdentifier' ? parent.property.name : '*';
              stmt.nsUses.push({ targetRel: ns.targetRel, name, isType: false });
            } else if (parent?.type === 'TSQualifiedName' && key === 'left') {
              const name = parent.right?.type === 'Identifier' ? parent.right.name : '*';
              stmt.nsUses.push({ targetRel: ns.targetRel, name, isType: true });
            } else if (parent?.type === 'VariableDeclarator' && key === 'init' && parent.id?.type === 'ObjectPattern') {
              for (const prop of parent.id.properties) {
                if (prop.type === 'Property' && !prop.computed && prop.key?.type === 'Identifier') {
                  stmt.nsUses.push({ targetRel: ns.targetRel, name: prop.key.name, isType });
                } else {
                  stmt.nsUses.push({ targetRel: ns.targetRel, name: '*', isType });
                }
              }
            } else {
              stmt.nsUses.push({ targetRel: ns.targetRel, name: '*', isType });
            }
          }
        }
      }

      for (const [childKey, value] of Object.entries(cur)) {
        if (Array.isArray(value)) {
          for (const el of value) {
            if (el && typeof el === 'object' && typeof el.type === 'string') {
              walkStmt(el, cur, childKey, nextShadowed, nextInType, nextBlock);
            }
          }
        } else if (value && typeof value === 'object' && typeof value.type === 'string') {
          walkStmt(value, cur, childKey, nextShadowed, nextInType, nextBlock);
        }
      }
    };

    walkStmt(node, null, null, new Set(), stmt.isTypeOnly, node);

    const parentMap = new Map();
    if (stmt.dynamicUses.some(d => d.pendingAwaitLoader || d.pendingThenLoader)) {
      walkAst(node, (n, p, k) => { parentMap.set(n, { parent: p, key: k }); });
      for (const d of stmt.dynamicUses) {
        if (d.pendingAwaitLoader) {
          const { awaitNode, block } = d.pendingAwaitLoader;
          const info = parentMap.get(awaitNode);
          const p = info?.parent;
          const k = info?.key;
          if (p?.type === 'MemberExpression' && k === 'object') {
            d.names = !p.computed && p.property?.type === 'Identifier' ? [p.property.name] : ['*'];
          } else if (p?.type === 'VariableDeclarator' && k === 'init' && p.id?.type === 'ObjectPattern') {
            const names = [];
            let ok = true;
            for (const prop of p.id.properties) {
              if (prop.type === 'Property' && !prop.computed && prop.key?.type === 'Identifier') names.push(prop.key.name);
              else { ok = false; break; }
            }
            d.names = ok ? names : ['*'];
          } else if (p?.type === 'VariableDeclarator' && k === 'init' && p.id?.type === 'Identifier') {
            const { used, opaque } = collectIdentifierMemberUsage(block, p.id.name);
            d.names = opaque ? ['*'] : [...used];
          } else {
            d.names = ['*'];
          }
          delete d.pendingAwaitLoader;
        } else if (d.pendingThenLoader) {
          const info = parentMap.get(d.pendingThenLoader.memberNode);
          const p = info?.parent;
          if (p?.type === 'CallExpression') {
            d.names = extractCallbackParamUsage(p.arguments[0]);
          } else {
            d.names = ['*'];
          }
          delete d.pendingThenLoader;
        }
      }
    }
  }

  for (const [, loader] of localLoaders) {
    if (loader.exported || loader.opaqueEscape) {
      const stmt = statements[loader.stmtIndex];
      if (stmt) stmt.dynamicUses.push({ targetRel: loader.targetRel, names: ['*'], legacyLoad: false });
    }
  }

  const bindingToStmts = new Map();
  for (const stmt of statements) {
    for (const name of stmt.bindings) {
      let list = bindingToStmts.get(name);
      if (!list) bindingToStmts.set(name, list = []);
      list.push(stmt.stmtIndex);
    }
  }

  const result = {
    relPath,
    declaredExports,
    reexports,
    starReexports,
    importedBindings,
    nsBindings,
    bareImports,
    workerUrls,
    statements,
    bindingToStmts,
  };
  cache.set(relPath, result);
  return result;
}

function moduleProvidesExport(root, relPath, exportName, moduleCache, visiting = new Set()) {
  if (visiting.has(relPath)) return false;
  visiting.add(relPath);
  const info = analyzeScriptModule(root, relPath, moduleCache);
  if (!info) return false;
  if (info.declaredExports.has(exportName)) return true;
  for (const star of info.starReexports) {
    if (moduleProvidesExport(root, star.targetRel, exportName, moduleCache, visiting)) return true;
  }
  return false;
}

function readRatchetFile(root) {
  const ratchetPath = resolve(root, 'tools/legacy-widgets-ratchet.json');
  const data = JSON.parse(readFileSync(ratchetPath, 'utf8'));
  const required = ['maxLegacyViews', 'maxLegacyRendererViews', 'maxUnusedExports'];
  for (const field of required) {
    const val = data[field];
    if (typeof val !== 'number' || !Number.isInteger(val) || val < 0) {
      throw new Error(`tools/legacy-widgets-ratchet.json must set integer ${field} >= 0`);
    }
  }
  return data;
}

function resolveImportFromFunction(fn, fromFile, root) {
  if (typeof fn !== 'function') return null;
  try {
    const ast = parseAst(`(${fn.toString()})`, { lang: 'ts' });
    let target = null;
    walkAst(ast, inner => {
      if (target || inner.type !== 'ImportExpression') return;
      const spec = staticStringLiteral(inner.source);
      if (spec === null) return;
      const resolved = resolveLocalImport(fromFile, spec, root);
      if (resolved) target = relPosix(root, resolved);
    });
    return target;
  } catch {
    return null;
  }
}

function parseLegacyManifestAstFallback(legacyPath, root) {
  const byKey = new Map();
  if (!existsSync(legacyPath)) return byKey;
  const ast = parseAst(readFileSync(legacyPath, 'utf8'), { lang: 'ts' });
  walkAst(ast, node => {
    if (node.type !== 'ObjectExpression') return;
    let kind = null;
    let view = '';
    let loadTarget = null;
    for (const prop of node.properties) {
      if (prop.type !== 'Property') continue;
      const propName = prop.key?.name ?? prop.key?.value;
      if (propName === 'kind') kind = staticStringLiteral(prop.value);
      else if (propName === 'view') view = staticStringLiteral(prop.value) ?? '';
      else if (propName === 'load') {
        walkAst(prop.value, inner => {
          if (loadTarget || inner.type !== 'ImportExpression') return;
          const spec = staticStringLiteral(inner.source);
          if (spec === null) return;
          const resolved = resolveLocalImport(legacyPath, spec, root);
          if (resolved) loadTarget = relPosix(root, resolved);
        });
      }
    }
    if (kind && loadTarget) byKey.set(`${kind}\0${view}`, loadTarget);
  });
  return byKey;
}

async function loadManifestData(root) {
  const metaByKind = new Map();
  const familyViews = [];
  const familyByKind = new Map();
  const legacyEntries = [];
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
      const loadTarget = resolveImportFromFunction(entry.load, legacyPath, root)
        ?? astFallback.get(`${entry.kind}\0${view ?? ''}`)
        ?? null;
      legacyEntries.push({
        kind: entry.kind,
        view,
        loadTarget,
        hasLoader: typeof entry.load === 'function',
      });
    }
  }

  const manifestPath = resolve(root, 'ui/src/widgets/manifest.ts');
  if (existsSync(manifestPath)) {
    const ast = parseAst(readFileSync(manifestPath, 'utf8'), { lang: 'ts' });
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
          let target = null;
          walkAst(prop.value, inner => {
            if (target || inner.type !== 'ImportExpression') return;
            const spec = staticStringLiteral(inner.source);
            if (spec === null) return;
            const resolved = resolveLocalImport(manifestPath, spec, root);
            if (resolved) target = relPosix(root, resolved);
          });
          astUiTargets.set(kind, target);
        }
      }
    }
    try {
      const mod = await import(`${pathToFileURL(manifestPath).href}?t=${stamp}`);
      if (mod.FAMILY_VIEW_UI && typeof mod.FAMILY_VIEW_UI === 'object') {
        for (const [kind, loader] of Object.entries(mod.FAMILY_VIEW_UI)) {
          const target = resolveImportFromFunction(loader?.load, manifestPath, root) ?? astUiTargets.get(kind) ?? null;
          familyViewUiEntries.set(kind, target);
          if (Array.isArray(loader?.views)) familyViewUiViews.set(kind, loader.views);
        }
      }
    } catch {
      for (const [kind, target] of astUiTargets) familyViewUiEntries.set(kind, target);
    }
  }

  return {
    metaByKind,
    familyViews,
    familyByKind,
    legacyFileExists,
    legacyEntries,
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

    if (entry.hasLoader && !entry.loadTarget) {
      unresolvedLoaders.push(label);
    }

    const covered = familyByKind.get(entry.kind);
    if (explicitView) {
      if (covered?.has(explicitView)) {
        staleEntries.push(label);
        continue;
      }
      addPair(entry.kind, explicitView);
      if (entry.loadTarget) activeLegacyTargets.add(entry.loadTarget);
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
      } else if (entry.loadTarget) {
        activeLegacyTargets.add(entry.loadTarget);
      }
    } else {
      if (covered?.has('default')) {
        staleEntries.push(label);
      } else {
        addPair(entry.kind, 'default');
        if (entry.loadTarget) activeLegacyTargets.add(entry.loadTarget);
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
  const moduleCache = new Map();
  const runtimeLive = new Set();
  const typeOnlyLive = new Set();
  const runtimeDemand = new Map();
  const typeDemand = new Map();
  const liveRuntimeStmts = new Map();
  const liveTypeStmts = new Map();
  const entryModules = new Set();
  const productionTestImports = [];
  const seenProdTestImport = new Set();

  const ensureSet = (map, key) => {
    let set = map.get(key);
    if (!set) map.set(key, set = new Set());
    return set;
  };

  let changed = false;

  const markRuntimeStmt = (rel, idx) => {
    const set = ensureSet(liveRuntimeStmts, rel);
    if (!set.has(idx)) {
      set.add(idx);
      changed = true;
    }
  };

  const markTypeStmt = (rel, idx) => {
    const set = ensureSet(liveTypeStmts, rel);
    if (!set.has(idx)) {
      set.add(idx);
      changed = true;
    }
  };

  const addModuleEdge = (fromRel, targetRel, isType, demandName) => {
    if (!targetRel) return false;
    const role = roleByRel.get(targetRel);
    if (role === 'test' || role === 'fixture' || role === 'test-support') {
      if (!fromRel || roleByRel.get(fromRel) === 'production') {
        const key = `${fromRel ?? '<root>'}->${targetRel}`;
        if (!seenProdTestImport.has(key)) {
          seenProdTestImport.add(key);
          productionTestImports.push({ from: fromRel ?? '<root>', to: targetRel, role });
        }
      }
      return false;
    }
    let added = false;
    if (!isType) {
      if (!runtimeLive.has(targetRel)) {
        runtimeLive.add(targetRel);
        added = true;
      }
      if (demandName) {
        const set = ensureSet(runtimeDemand, targetRel);
        if (!set.has(demandName)) {
          set.add(demandName);
          added = true;
        }
      }
    } else {
      if (!typeOnlyLive.has(targetRel)) {
        typeOnlyLive.add(targetRel);
        added = true;
      }
      if (demandName) {
        const set = ensureSet(typeDemand, targetRel);
        if (!set.has(demandName)) {
          set.add(demandName);
          added = true;
        }
      }
    }
    if (added) changed = true;
    return added;
  };

  const cssVisited = new Set();
  const seedCss = cssRel => {
    if (cssVisited.has(cssRel)) return;
    cssVisited.add(cssRel);
    const full = resolve(root, cssRel);
    if (!existsSync(full)) return;
    for (const spec of extractImportSpecifiers(full)) {
      const target = resolveLocalImport(full, spec.specifier, root);
      if (target) {
        const targetRel = relPosix(root, target);
        addModuleEdge(cssRel, targetRel, false, null);
        if (targetRel.endsWith('.css')) seedCss(targetRel);
      }
    }
  };

  const seedEntryModule = targetRel => {
    if (!targetRel) return;
    if (targetRel.endsWith('.css')) {
      addModuleEdge(null, targetRel, false, null);
      seedCss(targetRel);
      return;
    }
    if (!addModuleEdge(null, targetRel, false, null) && entryModules.has(targetRel)) return;
    entryModules.add(targetRel);
    const info = analyzeScriptModule(root, targetRel, moduleCache);
    if (!info) return;
    for (const stmt of info.statements) {
      if (stmt.isTypeOnly) markTypeStmt(targetRel, stmt.stmtIndex);
      else markRuntimeStmt(targetRel, stmt.stmtIndex);
    }
  };

  for (const htmlRel of entryPoints) {
    const full = resolve(root, htmlRel);
    if (!existsSync(full)) continue;
    for (const spec of extractImportSpecifiers(full)) {
      const target = resolveLocalImport(full, spec.specifier, root);
      if (target) seedEntryModule(relPosix(root, target));
    }
  }

  const viteConfigPath = resolve(root, 'ui/vite.config.ts');
  if (existsSync(viteConfigPath)) {
    const buildWalk = walkImportGraph(['ui/vite.config.ts'], root, {
      includeTypes: true,
      includeDynamic: true,
      includeWorkerUrls: true,
      shouldTraverse: file => !relPosix(root, file).startsWith('ui/src/'),
    });
    for (const file of buildWalk.visited) {
      const rel = relPosix(root, file);
      if (rel.startsWith('ui/src/')) continue;
      const info = analyzeScriptModule(root, rel, moduleCache);
      if (!info) continue;
      for (const stmt of info.statements) {
        for (const ref of stmt.refs) {
          const imp = info.importedBindings.get(ref);
          if (imp && imp.targetRel.startsWith('ui/src/')) {
            addModuleEdge(rel, imp.targetRel, imp.isType, imp.importedName);
          }
        }
        for (const tref of stmt.typeRefs) {
          const imp = info.importedBindings.get(tref);
          if (imp && imp.targetRel.startsWith('ui/src/')) {
            addModuleEdge(rel, imp.targetRel, true, imp.importedName);
          }
        }
        for (const nsUse of stmt.nsUses) {
          if (nsUse.targetRel.startsWith('ui/src/')) {
            addModuleEdge(rel, nsUse.targetRel, nsUse.isType, nsUse.name);
          }
        }
      }
    }
  }

  const processedRuntimeInit = new Set();
  const processedRuntimeStmts = new Map();
  const processedTypeStmts = new Map();

  changed = true;
  while (changed) {
    changed = false;

    for (const rel of [...runtimeLive]) {
      if (rel.endsWith('.css')) {
        seedCss(rel);
        continue;
      }
      const info = analyzeScriptModule(root, rel, moduleCache);
      if (!info) continue;

      if (!processedRuntimeInit.has(rel)) {
        processedRuntimeInit.add(rel);
        for (const bare of info.bareImports) {
          addModuleEdge(rel, bare.targetRel, bare.isType, null);
        }
        for (const stmt of info.statements) {
          if (stmt.isSideEffect) markRuntimeStmt(rel, stmt.stmtIndex);
        }
        if (rel === 'ui/src/widgets/legacyManifest.ts' && activeLegacyTargets) {
          for (const targetRel of activeLegacyTargets) {
            addModuleEdge(rel, targetRel, false, 'default');
          }
        }
      }

      const req = runtimeDemand.get(rel);
      if (req) {
        const allWanted = req.has('*');
        for (const [expName, meta] of info.declaredExports) {
          if (!allWanted && !req.has(expName)) continue;
          if (meta.isReexport) continue;
          if (meta.isType) {
            markTypeStmt(rel, meta.stmtIndex);
            if (meta.specifierOnly && meta.localName) {
              for (const idx of info.bindingToStmts.get(meta.localName) ?? []) markTypeStmt(rel, idx);
            }
          } else {
            markRuntimeStmt(rel, meta.stmtIndex);
            if (meta.specifierOnly && meta.localName) {
              for (const idx of info.bindingToStmts.get(meta.localName) ?? []) markRuntimeStmt(rel, idx);
            }
          }
        }
        for (const re of info.reexports) {
          if (allWanted || req.has(re.exported)) {
            addModuleEdge(rel, re.targetRel, re.isType, re.targetName);
          }
        }
        for (const star of info.starReexports) {
          if (allWanted) {
            addModuleEdge(rel, star.targetRel, star.isType, '*');
          } else {
            for (const name of req) {
              if (name !== 'default' && !info.declaredExports.has(name) && moduleProvidesExport(root, star.targetRel, name, moduleCache)) {
                addModuleEdge(rel, star.targetRel, star.isType, name);
              }
            }
          }
        }
      }

      const doneStmts = ensureSet(processedRuntimeStmts, rel);
      for (const idx of [...(liveRuntimeStmts.get(rel) ?? [])]) {
        if (doneStmts.has(idx)) continue;
        doneStmts.add(idx);
        const stmt = info.statements[idx];
        if (!stmt) continue;
        for (const ref of stmt.refs) {
          for (const depIdx of info.bindingToStmts.get(ref) ?? []) {
            const depStmt = info.statements[depIdx];
            if (depStmt?.isTypeOnly) markTypeStmt(rel, depIdx);
            else markRuntimeStmt(rel, depIdx);
          }
          const imp = info.importedBindings.get(ref);
          if (imp) addModuleEdge(rel, imp.targetRel, imp.isType, imp.importedName);
        }
        for (const tref of stmt.typeRefs) {
          for (const depIdx of info.bindingToStmts.get(tref) ?? []) markTypeStmt(rel, depIdx);
          const imp = info.importedBindings.get(tref);
          if (imp) addModuleEdge(rel, imp.targetRel, true, imp.importedName);
        }
        for (const nsUse of stmt.nsUses) {
          addModuleEdge(rel, nsUse.targetRel, nsUse.isType, nsUse.name);
        }
        for (const dyn of stmt.dynamicUses) {
          if (dyn.legacyLoad) continue;
          for (const name of dyn.names) addModuleEdge(rel, dyn.targetRel, false, name);
        }
        for (const inl of stmt.inlineTypeUses) {
          addModuleEdge(rel, inl.targetRel, true, inl.name);
        }
        for (const w of info.workerUrls) {
          if (w.stmtIndex === idx) seedEntryModule(w.targetRel);
        }
      }
    }

    for (const rel of new Set([...typeOnlyLive, ...runtimeLive])) {
      const info = analyzeScriptModule(root, rel, moduleCache);
      if (!info) continue;

      const req = typeDemand.get(rel);
      if (req) {
        const allWanted = req.has('*');
        for (const [expName, meta] of info.declaredExports) {
          if (!allWanted && !req.has(expName)) continue;
          if (meta.isReexport) continue;
          markTypeStmt(rel, meta.stmtIndex);
          if (meta.specifierOnly && meta.localName) {
            for (const idx of info.bindingToStmts.get(meta.localName) ?? []) markTypeStmt(rel, idx);
          }
        }
        for (const re of info.reexports) {
          if (allWanted || req.has(re.exported)) {
            addModuleEdge(rel, re.targetRel, true, re.targetName);
          }
        }
        for (const star of info.starReexports) {
          if (allWanted) {
            addModuleEdge(rel, star.targetRel, true, '*');
          } else {
            for (const name of req) {
              if (name !== 'default' && !info.declaredExports.has(name) && moduleProvidesExport(root, star.targetRel, name, moduleCache)) {
                addModuleEdge(rel, star.targetRel, true, name);
              }
            }
          }
        }
      }

      const doneStmts = ensureSet(processedTypeStmts, rel);
      for (const idx of [...(liveTypeStmts.get(rel) ?? [])]) {
        if (doneStmts.has(idx)) continue;
        doneStmts.add(idx);
        const stmt = info.statements[idx];
        if (!stmt) continue;
        for (const ref of [...stmt.refs, ...stmt.typeRefs]) {
          for (const depIdx of info.bindingToStmts.get(ref) ?? []) markTypeStmt(rel, depIdx);
          const imp = info.importedBindings.get(ref);
          if (imp) addModuleEdge(rel, imp.targetRel, true, imp.importedName);
        }
        for (const nsUse of stmt.nsUses) {
          addModuleEdge(rel, nsUse.targetRel, true, nsUse.name);
        }
        for (const inl of stmt.inlineTypeUses) {
          addModuleEdge(rel, inl.targetRel, true, inl.name);
        }
      }
    }
  }

  const liveProd = new Set();
  for (const rel of runtimeLive) {
    if (roleByRel.has(rel)) liveProd.add(rel);
  }
  for (const rel of typeOnlyLive) {
    if (roleByRel.has(rel)) liveProd.add(rel);
  }

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

  const unusedExports = [];
  for (const rel of [...liveProd].sort()) {
    if (rel.startsWith('ui/src/generated/') || entryModules.has(rel)) continue;
    const info = analyzeScriptModule(root, rel, moduleCache);
    if (!info) continue;
    const rReq = runtimeDemand.get(rel);
    const tReq = typeDemand.get(rel);
    if (rReq?.has('*') || tReq?.has('*')) continue;

    const liveStmts = new Set([...(liveRuntimeStmts.get(rel) ?? []), ...(liveTypeStmts.get(rel) ?? [])]);
    for (const [expName, meta] of [...info.declaredExports.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
      if (rReq?.has(expName) || tReq?.has(expName)) continue;
      if (meta.isReexport) {
        unusedExports.push({ file: rel, name: expName, key: `${rel}:${expName}`, isType: meta.isType, isReexport: true });
        continue;
      }
      if (meta.specifierOnly) {
        const localLive = Boolean(meta.localName && (info.bindingToStmts.get(meta.localName) ?? []).some(idx => liveStmts.has(idx)));
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

  const exportErrors = [];
  const exportNotices = [];
  if (unusedExports.length > maxUnusedExports) {
    exportErrors.push(`${unusedExports.length} unused export(s) in ui/src, above ratchet ${maxUnusedExports}`);
  } else if (unusedExports.length > 0 && unusedExports.length < maxUnusedExports) {
    exportNotices.push(`unused exports in ui/src dropped to ${unusedExports.length} (ratchet is ${maxUnusedExports}); lower maxUnusedExports in tools/legacy-widgets-ratchet.json to ${unusedExports.length}`);
  } else if (unusedExports.length === 0 && maxUnusedExports !== 0) {
    exportErrors.push(`no unused exports remain in ui/src; lower maxUnusedExports in tools/legacy-widgets-ratchet.json from ${maxUnusedExports} to 0`);
  }

  productionTestImports.sort((a, b) => `${a.from}->${a.to}`.localeCompare(`${b.from}->${b.to}`));

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

  const pushEntry = (kind, view, family, specTarget) => {
    const pair = `${kind}:${view}`;
    if (!byPair.has(pair)) {
      byPair.set(pair, { kind, view, family, specTarget });
    }
  };

  if (existsSync(dir)) {
    const files = readdirSync(dir).filter(name => name.endsWith('.ts') && !name.endsWith('.test.ts')).sort();

    for (const file of files) {
      const full = resolve(dir, file);
      const family = file.slice(0, -'.ts'.length);
      const ast = parseAst(readFileSync(full, 'utf8'), { lang: 'ts' });

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

      const firstImportIn = node => {
        let target = null;
        walkAst(node, inner => {
          if (target || inner.type !== 'ImportExpression') return;
          const spec = staticStringLiteral(inner.source);
          if (spec === null) return;
          const resolved = resolveLocalImport(full, spec, root);
          if (resolved) target = relPosix(root, resolved);
        });
        return target;
      };

      const record = (kind, view, specTarget) => {
        if (!kind || !specTarget) return;
        if (view === '*') {
          const concrete = runtimeViewsByKindFamily.get(`${kind}\0${family}`) ?? [];
          for (const v of concrete) pushEntry(kind, v, family, specTarget);
        } else {
          pushEntry(kind, view, family, specTarget);
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
          let specTarget = null;
          for (const prop of unwrapped.properties) {
            if (prop.type !== 'Property') continue;
            const propName = prop.key?.name ?? prop.key?.value;
            if (propName === 'kind') {
              kind = resolveStringExpr(prop.value, env);
            } else if (propName === 'view') {
              const resolvedView = resolveStringExpr(prop.value, env);
              view = resolvedView !== null ? resolvedView : '*';
            } else if (propName === 'loadSpec') {
              specTarget = firstImportIn(prop.value);
            }
          }
          record(kind, view, specTarget);
        } else if (unwrapped.type === 'CallExpression' && unwrapped.arguments.length >= 2) {
          const kind = resolveStringExpr(unwrapped.arguments[0], env);
          if (!kind) return;
          const specTarget = firstImportIn(unwrapped);
          const resolvedView = resolveStringExpr(unwrapped.arguments[1], env);
          const view = resolvedView !== null ? resolvedView : 'default';
          record(kind, view, specTarget);
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
    if (hit?.specTarget) {
      resolved.push(hit);
    } else {
      const fromFile = entry.family ? resolve(root, 'ui/src/widgets/manifest', `${entry.family}.ts`) : resolve(root, 'ui/src/widgets/familyManifest.ts');
      const fromFn = resolveImportFromFunction(entry.loadSpec, fromFile, root);
      if (fromFn) {
        resolved.push({ kind: entry.kind, view, family: entry.family ?? 'unknown', specTarget: fromFn });
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

  const errors = [];
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

function importsReactAtRuntime(root, relPath) {
  const full = resolve(root, relPath);
  if (!existsSync(full)) return false;
  const specs = extractImportSpecifiers(full, { includeTypes: false, includeDynamic: true, includeWorkerUrls: false });
  return specs.some(spec => spec.specifier === 'react' || spec.specifier.startsWith('react/'));
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
  for (const [kind, target] of familyViewUi) {
    if (!target) unresolvedUiLoaders.push(kind);
  }
  unresolvedUiLoaders.sort();

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
    if (file.endsWith('.tsx') || importsReactAtRuntime(root, file)) {
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

  const oldRenderer = new Set([...oldWidgetRenderers, ...oldCharts, ...oldDash]);

  const edgeCache = new Map();
  const shortestChain = startFiles => {
    const queue = startFiles.map(file => [file]);
    const seen = new Set(startFiles);
    while (queue.length > 0) {
      const chain = queue.shift();
      const cur = chain[chain.length - 1];
      if (oldRenderer.has(cur)) return chain;
      if (GLOBAL_REGISTRIES.has(cur)) continue;
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
    const uiViews = loaded.familyViewUiViews.get(entry.kind);
    const uiEntry = !uiViews || uiViews.includes(entry.view) ? familyViewUi.get(entry.kind) : undefined;
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

export async function runDeadCodeCheck(root) {
  const ratchetData = readRatchetFile(root);
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
  return { ui, rust, legacy, convertedLegacy, exports, totalIssues };
}

export function classifyAgainstBase(result, baseResult, baseSha = 'base') {
  const shortBase = baseSha.slice(0, 9);
  const baseDeadProd = new Set(baseResult.ui.deadProduction);
  const baseDeadTestTargets = new Map(baseResult.ui.deadTests.map(entry => [entry.test, new Set(entry.targets)]));
  const baseDeadFixtures = new Set(baseResult.ui.deadFixturesAndSupport.map(entry => entry.file));
  const baseProdTestImports = new Set((baseResult.ui.productionTestImports ?? []).map(e => `${e.from}->${e.to}`));
  const baseDeadProducts = new Set(baseResult.rust.deadProducts.map(entry => entry.product));
  const baseUnwired = new Set(baseResult.rust.unwiredRegistrations);
  const baseLegacyPairs = new Set(baseResult.legacy.pairs);
  const baseLegacyErrors = new Set(baseResult.legacy.errors);
  const baseOverRatchet = baseResult.legacy.count > baseResult.legacy.maxLegacyViews;

  const baseConvertedPairs = new Set((baseResult.convertedLegacy?.violations ?? []).map(v => v.pair));
  const baseConvertedErrors = new Set(baseResult.convertedLegacy?.errors ?? []);
  const baseConvertedOver = (baseResult.convertedLegacy?.violations?.length ?? 0) > baseResult.convertedLegacy.maxLegacyRendererViews;

  const baseUnusedExportKeys = new Set((baseResult.exports?.unusedExports ?? []).map(item => item.key));
  const baseExportErrors = new Set(baseResult.exports?.errors ?? []);
  const baseExportsOver = (baseResult.exports?.unusedExports?.length ?? 0) > baseResult.exports.maxUnusedExports;

  const kindOf = pair => pair.slice(0, pair.indexOf(':'));
  const legacyViewsOf = (pairs, kind) => pairs.filter(pair => kindOf(pair) === kind).length;
  const namedDefault = pair => baseLegacyPairs.has(`${kindOf(pair)}:default`) && !result.legacy.pairs.includes(`${kindOf(pair)}:default`)
    && legacyViewsOf(result.legacy.pairs, kindOf(pair)) === 1 && legacyViewsOf([...baseLegacyPairs], kindOf(pair)) === 1;
  const addedLegacyPairs = result.legacy.pairs.filter(pair => !baseLegacyPairs.has(pair) && !namedDefault(pair));
  const addedConvertedViolations = result.convertedLegacy.violations.filter(v => !baseConvertedPairs.has(v.pair));
  const addedUnusedExports = result.exports.unusedExports.filter(item => !baseUnusedExportKeys.has(item.key));

  const ratchetRaises = [];
  if (result.legacy.maxLegacyViews > baseResult.legacy.maxLegacyViews) {
    ratchetRaises.push(`maxLegacyViews raised from ${baseResult.legacy.maxLegacyViews} to ${result.legacy.maxLegacyViews}`);
  }
  if (result.convertedLegacy.maxLegacyRendererViews > baseResult.convertedLegacy.maxLegacyRendererViews) {
    ratchetRaises.push(`maxLegacyRendererViews raised from ${baseResult.convertedLegacy.maxLegacyRendererViews} to ${result.convertedLegacy.maxLegacyRendererViews}`);
  }
  if (result.exports.maxUnusedExports > baseResult.exports.maxUnusedExports) {
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

export function materializeCommitSnapshot(root, sha, destDir) {
  const treeOut = execFileSync(
    'git',
    ['ls-tree', '-r', '-z', '--name-only', sha],
    { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'] },
  );
  const files = treeOut.split('\0').filter(Boolean);
  const activeSpecs = BASE_SNAPSHOT_RULES
    .filter(rule => files.some(rule.match))
    .map(rule => rule.pathspec);
  if (activeSpecs.length === 0) return;
  const tarPath = join(destDir, '__base.tar');
  execFileSync(
    'git',
    ['archive', '--output', tarPath, sha, '--', ...activeSpecs],
    { cwd: root, stdio: ['ignore', 'ignore', 'pipe'] },
  );
  execFileSync('tar', ['-xf', '__base.tar'], { cwd: destDir, stdio: ['ignore', 'ignore', 'pipe'] });
  rmSync(tarPath, { force: true });
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
  if (!noBase) {
    const baseSha = resolveBaseCommit(root, againstArg);
    if (baseSha) {
      const baseResult = await runCheckAtCommit(root, baseSha);
      comparison = classifyAgainstBase(result, baseResult, baseSha);
    }
  }
  const body = formatReport(result, comparison, { showAllInFile: listInFile });
  if (body) console.log(body);
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
