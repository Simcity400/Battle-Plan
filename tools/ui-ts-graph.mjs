import { existsSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, extname, join, resolve } from 'node:path';
import { API, SignatureKind, SymbolFlags, TypeFlags } from 'typescript/unstable/sync';
import { NodeFlags, SyntaxKind } from 'typescript/unstable/ast';
import { extractImportSpecifiers, relPosix, resolveLocalImport, toPosix } from './ui-import-graph.mjs';

const PURE_NEW_CONSTRUCTORS = new Set([
  'Set', 'Map', 'WeakSet', 'WeakMap', 'RegExp', 'URL', 'URLSearchParams',
  'Error', 'TypeError', 'RangeError', 'ArrayBuffer', 'DataView',
  'Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array', 'Uint16Array',
  'Int32Array', 'Uint32Array', 'Float32Array', 'Float64Array', 'BigInt64Array', 'BigUint64Array',
  'TextEncoder', 'TextDecoder',
]);

const PURE_OBJECT_METHODS = new Set(['freeze', 'seal', 'create', 'fromEntries']);
const PURE_SYMBOL_METHODS = new Set(['for']);

const TYPE_NODE_KINDS = new Set([
  SyntaxKind.TypeReference,
  SyntaxKind.TypeLiteral,
  SyntaxKind.FunctionType,
  SyntaxKind.ConstructorType,
  SyntaxKind.TypeQuery,
  SyntaxKind.ImportType,
  SyntaxKind.IndexedAccessType,
  SyntaxKind.MappedType,
  SyntaxKind.ConditionalType,
  SyntaxKind.TupleType,
  SyntaxKind.ArrayType,
  SyntaxKind.UnionType,
  SyntaxKind.IntersectionType,
  SyntaxKind.ParenthesizedType,
  SyntaxKind.TypeOperator,
  SyntaxKind.TypePredicate,
  SyntaxKind.InferType,
  SyntaxKind.TemplateLiteralType,
  SyntaxKind.NamedTupleMember,
  SyntaxKind.OptionalType,
  SyntaxKind.RestType,
  SyntaxKind.InterfaceDeclaration,
  SyntaxKind.TypeAliasDeclaration,
  SyntaxKind.TypeParameter,
]);

const ASSIGNMENT_OPERATORS = new Set([
  SyntaxKind.EqualsToken,
  SyntaxKind.PlusEqualsToken,
  SyntaxKind.MinusEqualsToken,
  SyntaxKind.AsteriskAsteriskEqualsToken,
  SyntaxKind.AsteriskEqualsToken,
  SyntaxKind.SlashEqualsToken,
  SyntaxKind.PercentEqualsToken,
  SyntaxKind.AmpersandEqualsToken,
  SyntaxKind.BarEqualsToken,
  SyntaxKind.CaretEqualsToken,
  SyntaxKind.LessThanLessThanEqualsToken,
  SyntaxKind.GreaterThanGreaterThanEqualsToken,
  SyntaxKind.GreaterThanGreaterThanGreaterThanEqualsToken,
  SyntaxKind.BarBarEqualsToken,
  SyntaxKind.AmpersandAmpersandEqualsToken,
  SyntaxKind.QuestionQuestionEqualsToken,
]);

const PURE_LITERAL_KINDS = new Set([
  SyntaxKind.NumericLiteral,
  SyntaxKind.StringLiteral,
  SyntaxKind.NoSubstitutionTemplateLiteral,
  SyntaxKind.BigIntLiteral,
  SyntaxKind.RegularExpressionLiteral,
  SyntaxKind.TrueKeyword,
  SyntaxKind.FalseKeyword,
  SyntaxKind.NullKeyword,
  SyntaxKind.UndefinedKeyword,
  SyntaxKind.ThisKeyword,
  SyntaxKind.Identifier,
  SyntaxKind.ArrowFunction,
  SyntaxKind.FunctionExpression,
]);

function isSkippedIdent(node, parent) {
  return (parent?.kind === SyntaxKind.PropertyAccessExpression && parent.name === node)
    || (parent?.kind === SyntaxKind.PropertyAssignment && parent.name === node)
    || (parent?.kind === SyntaxKind.PropertySignature && parent.name === node)
    || (parent?.kind === SyntaxKind.MethodDeclaration && parent.name === node)
    || (parent?.kind === SyntaxKind.PropertyDeclaration && parent.name === node)
    || (parent?.kind === SyntaxKind.GetAccessor && parent.name === node)
    || (parent?.kind === SyntaxKind.SetAccessor && parent.name === node)
    || (parent?.kind === SyntaxKind.JsxAttribute && parent.name === node)
    || (parent?.kind === SyntaxKind.BindingElement && parent.propertyName === node)
    || (parent?.kind === SyntaxKind.BindingElement && parent.name === node)
    || (parent?.kind === SyntaxKind.Parameter && parent.name === node)
    || (parent?.kind === SyntaxKind.VariableDeclaration && parent.name === node)
    || (parent?.kind === SyntaxKind.FunctionDeclaration && parent.name === node)
    || (parent?.kind === SyntaxKind.FunctionExpression && parent.name === node)
    || (parent?.kind === SyntaxKind.ClassDeclaration && parent.name === node)
    || (parent?.kind === SyntaxKind.ClassExpression && parent.name === node)
    || (parent?.kind === SyntaxKind.InterfaceDeclaration && parent.name === node)
    || (parent?.kind === SyntaxKind.TypeAliasDeclaration && parent.name === node)
    || (parent?.kind === SyntaxKind.EnumDeclaration && parent.name === node)
    || (parent?.kind === SyntaxKind.EnumMember && parent.name === node)
    || (parent?.kind === SyntaxKind.TypeParameter && parent.name === node)
    || (parent?.kind === SyntaxKind.LabeledStatement && parent.label === node)
    || (parent?.kind === SyntaxKind.BreakStatement && parent.label === node)
    || (parent?.kind === SyntaxKind.ContinueStatement && parent.label === node)
    || ((parent?.kind === SyntaxKind.JsxOpeningElement || parent?.kind === SyntaxKind.JsxSelfClosingElement || parent?.kind === SyntaxKind.JsxClosingElement)
      && parent.tagName === node && /^[a-z]/.test(node.text));
}

function isDeferredNestedDecl(node, rootNode) {
  if (!node || node === rootNode) return false;
  if (node.kind === SyntaxKind.FunctionDeclaration) {
    return node.name?.kind === SyntaxKind.Identifier && Boolean(node.body);
  }
  if (node.kind === SyntaxKind.VariableDeclaration && node.name?.kind === SyntaxKind.Identifier && node.initializer) {
    const init = unwrapTsExpr(node.initializer);
    return init?.kind === SyntaxKind.ArrowFunction || init?.kind === SyntaxKind.FunctionExpression;
  }
  return false;
}

function extractObjectBindingMembers(pattern, checker) {
  const members = new Set();
  for (const el of pattern.elements ?? []) {
    if (el.dotDotDotToken) return null;
    if (el.propertyName) {
      if (el.propertyName.kind === SyntaxKind.Identifier || el.propertyName.kind === SyntaxKind.StringLiteral) {
        members.add(el.propertyName.text);
      } else if (el.propertyName.kind === SyntaxKind.ComputedPropertyName) {
        const keys = stringLiteralsFromType(checker.getTypeAtLocation(el.propertyName.expression));
        if (!keys) return null;
        for (const k of keys) members.add(k);
      } else {
        return null;
      }
    } else if (el.name?.kind === SyntaxKind.Identifier) {
      members.add(el.name.text);
    } else {
      return null;
    }
  }
  return members;
}

function unwrapTsExpr(node) {
  let cur = node;
  while (cur && (
    cur.kind === SyntaxKind.ParenthesizedExpression
    || cur.kind === SyntaxKind.AsExpression
    || cur.kind === SyntaxKind.SatisfiesExpression
    || cur.kind === SyntaxKind.TypeAssertionExpression
    || cur.kind === SyntaxKind.NonNullExpression
  )) {
    cur = cur.expression;
  }
  return cur;
}

function hasModifier(node, kind) {
  return Boolean(node?.modifiers?.some(mod => mod.kind === kind));
}

function isPureCall(node) {
  const callee = unwrapTsExpr(node.expression);
  if (!callee) return false;
  if (callee.kind === SyntaxKind.Identifier && callee.text === 'Symbol') {
    return (node.arguments ?? []).every(arg => !isSideEffectingExpr(arg));
  }
  if (callee.kind === SyntaxKind.PropertyAccessExpression && callee.expression?.kind === SyntaxKind.Identifier) {
    const obj = callee.expression.text;
    const prop = callee.name?.text;
    if ((obj === 'Object' && PURE_OBJECT_METHODS.has(prop)) || (obj === 'Symbol' && PURE_SYMBOL_METHODS.has(prop))) {
      return (node.arguments ?? []).every(arg => !isSideEffectingExpr(arg));
    }
  }
  return false;
}

function isPureNew(node) {
  const callee = unwrapTsExpr(node.expression);
  if (callee?.kind === SyntaxKind.Identifier && PURE_NEW_CONSTRUCTORS.has(callee.text)) {
    return (node.arguments ?? []).every(arg => !isSideEffectingExpr(arg));
  }
  return false;
}

function classHasSideEffects(node) {
  if (node.modifiers?.some(m => m.kind === SyntaxKind.Decorator)) return true;
  for (const clause of node.heritageClauses ?? []) {
    if (clause.token === SyntaxKind.ExtendsKeyword) {
      for (const t of clause.types ?? []) {
        if (isSideEffectingExpr(t.expression)) return true;
      }
    }
  }
  for (const member of node.members ?? []) {
    if (member.kind === SyntaxKind.ClassStaticBlockDeclaration) return true;
    if (member.kind === SyntaxKind.PropertyDeclaration && hasModifier(member, SyntaxKind.StaticKeyword) && member.initializer) {
      if (isSideEffectingExpr(member.initializer)) return true;
    }
  }
  return false;
}

function isSideEffectingExpr(node) {
  const expr = unwrapTsExpr(node);
  if (!expr) return false;
  if (PURE_LITERAL_KINDS.has(expr.kind)) return false;
  if (expr.kind === SyntaxKind.ClassExpression) return classHasSideEffects(expr);
  if (expr.kind === SyntaxKind.ArrayLiteralExpression) {
    return (expr.elements ?? []).some(el => el && isSideEffectingExpr(el));
  }
  if (expr.kind === SyntaxKind.SpreadElement || expr.kind === SyntaxKind.SpreadAssignment) {
    return isSideEffectingExpr(expr.expression);
  }
  if (expr.kind === SyntaxKind.ObjectLiteralExpression) {
    return (expr.properties ?? []).some(prop => {
      if (prop.kind === SyntaxKind.SpreadAssignment) return isSideEffectingExpr(prop.expression);
      if (prop.kind === SyntaxKind.ShorthandPropertyAssignment) return false;
      if (prop.name?.kind === SyntaxKind.ComputedPropertyName && isSideEffectingExpr(prop.name.expression)) return true;
      if (prop.kind === SyntaxKind.PropertyAssignment) return isSideEffectingExpr(prop.initializer);
      return false;
    });
  }
  if (expr.kind === SyntaxKind.TemplateExpression) {
    return (expr.templateSpans ?? []).some(span => isSideEffectingExpr(span.expression));
  }
  if (expr.kind === SyntaxKind.PrefixUnaryExpression) {
    if (expr.operator === SyntaxKind.PlusPlusToken || expr.operator === SyntaxKind.MinusMinusToken) return true;
    return isSideEffectingExpr(expr.operand);
  }
  if (expr.kind === SyntaxKind.PostfixUnaryExpression) return true;
  if (expr.kind === SyntaxKind.TypeOfExpression || expr.kind === SyntaxKind.VoidExpression) {
    return isSideEffectingExpr(expr.expression);
  }
  if (expr.kind === SyntaxKind.BinaryExpression) {
    if (ASSIGNMENT_OPERATORS.has(expr.operatorToken?.kind)) return true;
    return isSideEffectingExpr(expr.left) || isSideEffectingExpr(expr.right);
  }
  if (expr.kind === SyntaxKind.ConditionalExpression) {
    return isSideEffectingExpr(expr.condition) || isSideEffectingExpr(expr.whenTrue) || isSideEffectingExpr(expr.whenFalse);
  }
  if (expr.kind === SyntaxKind.PropertyAccessExpression) {
    return isSideEffectingExpr(expr.expression);
  }
  if (expr.kind === SyntaxKind.ElementAccessExpression) {
    return isSideEffectingExpr(expr.expression) || isSideEffectingExpr(expr.argumentExpression);
  }
  if (expr.kind === SyntaxKind.NewExpression) return !isPureNew(expr);
  if (expr.kind === SyntaxKind.CallExpression) return !isPureCall(expr);
  return true;
}

function collectPatternNames(pattern, out) {
  if (!pattern) return;
  if (pattern.kind === SyntaxKind.Identifier) {
    out.push(pattern.text);
    return;
  }
  if (pattern.kind === SyntaxKind.ObjectBindingPattern || pattern.kind === SyntaxKind.ArrayBindingPattern) {
    for (const el of pattern.elements ?? []) {
      if (el?.name) collectPatternNames(el.name, out);
    }
  }
}

function nodeKey(node) {
  return `${node.pos}:${node.end}:${node.kind}`;
}

function staticTsString(node) {
  const expr = unwrapTsExpr(node);
  if (!expr) return null;
  if (expr.kind === SyntaxKind.StringLiteral || expr.kind === SyntaxKind.NoSubstitutionTemplateLiteral) {
    return expr.text;
  }
  return null;
}

function isTsImportMetaUrl(node) {
  return node?.kind === SyntaxKind.PropertyAccessExpression
    && node.name?.text === 'url'
    && node.expression?.kind === SyntaxKind.MetaProperty
    && node.expression.keywordToken === SyntaxKind.ImportKeyword;
}

function isTsLazyCall(node) {
  if (node?.kind !== SyntaxKind.CallExpression) return false;
  const callee = unwrapTsExpr(node.expression);
  if (callee?.kind === SyntaxKind.Identifier && callee.text === 'lazy') return true;
  if (callee?.kind === SyntaxKind.PropertyAccessExpression
    && callee.expression?.kind === SyntaxKind.Identifier && callee.expression.text === 'React'
    && callee.name?.text === 'lazy') {
    return true;
  }
  return false;
}

function leftmostQualified(node) {
  let cur = node;
  while (cur && cur.kind === SyntaxKind.QualifiedName) cur = cur.left;
  return cur?.kind === SyntaxKind.Identifier ? cur.text : null;
}

function stringLiteralsFromType(type) {
  if (!type) return null;
  if (type.flags & TypeFlags.StringLiteral) return [type.value];
  if (type.isUnionType()) {
    const out = [];
    for (const part of type.getTypes()) {
      if (part.flags & TypeFlags.StringLiteral) out.push(part.value);
      else return null;
    }
    return out.length > 0 ? out : null;
  }
  return null;
}

function moduleRelFromType(type, modSymIdToRel, checker, depth = 0) {
  if (!type || depth > 4) return null;
  const sym = type.getSymbol();
  if (sym && modSymIdToRel.has(sym.id)) return modSymIdToRel.get(sym.id);
  if (type.isUnionType()) {
    for (const part of type.getTypes()) {
      const hit = moduleRelFromType(part, modSymIdToRel, checker, depth + 1);
      if (hit) return hit;
    }
  }
  return null;
}

function typeInvolvesModule(type, modSymIdToRel, checker, depth = 0) {
  if (!type || depth > 3) return false;
  if (moduleRelFromType(type, modSymIdToRel, checker, depth)) return true;
  if (type.isTypeReference()) {
    const args = checker.getTypeArguments(type);
    for (const arg of args) {
      if (typeInvolvesModule(arg, modSymIdToRel, checker, depth + 1)) return true;
    }
  }
  if (type.isUnionType()) {
    for (const part of type.getTypes()) {
      if (typeInvolvesModule(part, modSymIdToRel, checker, depth + 1)) return true;
    }
  }
  const sigs = checker.getSignaturesOfType(type, SignatureKind.Call);
  for (const sig of sigs) {
    const ret = checker.getReturnTypeOfSignature(sig);
    if (typeInvolvesModule(ret, modSymIdToRel, checker, depth + 1)) return true;
  }
  return false;
}

function unwrapPromiseModuleType(type, modSymIdToRel, checker) {
  if (!type) return null;
  const direct = moduleRelFromType(type, modSymIdToRel, checker);
  if (direct) return direct;
  if (type.isTypeReference()) {
    for (const arg of checker.getTypeArguments(type)) {
      const hit = moduleRelFromType(arg, modSymIdToRel, checker);
      if (hit) return hit;
    }
  }
  return null;
}

function isOpaqueWideningType(type, checker) {
  if (!type) return false;
  if (type.flags & (TypeFlags.Any | TypeFlags.Unknown)) return true;
  const sigs = checker.getSignaturesOfType(type, SignatureKind.Call);
  const target = sigs.length > 0 ? checker.getReturnTypeOfSignature(sigs[0]) : type;
  if (!target) return false;
  if (target.flags & (TypeFlags.Any | TypeFlags.Unknown)) return true;
  if (target.isTypeReference()) {
    const args = checker.getTypeArguments(target);
    if (args.length === 1 && (args[0].flags & (TypeFlags.Any | TypeFlags.Unknown))) return true;
  }
  return false;
}

function isDefaultOnlyContextualType(type, checker) {
  if (!type) return false;
  const sigs = checker.getSignaturesOfType(type, SignatureKind.Call);
  const target = sigs.length > 0 ? checker.getReturnTypeOfSignature(sigs[0]) : type;
  if (!target) return false;
  let inner = target;
  if (target.isTypeReference()) {
    const args = checker.getTypeArguments(target);
    if (args.length === 1) inner = args[0];
  }
  if (!inner || (inner.flags & (TypeFlags.Any | TypeFlags.Unknown))) return false;
  const props = checker.getPropertiesOfType(inner);
  return props.length === 1 && props[0].name === 'default';
}

function openTsProject(root) {
  const existingConfig = resolve(root, 'ui/tsconfig.json');
  let configPath = existingConfig;
  let tempConfig = null;
  if (!existsSync(existingConfig)) {
    tempConfig = resolve(root, '.bp-dead-code.tsconfig.json');
    const include = ['ui/src'];
    if (existsSync(resolve(root, 'ui/vite.config.ts'))) include.push('ui/vite.config.ts');
    writeFileSync(tempConfig, JSON.stringify({
      compilerOptions: {
        target: 'ES2022',
        lib: ['ES2022', 'DOM', 'DOM.Iterable'],
        module: 'ESNext',
        moduleResolution: 'Bundler',
        jsx: 'react-jsx',
        noEmit: true,
        allowImportingTsExtensions: true,
        skipLibCheck: true,
      },
      include,
    }), 'utf8');
    configPath = tempConfig;
  }

  const api = new API({ cwd: root });
  const snapshot = api.updateSnapshot({ openProject: configPath });
  const project = snapshot.getProject(configPath);
  return {
    project,
    close() {
      try { snapshot.dispose(); } catch {}
      try { api.close(); } catch {}
      if (tempConfig) {
        try { rmSync(tempConfig, { force: true }); } catch {}
      }
    },
  };
}

export function analyzeUiSymbolGraph(root, allFiles, roleByRel, entryPoints) {
  const canonRelByLowerAbs = new Map();
  for (const file of allFiles) {
    canonRelByLowerAbs.set(toPosix(file.full).toLowerCase(), file.rel);
  }
  for (const extra of ['ui/vite.config.ts', 'ui/browserBridge.ts', 'ui/replayLibrary.ts']) {
    const full = resolve(root, extra);
    if (existsSync(full)) canonRelByLowerAbs.set(toPosix(full).toLowerCase(), extra);
  }
  const rootPrefixLower = `${toPosix(resolve(root)).toLowerCase().replace(/\/+$/, '')}/`;

  const toRel = absPath => {
    if (!absPath) return null;
    const norm = toPosix(absPath);
    const lower = norm.toLowerCase();
    const hit = canonRelByLowerAbs.get(lower);
    if (hit) return hit;
    if (lower.startsWith(rootPrefixLower)) {
      const rel = norm.slice(rootPrefixLower.length);
      if (!rel.startsWith('node_modules/') && !rel.includes('/node_modules/')) return rel;
    }
    return null;
  };

  const tsSession = openTsProject(root);
  try {
    const { project } = tsSession;
    const program = project.program;
    const checker = project.checker;

    const sfByRel = new Map();
    const prodSfs = [];
    for (const fileName of program.getSourceFileNames()) {
      const rel = toRel(fileName);
      if (!rel) continue;
      const sf = program.getSourceFile(fileName);
      if (!sf) continue;
      sfByRel.set(rel, sf);
      const role = roleByRel.get(rel);
      if (role === 'production' || !rel.startsWith('ui/src/')) {
        prodSfs.push({ rel, sf });
      }
    }

    const modSymIdToRel = new Map();
    const relToModSym = new Map();
    const CHUNK = 2000;
    for (let i = 0; i < prodSfs.length; i += CHUNK) {
      const slice = prodSfs.slice(i, i + CHUNK);
      const syms = checker.getSymbolAtLocation(slice.map(item => item.sf));
      for (let j = 0; j < slice.length; j += 1) {
        const sym = syms[j];
        if (sym) {
          modSymIdToRel.set(sym.id, slice[j].rel);
          relToModSym.set(slice[j].rel, sym);
        }
      }
    }

    const fileInfoCache = new Map();

    const getFileInfo = rel => {
      if (fileInfoCache.has(rel)) return fileInfoCache.get(rel);
      const sf = sfByRel.get(rel);
      if (!sf) {
        fileInfoCache.set(rel, null);
        return null;
      }
      const full = resolve(root, rel);
      const units = [];
      const topDeclByKey = new Map();
      const fileScopeNames = new Set();
      const bareImports = [];
      const nsImportNames = new Set();
      const importBindingInfo = new Map();
      let hasDynamicImport = false;
      let hasNamespaceImport = false;

      const registerPatternTopDecls = (pattern, unitIndex) => {
        if (!pattern) return;
        if (pattern.kind === SyntaxKind.Identifier) {
          fileScopeNames.add(pattern.text);
          return;
        }
        if (pattern.kind === SyntaxKind.ObjectBindingPattern || pattern.kind === SyntaxKind.ArrayBindingPattern) {
          for (const el of pattern.elements ?? []) {
            if (!el) continue;
            topDeclByKey.set(nodeKey(el), { kind: 'unit', unitIndex });
            registerPatternTopDecls(el.name, unitIndex);
          }
        }
      };

      for (const stmt of sf.statements) {
        if (stmt.kind === SyntaxKind.ImportDeclaration) {
          const spec = staticTsString(stmt.moduleSpecifier);
          const target = spec !== null ? resolveLocalImport(full, spec, root) : null;
          const targetRel = target ? relPosix(root, target) : null;
          const clause = stmt.importClause;
          const clauseTypeOnly = Boolean(clause?.isTypeOnly);
          if (!clause) {
            if (targetRel) bareImports.push({ targetRel, isType: false });
            continue;
          }
          if (clause.name) {
            fileScopeNames.add(clause.name.text);
            topDeclByKey.set(nodeKey(clause), { kind: 'import', targetRel, importedName: 'default', isType: clauseTypeOnly, isNamespace: false });
            importBindingInfo.set(clause.name.text, { targetRel, importedName: 'default', isType: clauseTypeOnly, isNamespace: false });
          }
          const nb = clause.namedBindings;
          if (nb?.kind === SyntaxKind.NamespaceImport) {
            hasNamespaceImport = true;
            nsImportNames.add(nb.name.text);
            fileScopeNames.add(nb.name.text);
            topDeclByKey.set(nodeKey(nb), { kind: 'import', targetRel, importedName: '*', isType: clauseTypeOnly, isNamespace: true });
            importBindingInfo.set(nb.name.text, { targetRel, importedName: '*', isType: clauseTypeOnly, isNamespace: true });
          } else if (nb?.kind === SyntaxKind.NamedImports) {
            for (const el of nb.elements ?? []) {
              const itemTypeOnly = clauseTypeOnly || Boolean(el.isTypeOnly);
              const importedName = el.propertyName?.text ?? el.name.text;
              fileScopeNames.add(el.name.text);
              topDeclByKey.set(nodeKey(el), { kind: 'import', targetRel, importedName, isType: itemTypeOnly, isNamespace: false });
              importBindingInfo.set(el.name.text, { targetRel, importedName, isType: itemTypeOnly, isNamespace: false });
            }
          }
          continue;
        }

        if (stmt.kind === SyntaxKind.ExportDeclaration) {
          const spec = stmt.moduleSpecifier ? staticTsString(stmt.moduleSpecifier) : null;
          const target = spec !== null ? resolveLocalImport(full, spec, root) : null;
          const targetRel = target ? relPosix(root, target) : null;
          if (stmt.exportClause?.kind === SyntaxKind.NamedExports) {
            for (const el of stmt.exportClause.elements ?? []) {
              const itemTypeOnly = Boolean(stmt.isTypeOnly || el.isTypeOnly);
              const exportedName = el.name.text;
              const localName = el.propertyName?.text ?? el.name.text;
              topDeclByKey.set(nodeKey(el), { kind: 'exportSpec', targetRel, exportedName, localName, isType: itemTypeOnly });
            }
          } else if (stmt.exportClause?.kind === SyntaxKind.NamespaceExport) {
            topDeclByKey.set(nodeKey(stmt.exportClause), {
              kind: 'exportNs',
              targetRel,
              exportedName: stmt.exportClause.name.text,
              isType: Boolean(stmt.isTypeOnly),
            });
          }
          continue;
        }

        if (stmt.kind === SyntaxKind.VariableStatement) {
          const exported = hasModifier(stmt, SyntaxKind.ExportKeyword);
          for (const decl of stmt.declarationList.declarations) {
            const unitIndex = units.length;
            const bindings = [];
            collectPatternNames(decl.name, bindings);
            units.push({
              unitIndex,
              node: decl,
              parentStmt: stmt,
              isExported: exported,
              isTypeOnly: false,
              isSideEffect: Boolean(decl.initializer && isSideEffectingExpr(decl.initializer)),
              bindings,
            });
            topDeclByKey.set(nodeKey(decl), { kind: 'unit', unitIndex });
            registerPatternTopDecls(decl.name, unitIndex);
          }
          continue;
        }

        const unitIndex = units.length;
        const isTypeOnly = stmt.kind === SyntaxKind.InterfaceDeclaration
          || stmt.kind === SyntaxKind.TypeAliasDeclaration
          || (stmt.kind === SyntaxKind.ModuleDeclaration && hasModifier(stmt, SyntaxKind.DeclareKeyword));
        const isExported = hasModifier(stmt, SyntaxKind.ExportKeyword) || stmt.kind === SyntaxKind.ExportAssignment;
        let isSideEffect = false;
        if (stmt.kind === SyntaxKind.FunctionDeclaration || stmt.kind === SyntaxKind.InterfaceDeclaration
          || stmt.kind === SyntaxKind.TypeAliasDeclaration || stmt.kind === SyntaxKind.EmptyStatement
          || stmt.kind === SyntaxKind.ModuleDeclaration) {
          isSideEffect = false;
        } else if (stmt.kind === SyntaxKind.ClassDeclaration) {
          isSideEffect = classHasSideEffects(stmt);
        } else if (stmt.kind === SyntaxKind.EnumDeclaration) {
          isSideEffect = (stmt.members ?? []).some(m => m.initializer && isSideEffectingExpr(m.initializer));
        } else if (stmt.kind === SyntaxKind.ExportAssignment) {
          isSideEffect = isSideEffectingExpr(stmt.expression);
        } else {
          isSideEffect = true;
        }
        const bindings = [];
        if (stmt.name?.kind === SyntaxKind.Identifier) {
          bindings.push(stmt.name.text);
          fileScopeNames.add(stmt.name.text);
        }
        units.push({
          unitIndex,
          node: stmt,
          parentStmt: stmt,
          isExported,
          isTypeOnly,
          isSideEffect,
          bindings,
        });
        topDeclByKey.set(nodeKey(stmt), { kind: 'unit', unitIndex });
      }

      const scanDynamic = n => {
        if (hasDynamicImport) return;
        if (n.kind === SyntaxKind.ImportKeyword) {
          hasDynamicImport = true;
          return;
        }
        n.forEachChild(scanDynamic);
      };
      scanDynamic(sf);

      let modSym = relToModSym.get(rel);
      if (!modSym) {
        modSym = checker.getSymbolAtLocation(sf);
        if (modSym) {
          modSymIdToRel.set(modSym.id, rel);
          relToModSym.set(rel, modSym);
        }
      }

      const declaredExports = new Map();
      const exportSymbolByName = new Map();
      if (modSym) {
        for (const sym of checker.getExportsOfModule(modSym)) {
          exportSymbolByName.set(sym.name, sym);
          const ownDecls = (sym.declarations ?? []).filter(d => toRel(d.path) === rel);
          if (ownDecls.length === 0) continue;
          const resolvedDecls = ownDecls.map(d => d.resolve(project)).filter(Boolean);
          const isReexport = resolvedDecls.some(d =>
            (d.kind === SyntaxKind.ExportSpecifier && Boolean(d.parent?.parent?.moduleSpecifier))
            || d.kind === SyntaxKind.NamespaceExport,
          );
          const isLocalSpecifier = !isReexport && resolvedDecls.some(d => d.kind === SyntaxKind.ExportSpecifier);
          const targetSym = (sym.flags & SymbolFlags.Alias) ? checker.getAliasedSymbol(sym) : sym;
          const isType = resolvedDecls.every(d =>
            d.kind === SyntaxKind.InterfaceDeclaration
            || d.kind === SyntaxKind.TypeAliasDeclaration
            || Boolean(d.isTypeOnly)
            || Boolean(d.parent?.parent?.isTypeOnly),
          ) || (!checker.isUnknownSymbol(targetSym) && (targetSym.flags & SymbolFlags.Value) === 0);

          const unitIndices = [];
          for (const d of resolvedDecls) {
            const mapped = topDeclByKey.get(nodeKey(d));
            if (mapped?.kind === 'unit' && !unitIndices.includes(mapped.unitIndex)) {
              unitIndices.push(mapped.unitIndex);
            }
          }
          if (isLocalSpecifier && unitIndices.length === 0) {
            for (const d of resolvedDecls) {
              if (d.kind !== SyntaxKind.ExportSpecifier) continue;
              const localTarget = checker.getExportSpecifierLocalTargetSymbol(d);
              for (const ld of localTarget?.declarations ?? []) {
                if (toRel(ld.path) !== rel) continue;
                const lr = ld.resolve(project);
                if (!lr) continue;
                const mapped = topDeclByKey.get(nodeKey(lr));
                if (mapped?.kind === 'unit' && !unitIndices.includes(mapped.unitIndex)) {
                  unitIndices.push(mapped.unitIndex);
                }
              }
            }
          }
          declaredExports.set(sym.name, {
            name: sym.name,
            symbol: sym,
            isType,
            isReexport,
            isLocalSpecifier,
            unitIndices,
          });
        }
      }

      const info = {
        rel,
        sf,
        full,
        units,
        topDeclByKey,
        fileScopeNames,
        bareImports,
        nsImportNames,
        importBindingInfo,
        hasDynamicImport,
        hasNamespaceImport,
        declaredExports,
        exportSymbolByName,
      };
      fileInfoCache.set(rel, info);
      return info;
    };

    const runtimeLive = new Set();
    const typeOnlyLive = new Set();
    const runtimeDemand = new Map();
    const typeDemand = new Map();
    const liveRuntimeUnits = new Map();
    const liveTypeUnits = new Map();
    const referencedUnits = new Map();
    const entryModules = new Set();
    const productionTestImports = [];
    const seenProdTestImport = new Set();

    const ensureSet = (map, key) => {
      let set = map.get(key);
      if (!set) map.set(key, set = new Set());
      return set;
    };

    let changed = false;

    const markRuntimeUnit = (rel, idx, fromReference = false) => {
      const set = ensureSet(liveRuntimeUnits, rel);
      if (!set.has(idx)) {
        set.add(idx);
        changed = true;
      }
      if (fromReference) {
        const refSet = ensureSet(referencedUnits, rel);
        if (!refSet.has(idx)) refSet.add(idx);
      }
    };

    const markTypeUnit = (rel, idx, fromReference = false) => {
      const set = ensureSet(liveTypeUnits, rel);
      if (!set.has(idx)) {
        set.add(idx);
        changed = true;
      }
      if (fromReference) {
        const refSet = ensureSet(referencedUnits, rel);
        if (!refSet.has(idx)) refSet.add(idx);
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
      entryModules.add(targetRel);
      addModuleEdge(null, targetRel, false, null);
    };

    for (const htmlRel of entryPoints) {
      const full = resolve(root, htmlRel);
      if (!existsSync(full)) continue;
      for (const spec of extractImportSpecifiers(full)) {
        const target = resolveLocalImport(full, spec.specifier, root);
        if (target) seedEntryModule(relPosix(root, target));
      }
    }

    const viteConfigRel = 'ui/vite.config.ts';
    if (sfByRel.has(viteConfigRel)) {
      addModuleEdge(null, viteConfigRel, false, '*');
    }

    const symbolInvolvesModuleCache = new Map();
    const symbolInvolvesModule = sym => {
      if (!sym) return false;
      if (symbolInvolvesModuleCache.has(sym.id)) return symbolInvolvesModuleCache.get(sym.id);
      const t = checker.getTypeOfSymbol(sym);
      const res = typeInvolvesModule(t, modSymIdToRel, checker);
      symbolInvolvesModuleCache.set(sym.id, res);
      return res;
    };

    const followSymbolReference = (fromRel, sym, inType) => {
      if (!sym || checker.isUnknownSymbol(sym)) return false;
      let cur = sym;
      let forceType = inType;
      const seen = new Set();
      let touchesModule = false;

      while (cur && !checker.isUnknownSymbol(cur) && !seen.has(cur.id)) {
        seen.add(cur.id);
        const decls = cur.declarations ?? [];
        let handledHop = false;

        for (const handle of decls) {
          const declRel = toRel(handle.path);
          if (!declRel) continue;
          const declInfo = getFileInfo(declRel);
          if (!declInfo) continue;
          const resolved = handle.resolve(project);
          if (!resolved) continue;
          const mapped = declInfo.topDeclByKey.get(nodeKey(resolved));
          if (!mapped) continue;

          if (mapped.kind === 'import') {
            handledHop = true;
            if (mapped.isType) forceType = true;
            if (mapped.isNamespace) {
              touchesModule = true;
              if (mapped.targetRel) addModuleEdge(declRel, mapped.targetRel, forceType, null);
            } else if (mapped.targetRel) {
              addModuleEdge(declRel, mapped.targetRel, forceType, mapped.importedName);
            }
          } else if (mapped.kind === 'exportSpec') {
            handledHop = true;
            if (mapped.isType) forceType = true;
            addModuleEdge(fromRel, declRel, forceType, mapped.exportedName);
            if (mapped.targetRel) {
              addModuleEdge(declRel, mapped.targetRel, forceType, mapped.localName);
            }
          } else if (mapped.kind === 'exportNs') {
            handledHop = true;
            if (mapped.isType) forceType = true;
            touchesModule = true;
            addModuleEdge(fromRel, declRel, forceType, mapped.exportedName);
            if (mapped.targetRel) {
              addModuleEdge(declRel, mapped.targetRel, forceType, null);
            }
          } else if (mapped.kind === 'unit') {
            handledHop = true;
            const unit = declInfo.units[mapped.unitIndex];
            const isType = forceType || unit.isTypeOnly || (cur.flags & SymbolFlags.Value) === 0;
            if (declRel === fromRel) {
              if (isType) markTypeUnit(declRel, mapped.unitIndex, true);
              else markRuntimeUnit(declRel, mapped.unitIndex, true);
            } else {
              const expName = hasModifier(unit.parentStmt, SyntaxKind.DefaultKeyword) || unit.node.kind === SyntaxKind.ExportAssignment
                ? 'default'
                : cur.name;
              addModuleEdge(fromRel, declRel, isType, expName);
              if (isType) markTypeUnit(declRel, mapped.unitIndex, true);
              else markRuntimeUnit(declRel, mapped.unitIndex, true);
            }
          }
        }

        if (!handledHop && !(cur.flags & SymbolFlags.Alias)) break;
        if (cur.flags & SymbolFlags.Alias) {
          const next = checker.getImmediateAliasedSymbol(cur);
          if (!next || checker.isUnknownSymbol(next)) break;
          cur = next;
        } else {
          break;
        }
      }

      if (!touchesModule && !forceType && symbolInvolvesModule(sym)) {
        touchesModule = true;
      }
      return touchesModule;
    };

    const demandExportOnModule = (rel, expName, isType) => {
      const info = getFileInfo(rel);
      if (!info) return;
      const allWanted = expName === '*';
      if (allWanted) {
        for (const [, exp] of info.declaredExports) {
          demandExportOnModule(rel, exp.name, isType || exp.isType);
        }
        for (const stmt of info.sf.statements) {
          if (stmt.kind === SyntaxKind.ExportDeclaration && !stmt.exportClause && stmt.moduleSpecifier) {
            const spec = staticTsString(stmt.moduleSpecifier);
            const target = spec !== null ? resolveLocalImport(info.full, spec, root) : null;
            if (target) addModuleEdge(rel, relPosix(root, target), isType || Boolean(stmt.isTypeOnly), '*');
          }
        }
        return;
      }
      const sym = info.exportSymbolByName.get(expName);
      if (!sym) return;
      const declMeta = info.declaredExports.get(expName);
      const effectiveType = isType || Boolean(declMeta?.isType) || (sym.flags & SymbolFlags.Value) === 0;
      if (declMeta) {
        for (const idx of declMeta.unitIndices) {
          if (effectiveType) markTypeUnit(rel, idx, true);
          else markRuntimeUnit(rel, idx, true);
        }
      }
      if (sym.flags & SymbolFlags.Alias) {
        const next = checker.getImmediateAliasedSymbol(sym);
        if (next && !checker.isUnknownSymbol(next)) {
          followSymbolReference(rel, next, effectiveType);
        }
      } else {
        for (const handle of sym.declarations ?? []) {
          const targetRel = toRel(handle.path);
          if (!targetRel) continue;
          const targetInfo = getFileInfo(targetRel);
          if (!targetInfo) continue;
          const resolved = handle.resolve(project);
          if (!resolved) continue;
          const mapped = targetInfo.topDeclByKey.get(nodeKey(resolved));
          if (mapped?.kind === 'unit') {
            if (targetRel !== rel) addModuleEdge(rel, targetRel, effectiveType, expName);
            if (effectiveType) markTypeUnit(targetRel, mapped.unitIndex, true);
            else markRuntimeUnit(targetRel, mapped.unitIndex, true);
          }
        }
      }
    };

    const resolveCallableDecls = calleeNode => {
      const callee = unwrapTsExpr(calleeNode);
      if (!callee || callee.kind !== SyntaxKind.Identifier) return null;
      let sym = checker.getSymbolAtLocation(callee);
      if (!sym || checker.isUnknownSymbol(sym)) return null;
      if (sym.flags & SymbolFlags.Alias) {
        sym = checker.getAliasedSymbol(sym);
        if (!sym || checker.isUnknownSymbol(sym)) return null;
      }
      const decls = sym.declarations ?? [];
      if (decls.length === 0) return null;
      const fns = [];
      for (const handle of decls) {
        const declRel = toRel(handle.path);
        if (!declRel || !sfByRel.has(declRel)) return null;
        const decl = handle.resolve(project);
        if (!decl) return null;
        if (decl.kind === SyntaxKind.FunctionDeclaration) {
          if (decl.body) fns.push({ declRel, fn: decl });
        } else if (decl.kind === SyntaxKind.VariableDeclaration && decl.initializer) {
          const list = decl.parent;
          const isConst = list?.kind === SyntaxKind.VariableDeclarationList
            && Boolean(list.flags & NodeFlags.Const);
          if (!isConst) return null;
          const init = unwrapTsExpr(decl.initializer);
          if ((init?.kind === SyntaxKind.ArrowFunction || init?.kind === SyntaxKind.FunctionExpression) && init.body) {
            fns.push({ declRel, fn: init });
          } else {
            return null;
          }
        } else {
          return null;
        }
      }
      return fns.length > 0 ? fns : null;
    };

    const collectReachableValueSubtree = (rootNodes, onNode) => {
      const parentOfLocal = new Map();
      const deferredNodes = new Set();
      const liveDeferred = new Set();
      const deferredBySymId = new Map();
      const deferredNames = new Set();
      const allIdents = [];
      const allShorthands = [];
      const pending = rootNodes.filter(Boolean).map(node => ({ node, parent: null }));

      const walkReachable = (node, parent, rootNode, newlyDeferred) => {
        if (!node) return;
        parentOfLocal.set(node, parent);
        if (TYPE_NODE_KINDS.has(node.kind)) return;
        if (isDeferredNestedDecl(node, rootNode)) {
          if (!deferredNodes.has(node)) {
            deferredNodes.add(node);
            deferredNames.add(node.name.text);
            newlyDeferred.push({ node, parent });
          }
          return;
        }
        onNode(node, parent, parentOfLocal);
        if (node.kind === SyntaxKind.ShorthandPropertyAssignment) {
          allShorthands.push(node);
        } else if (node.kind === SyntaxKind.Identifier && !isSkippedIdent(node, parent)) {
          allIdents.push(node);
        }
        node.forEachChild(child => {
          const childInType = (node.type === child)
            || (Array.isArray(node.typeArguments) && node.typeArguments.includes(child))
            || (Array.isArray(node.typeParameters) && node.typeParameters.includes(child));
          if (!childInType) walkReachable(child, node, rootNode, newlyDeferred);
        });
      };

      let identCursor = 0;
      let shorthandCursor = 0;
      const activate = sym => {
        const targets = deferredBySymId.get(sym.id);
        if (!targets) return;
        for (const target of targets) {
          if (!liveDeferred.has(target.node)) {
            liveDeferred.add(target.node);
            pending.push(target);
          }
        }
      };

      while (pending.length > 0) {
        const newlyDeferred = [];
        const batch = pending.splice(0, pending.length);
        for (const item of batch) {
          walkReachable(item.node, item.parent, item.node, newlyDeferred);
        }
        if (newlyDeferred.length > 0) {
          const declSyms = checker.getSymbolAtLocation(newlyDeferred.map(item => item.node.name));
          for (let i = 0; i < newlyDeferred.length; i += 1) {
            const sym = declSyms[i];
            if (!sym) continue;
            let list = deferredBySymId.get(sym.id);
            if (!list) deferredBySymId.set(sym.id, list = []);
            list.push(newlyDeferred[i]);
          }
          identCursor = 0;
          shorthandCursor = 0;
        }
        if (deferredBySymId.size > 0) {
          const matching = [];
          for (let i = identCursor; i < allIdents.length; i += 1) {
            if (deferredNames.has(allIdents[i].text)) matching.push(allIdents[i]);
          }
          identCursor = allIdents.length;
          if (matching.length > 0) {
            const syms = checker.getSymbolAtLocation(matching);
            for (const sym of syms) {
              if (sym) activate(sym);
            }
          }
          for (let i = shorthandCursor; i < allShorthands.length; i += 1) {
            const sh = allShorthands[i];
            if (!deferredNames.has(sh.name?.text)) continue;
            const sym = checker.getShorthandAssignmentValueSymbol(sh);
            if (sym) activate(sym);
          }
          shorthandCursor = allShorthands.length;
        }
      }
      return parentOfLocal;
    };

    const functionUsesArguments = (fn, rootNodes) => {
      if (fn.kind === SyntaxKind.ArrowFunction) return false;
      const hits = [];
      collectReachableValueSubtree(rootNodes, (node, parent) => {
        if (node.kind === SyntaxKind.Identifier && node.text === 'arguments' && !isSkippedIdent(node, parent)) {
          hits.push(node);
        }
      });
      if (hits.length === 0) return false;
      const syms = checker.getSymbolAtLocation(hits);
      for (let i = 0; i < hits.length; i += 1) {
        const sym = syms[i];
        if (!sym || checker.isUnknownSymbol(sym)) return true;
        const decls = sym.declarations ?? [];
        if (decls.length === 0) return true;
        const isLocalShadow = decls.every(handle => {
          const d = handle.resolve(project);
          return d && d.pos >= fn.pos && d.end <= fn.end;
        });
        if (!isLocalShadow) return true;
      }
      return false;
    };

    const consumedMembersOfLocalSymbol = (sym, symName, rootNodes, visitedFnParams, visitedSyms) => {
      const members = new Set();
      const identHits = [];
      const shorthandHits = [];
      const localParentOf = collectReachableValueSubtree(rootNodes, (node, parent) => {
        if (node.kind === SyntaxKind.ShorthandPropertyAssignment && node.name?.text === symName) {
          shorthandHits.push(node);
        } else if (node.kind === SyntaxKind.Identifier && node.text === symName && !isSkippedIdent(node, parent)) {
          identHits.push(node);
        }
      });

      for (const sh of shorthandHits) {
        const shSym = checker.getShorthandAssignmentValueSymbol(sh);
        if (shSym?.id === sym.id) return null;
      }

      if (identHits.length > 0) {
        const hitSyms = checker.getSymbolAtLocation(identHits);
        for (let i = 0; i < identHits.length; i += 1) {
          if (hitSyms[i]?.id !== sym.id) continue;
          let cur = identHits[i];
          let parent = localParentOf.get(cur);
          while (parent && (
            parent.kind === SyntaxKind.ParenthesizedExpression
            || parent.kind === SyntaxKind.AsExpression
            || parent.kind === SyntaxKind.SatisfiesExpression
            || parent.kind === SyntaxKind.NonNullExpression
          )) {
            cur = parent;
            parent = localParentOf.get(cur);
          }
          if (!parent) return null;
          if (parent.kind === SyntaxKind.PropertyAccessExpression && parent.expression === cur) {
            members.add(parent.name.text);
          } else if (parent.kind === SyntaxKind.ElementAccessExpression && parent.expression === cur) {
            const keys = stringLiteralsFromType(checker.getTypeAtLocation(parent.argumentExpression));
            if (!keys) return null;
            for (const k of keys) members.add(k);
          } else if (parent.kind === SyntaxKind.VariableDeclaration && parent.initializer === cur) {
            if (parent.name.kind === SyntaxKind.ObjectBindingPattern) {
              const extracted = extractObjectBindingMembers(parent.name, checker);
              if (!extracted) return null;
              for (const k of extracted) members.add(k);
            } else if (parent.name.kind === SyntaxKind.Identifier) {
              const aliasSym = checker.getSymbolAtLocation(parent.name);
              if (!aliasSym) return null;
              if (!visitedSyms.has(aliasSym.id)) {
                visitedSyms.add(aliasSym.id);
                const nested = consumedMembersOfLocalSymbol(aliasSym, parent.name.text, rootNodes, visitedFnParams, visitedSyms);
                if (!nested) return null;
                for (const k of nested) members.add(k);
              }
            } else {
              return null;
            }
          } else if (parent.kind === SyntaxKind.CallExpression && Array.isArray(parent.arguments)) {
            const subIdx = parent.arguments.indexOf(cur);
            if (subIdx < 0) return null;
            const nested = consumedMembersOfCallArg(parent, subIdx, visitedFnParams);
            if (!nested) return null;
            for (const k of nested) members.add(k);
          } else if (parent.kind === SyntaxKind.TypeQuery || parent.kind === SyntaxKind.ExpressionStatement || parent.kind === SyntaxKind.VoidExpression) {
            continue;
          } else {
            return null;
          }
        }
      }
      return members;
    };

    const consumedMembersOfCallArg = (callNode, argIdx, visitedFnParams) => {
      const args = callNode.arguments ?? [];
      for (let i = 0; i <= argIdx && i < args.length; i += 1) {
        if (args[i]?.kind === SyntaxKind.SpreadElement) return null;
      }
      const fns = resolveCallableDecls(callNode.expression);
      if (!fns) return null;
      const allMembers = new Set();
      for (const { declRel, fn } of fns) {
        const rawParams = fn.parameters ?? [];
        const params = rawParams.filter((p, i) => !(i === 0 && p.name?.kind === SyntaxKind.Identifier && p.name.text === 'this'));
        const rootNodes = [fn.body, ...params.map(p => p.initializer).filter(Boolean)];
        if (functionUsesArguments(fn, rootNodes)) return null;
        for (let i = 0; i <= argIdx && i < params.length; i += 1) {
          if (params[i]?.dotDotDotToken) return null;
        }
        const param = params[argIdx];
        if (!param) continue;
        const visitKey = `${declRel}:${nodeKey(fn)}:${argIdx}`;
        if (visitedFnParams.has(visitKey)) continue;
        visitedFnParams.add(visitKey);
        if (param.name.kind === SyntaxKind.ObjectBindingPattern) {
          const extracted = extractObjectBindingMembers(param.name, checker);
          if (!extracted) return null;
          for (const k of extracted) allMembers.add(k);
        } else if (param.name.kind === SyntaxKind.Identifier) {
          const paramSym = checker.getSymbolAtLocation(param.name);
          if (!paramSym) return null;
          const used = consumedMembersOfLocalSymbol(paramSym, param.name.text, rootNodes, visitedFnParams, new Set([paramSym.id]));
          if (!used) return null;
          for (const k of used) allMembers.add(k);
        } else {
          return null;
        }
      }
      return allMembers;
    };

    const processUnit = (rel, unitIndex, unitIsType) => {
      const info = getFileInfo(rel);
      if (!info) return;
      const unit = info.units[unitIndex];
      if (!unit) return;
      const isWidgetManifestFile = rel === 'ui/src/widgets/manifest.ts';

      const identCandidates = [];
      const shorthandCandidates = [];
      const dynamicImports = [];
      const inlineTypeImports = [];
      const workerUrlTargets = [];
      const qualifiedTypes = [];
      const parentOf = new Map();
      const deferredNodes = new Set();
      const liveDeferredRuntime = new Set();
      const liveDeferredType = new Set();
      const deferredBySymId = new Map();
      const deferredNames = new Set();
      const allLocalIdents = [];
      const allLocalShorthands = [];

      const walk = (node, parent, inType, rootNode, newlyDeferred) => {
        if (!node) return;
        parentOf.set(node, parent);
        const nextInType = inType || TYPE_NODE_KINDS.has(node.kind);

        if (isDeferredNestedDecl(node, rootNode)) {
          if (!deferredNodes.has(node)) {
            deferredNodes.add(node);
            deferredNames.add(node.name.text);
            newlyDeferred.push({ node, parent, inType: nextInType });
          }
          return;
        }

        if (node.kind === SyntaxKind.ShorthandPropertyAssignment && !nextInType) {
          allLocalShorthands.push({ node, inType: nextInType });
          if (info.fileScopeNames.has(node.name?.text)) {
            shorthandCandidates.push(node);
          }
        }

        if (!nextInType && node.kind === SyntaxKind.NewExpression
          && node.expression?.kind === SyntaxKind.Identifier && node.expression.text === 'URL'
          && (node.arguments?.length ?? 0) >= 2
          && isTsImportMetaUrl(node.arguments[1])) {
          const spec = staticTsString(node.arguments[0]);
          if (spec !== null) {
            const target = resolveLocalImport(info.full, spec, root);
            if (target) workerUrlTargets.push(relPosix(root, target));
          }
        }

        if (node.kind === SyntaxKind.ImportType) {
          const lit = node.argument?.literal ?? node.argument;
          const spec = staticTsString(lit);
          if (spec !== null) {
            const target = resolveLocalImport(info.full, spec, root);
            if (target) {
              inlineTypeImports.push({ targetRel: relPosix(root, target), name: leftmostQualified(node.qualifier) ?? '*' });
            }
          }
        }

        if (node.kind === SyntaxKind.CallExpression && node.expression?.kind === SyntaxKind.ImportKeyword) {
          const spec = node.arguments?.[0] ? staticTsString(node.arguments[0]) : null;
          if (spec !== null) {
            const target = resolveLocalImport(info.full, spec, root);
            if (target) {
              dynamicImports.push({ node, targetRel: relPosix(root, target), isType: nextInType });
            }
          }
        }

        if (node.kind === SyntaxKind.QualifiedName && nextInType && node.left?.kind === SyntaxKind.Identifier && info.nsImportNames.has(node.left.text)) {
          qualifiedTypes.push({ nsNode: node.left, member: node.right.text });
        }

        if (node.kind === SyntaxKind.Identifier && !isSkippedIdent(node, parent)) {
          allLocalIdents.push({ node, inType: nextInType });
          if (info.fileScopeNames.has(node.text)) {
            identCandidates.push({ node, inType: nextInType });
          }
        }

        node.forEachChild(child => {
          const childInType = nextInType
            || (node.type === child)
            || (Array.isArray(node.typeArguments) && node.typeArguments.includes(child))
            || (Array.isArray(node.typeParameters) && node.typeParameters.includes(child))
            || (node.kind === SyntaxKind.HeritageClause && node.token === SyntaxKind.ImplementsKeyword)
            || (node.kind === SyntaxKind.InterfaceDeclaration && Array.isArray(node.heritageClauses) && node.heritageClauses.includes(child));
          walk(child, node, childInType, rootNode, newlyDeferred);
        });
      };

      const pendingRoots = [{ node: unit.node, parent: null, inType: unitIsType || unit.isTypeOnly }];
      let checkedIdentCursor = 0;
      let checkedShorthandCursor = 0;

      const activateDeferred = (sym, refInType) => {
        const targets = deferredBySymId.get(sym.id);
        if (!targets) return;
        for (const target of targets) {
          const walkInType = target.inType || refInType;
          const liveSet = walkInType ? liveDeferredType : liveDeferredRuntime;
          if (!liveSet.has(target.node)) {
            liveSet.add(target.node);
            pendingRoots.push({ node: target.node, parent: target.parent, inType: walkInType });
          }
        }
      };

      while (pendingRoots.length > 0) {
        const newlyDeferred = [];
        const batch = pendingRoots.splice(0, pendingRoots.length);
        for (const rootItem of batch) {
          walk(rootItem.node, rootItem.parent, rootItem.inType, rootItem.node, newlyDeferred);
        }
        if (newlyDeferred.length > 0) {
          const declSyms = checker.getSymbolAtLocation(newlyDeferred.map(item => item.node.name));
          for (let i = 0; i < newlyDeferred.length; i += 1) {
            const sym = declSyms[i];
            if (!sym) continue;
            let list = deferredBySymId.get(sym.id);
            if (!list) deferredBySymId.set(sym.id, list = []);
            list.push(newlyDeferred[i]);
          }
          checkedIdentCursor = 0;
          checkedShorthandCursor = 0;
        }
        if (deferredBySymId.size > 0) {
          const matchingIdents = [];
          for (let i = checkedIdentCursor; i < allLocalIdents.length; i += 1) {
            const item = allLocalIdents[i];
            if (deferredNames.has(item.node.text)) matchingIdents.push(item);
          }
          checkedIdentCursor = allLocalIdents.length;
          if (matchingIdents.length > 0) {
            const syms = checker.getSymbolAtLocation(matchingIdents.map(item => item.node));
            for (let i = 0; i < matchingIdents.length; i += 1) {
              const sym = syms[i];
              if (sym) activateDeferred(sym, matchingIdents[i].inType);
            }
          }
          for (let i = checkedShorthandCursor; i < allLocalShorthands.length; i += 1) {
            const item = allLocalShorthands[i];
            if (!deferredNames.has(item.node.name?.text)) continue;
            const sym = checker.getShorthandAssignmentValueSymbol(item.node);
            if (sym) activateDeferred(sym, item.inType);
          }
          checkedShorthandCursor = allLocalShorthands.length;
        }
      }

      let touchesModule = dynamicImports.length > 0;

      if (identCandidates.length > 0) {
        const syms = checker.getSymbolAtLocation(identCandidates.map(item => item.node));
        for (let i = 0; i < identCandidates.length; i += 1) {
          const sym = syms[i];
          if (!sym) continue;
          const effectiveType = unitIsType || identCandidates[i].inType;
          if (followSymbolReference(rel, sym, effectiveType)) {
            touchesModule = true;
          }
        }
      }

      for (const shNode of shorthandCandidates) {
        const sym = checker.getShorthandAssignmentValueSymbol(shNode);
        if (!sym) continue;
        if (followSymbolReference(rel, sym, unitIsType)) {
          touchesModule = true;
        }
      }

      for (const qt of qualifiedTypes) {
        const sym = checker.getSymbolAtLocation(qt.nsNode);
        if (!sym) continue;
        const decl = sym.declarations?.[0]?.resolve(project);
        const mapped = decl ? info.topDeclByKey.get(nodeKey(decl)) : null;
        if (mapped?.kind === 'import' && mapped.isNamespace && mapped.targetRel) {
          addModuleEdge(rel, mapped.targetRel, true, qt.member);
        }
      }

      if (!unitIsType) {
        for (const wRel of workerUrlTargets) {
          seedEntryModule(wRel);
        }
      }

      for (const inl of inlineTypeImports) {
        addModuleEdge(rel, inl.targetRel, true, inl.name);
      }

      if (!unitIsType) {
        for (const dyn of dynamicImports) {
          const dynIsType = dyn.isType;
          addModuleEdge(rel, dyn.targetRel, dynIsType, null);
          let cur = dyn.node;
          let p = parentOf.get(cur);
          while (p && (p.kind === SyntaxKind.ParenthesizedExpression || p.kind === SyntaxKind.AsExpression || p.kind === SyntaxKind.SatisfiesExpression)) {
            cur = p;
            p = parentOf.get(cur);
          }
          if (p && (p.kind === SyntaxKind.ArrowFunction || p.kind === SyntaxKind.ReturnStatement)) {
            const fnNode = p.kind === SyntaxKind.ArrowFunction ? p : parentOf.get(parentOf.get(p));
            const fnParent = fnNode ? parentOf.get(fnNode) : null;
            if (fnParent && isTsLazyCall(fnParent)) {
              addModuleEdge(rel, dyn.targetRel, dynIsType, 'default');
              continue;
            }
            if (isWidgetManifestFile && fnParent?.kind === SyntaxKind.PropertyAssignment && fnParent.name?.text === 'load') {
              addModuleEdge(rel, dyn.targetRel, dynIsType, 'default');
              continue;
            }
            if (fnNode) {
              const ctxType = checker.getContextualType(fnNode);
              if (ctxType) {
                if (isDefaultOnlyContextualType(ctxType, checker)) {
                  addModuleEdge(rel, dyn.targetRel, dynIsType, 'default');
                } else if (isOpaqueWideningType(ctxType, checker)) {
                  addModuleEdge(rel, dyn.targetRel, dynIsType, '*');
                }
              }
            }
          }
        }
      }

      if (touchesModule) {
        const propAccesses = [];
        const elemAccesses = [];
        const objBindings = [];
        const potentialEscapes = [];

        const walkMod = (node, inTypeSyntax, bodyIsTypeOnly) => {
          if (!node) return;
          let nextBodyIsTypeOnly = bodyIsTypeOnly;
          if (deferredNodes.has(node)) {
            if (!liveDeferredRuntime.has(node) && !liveDeferredType.has(node)) return;
            nextBodyIsTypeOnly = bodyIsTypeOnly || !liveDeferredRuntime.has(node);
          }
          const nextInTypeSyntax = inTypeSyntax || TYPE_NODE_KINDS.has(node.kind);
          if (node.kind === SyntaxKind.PropertyAccessExpression && !nextInTypeSyntax) {
            propAccesses.push({ node, isType: nextBodyIsTypeOnly });
          } else if (node.kind === SyntaxKind.ElementAccessExpression && !nextInTypeSyntax) {
            elemAccesses.push({ node, isType: nextBodyIsTypeOnly });
          } else if (node.kind === SyntaxKind.ObjectBindingPattern && !nextInTypeSyntax) {
            objBindings.push({ node, isType: nextBodyIsTypeOnly });
          } else if (node.kind === SyntaxKind.IndexedAccessType) {
            elemAccesses.push({ node, isType: true });
          } else if (!nextInTypeSyntax && (
            node.kind === SyntaxKind.Identifier
            || node.kind === SyntaxKind.AwaitExpression
            || node.kind === SyntaxKind.CallExpression
          )) {
            let cur = node;
            let parent = parentOf.get(cur);
            while (parent && (
              parent.kind === SyntaxKind.ParenthesizedExpression
              || parent.kind === SyntaxKind.AsExpression
              || parent.kind === SyntaxKind.SatisfiesExpression
              || parent.kind === SyntaxKind.NonNullExpression
            )) {
              cur = parent;
              parent = parentOf.get(cur);
            }
            const consumedCleanly = !parent
              || (parent.kind === SyntaxKind.PropertyAccessExpression && parent.expression === cur)
              || (parent.kind === SyntaxKind.ElementAccessExpression && parent.expression === cur)
              || (parent.kind === SyntaxKind.VariableDeclaration && (parent.name === cur || parent.initializer === cur))
              || (parent.kind === SyntaxKind.Parameter && parent.name === cur)
              || (parent.kind === SyntaxKind.BindingElement)
              || (parent.kind === SyntaxKind.AwaitExpression && parent.expression === cur)
              || (parent.kind === SyntaxKind.ReturnStatement)
              || (parent.kind === SyntaxKind.ArrowFunction && parent.body === cur)
              || (parent.kind === SyntaxKind.ExpressionStatement)
              || (parent.kind === SyntaxKind.VoidExpression)
              || (parent.kind === SyntaxKind.PropertyAccessExpression && parent.name === cur)
              || (parent.kind === SyntaxKind.PropertyAssignment && parent.name === cur)
              || (parent.kind === SyntaxKind.CallExpression && parent.expression === cur)
              || (parent.kind === SyntaxKind.TypeQuery);
            if (!consumedCleanly) {
              potentialEscapes.push({ node, isType: nextBodyIsTypeOnly });
            }
          }
          node.forEachChild(child => {
            const childInTypeSyntax = nextInTypeSyntax || (node.type === child);
            walkMod(child, childInTypeSyntax, nextBodyIsTypeOnly);
          });
        };

        walkMod(unit.node, false, unitIsType || unit.isTypeOnly);

        const typeQueryNodes = [];
        for (const item of propAccesses) typeQueryNodes.push(item.node.expression);
        for (const item of elemAccesses) {
          if (item.node.kind === SyntaxKind.ElementAccessExpression) {
            typeQueryNodes.push(item.node.expression, item.node.argumentExpression);
          } else {
            typeQueryNodes.push(item.node.objectType, item.node.indexType);
          }
        }
        for (const item of objBindings) typeQueryNodes.push(item.node);
        for (const item of potentialEscapes) typeQueryNodes.push(item.node);

        if (typeQueryNodes.length > 0) {
          const types = checker.getTypeAtLocation(typeQueryNodes);
          let cursor = 0;

          for (const item of propAccesses) {
            const objType = types[cursor++];
            const targetRel = moduleRelFromType(objType, modSymIdToRel, checker);
            if (targetRel) {
              addModuleEdge(rel, targetRel, item.isType, item.node.name.text);
            }
          }

          for (const item of elemAccesses) {
            const objType = types[cursor++];
            const keyType = types[cursor++];
            const targetRel = moduleRelFromType(objType, modSymIdToRel, checker);
            if (targetRel) {
              const isTypeAccess = item.isType || item.node.kind === SyntaxKind.IndexedAccessType;
              const keys = stringLiteralsFromType(keyType);
              if (keys) {
                for (const k of keys) addModuleEdge(rel, targetRel, isTypeAccess, k);
              } else {
                addModuleEdge(rel, targetRel, isTypeAccess, '*');
              }
            }
          }

          for (const item of objBindings) {
            const patType = types[cursor++];
            const targetRel = moduleRelFromType(patType, modSymIdToRel, checker);
            if (targetRel) {
              for (const el of item.node.elements ?? []) {
                if (el.dotDotDotToken) {
                  addModuleEdge(rel, targetRel, item.isType, '*');
                } else if (el.propertyName) {
                  if (el.propertyName.kind === SyntaxKind.Identifier || el.propertyName.kind === SyntaxKind.StringLiteral) {
                    addModuleEdge(rel, targetRel, item.isType, el.propertyName.text);
                  } else if (el.propertyName.kind === SyntaxKind.ComputedPropertyName) {
                    const kt = checker.getTypeAtLocation(el.propertyName.expression);
                    const keys = stringLiteralsFromType(kt);
                    if (keys) {
                      for (const k of keys) addModuleEdge(rel, targetRel, item.isType, k);
                    } else {
                      addModuleEdge(rel, targetRel, item.isType, '*');
                    }
                  } else {
                    addModuleEdge(rel, targetRel, item.isType, '*');
                  }
                } else if (el.name?.kind === SyntaxKind.Identifier) {
                  addModuleEdge(rel, targetRel, item.isType, el.name.text);
                } else {
                  addModuleEdge(rel, targetRel, item.isType, '*');
                }
              }
            }
          }

          for (const item of potentialEscapes) {
            const n = item.node;
            const exprType = types[cursor++];
            const directMod = moduleRelFromType(exprType, modSymIdToRel, checker);
            if (directMod) {
              let cur = n;
              let p = parentOf.get(cur);
              while (p && (
                p.kind === SyntaxKind.ParenthesizedExpression
                || p.kind === SyntaxKind.AsExpression
                || p.kind === SyntaxKind.SatisfiesExpression
                || p.kind === SyntaxKind.NonNullExpression
              )) {
                cur = p;
                p = parentOf.get(cur);
              }
              if (p?.kind === SyntaxKind.CallExpression && Array.isArray(p.arguments)) {
                const argIdx = p.arguments.indexOf(cur);
                if (argIdx >= 0) {
                  const consumed = consumedMembersOfCallArg(p, argIdx, new Set());
                  if (consumed) {
                    addModuleEdge(rel, directMod, item.isType, null);
                    for (const member of consumed) {
                      addModuleEdge(rel, directMod, item.isType, member);
                    }
                    continue;
                  }
                }
              }
              addModuleEdge(rel, directMod, item.isType, '*');
              continue;
            }
            const p = parentOf.get(n);
            if (p?.kind === SyntaxKind.CallExpression) {
              const sigs = checker.getSignaturesOfType(exprType, SignatureKind.Call);
              const loaderTarget = sigs.length > 0
                ? unwrapPromiseModuleType(checker.getReturnTypeOfSignature(sigs[0]), modSymIdToRel, checker)
                : null;
              if (loaderTarget) {
                if (isTsLazyCall(p)) {
                  addModuleEdge(rel, loaderTarget, item.isType, 'default');
                } else {
                  const ctxType = checker.getContextualType(n);
                  if (ctxType && isDefaultOnlyContextualType(ctxType, checker)) {
                    addModuleEdge(rel, loaderTarget, item.isType, 'default');
                  } else if (ctxType && isOpaqueWideningType(ctxType, checker)) {
                    addModuleEdge(rel, loaderTarget, item.isType, '*');
                  }
                }
              }
            }
          }
        }
      }
    };

    const processedRuntimeInit = new Set();
    const processedRuntimeUnits = new Map();
    const processedTypeUnits = new Map();
    const processedRuntimeDemands = new Map();
    const processedTypeDemands = new Map();

    changed = true;
    while (changed) {
      changed = false;

      for (const rel of [...runtimeLive]) {
        if (rel.endsWith('.css')) {
          seedCss(rel);
          continue;
        }
        const info = getFileInfo(rel);
        if (!info) continue;

        if (!processedRuntimeInit.has(rel)) {
          processedRuntimeInit.add(rel);
          for (const bare of info.bareImports) {
            addModuleEdge(rel, bare.targetRel, bare.isType, null);
          }
          for (const unit of info.units) {
            if (unit.isSideEffect) markRuntimeUnit(rel, unit.unitIndex, false);
          }
        }

        const req = runtimeDemand.get(rel);
        if (req) {
          const doneReq = ensureSet(processedRuntimeDemands, rel);
          for (const expName of [...req]) {
            if (doneReq.has(expName)) continue;
            doneReq.add(expName);
            demandExportOnModule(rel, expName, false);
          }
        }

        const doneUnits = ensureSet(processedRuntimeUnits, rel);
        for (const idx of [...(liveRuntimeUnits.get(rel) ?? [])]) {
          if (doneUnits.has(idx)) continue;
          doneUnits.add(idx);
          processUnit(rel, idx, false);
        }
      }

      for (const rel of new Set([...typeOnlyLive, ...runtimeLive])) {
        const info = getFileInfo(rel);
        if (!info) continue;

        const req = typeDemand.get(rel);
        if (req) {
          const doneReq = ensureSet(processedTypeDemands, rel);
          for (const expName of [...req]) {
            if (doneReq.has(expName)) continue;
            doneReq.add(expName);
            demandExportOnModule(rel, expName, true);
          }
        }

        const doneUnits = ensureSet(processedTypeUnits, rel);
        for (const idx of [...(liveTypeUnits.get(rel) ?? [])]) {
          if (doneUnits.has(idx)) continue;
          doneUnits.add(idx);
          processUnit(rel, idx, true);
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

    const unusedExports = [];
    for (const rel of [...liveProd].sort()) {
      if (rel.startsWith('ui/src/generated/') || entryModules.has(rel)) continue;
      const info = getFileInfo(rel);
      if (!info) continue;
      const rReq = runtimeDemand.get(rel);
      const tReq = typeDemand.get(rel);
      if (rReq?.has('*') || tReq?.has('*')) continue;

      const refUnits = referencedUnits.get(rel) ?? new Set();
      for (const [expName, meta] of [...info.declaredExports.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
        if (rReq?.has(expName) || tReq?.has(expName)) continue;
        if (meta.isReexport) {
          unusedExports.push({ file: rel, name: expName, key: `${rel}:${expName}`, isType: meta.isType, isReexport: true });
          continue;
        }
        if (meta.isLocalSpecifier) {
          unusedExports.push({ file: rel, name: expName, key: `${rel}:${expName}`, isType: meta.isType, isReexport: false });
          continue;
        }
        const locallyReferenced = meta.unitIndices.some(idx => refUnits.has(idx));
        if (!locallyReferenced) {
          unusedExports.push({ file: rel, name: expName, key: `${rel}:${expName}`, isType: meta.isType, isReexport: false });
        }
      }
    }

    productionTestImports.sort((a, b) => `${a.from}->${a.to}`.localeCompare(`${b.from}->${b.to}`));

    return {
      liveProd,
      runtimeLive,
      typeOnlyLive,
      productionTestImports,
      unusedExports,
    };
  } finally {
    tsSession.close();
  }
}
