import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseAst } from 'rolldown/parseAst';
import '../widgets/definitions.ts';
import { createWidget, resolveWindow } from './model.ts';
import type { BoundTrack, HourWindow, WidgetSpec } from './model';
import type { ProductRequest } from './client';
import * as built from './widgetRequests.ts';

// The readiness probe's table (crates/dashboard/src/readiness/widgets.json) names each
// widget's first-render requests. A freshly mounted dashboard starts its clock at the
// branch's first hour and creates every widget at its default time and options.
const root = fileURLToPath(new URL('..', import.meta.url));
const table = JSON.parse(readFileSync(join(root, '../../crates/dashboard/src/readiness/widgets.json'), 'utf8')) as {
  widgets: { kind: string; requests?: Template[]; auxiliary?: Template[] }[];
};
interface Template { product: string; route: string; params: Record<string, unknown> }

const BRANCH = '3';
const TAG = 'GER';
const trackOf = (firstHour: number, lastHour: number) => ({
  id: 'main', roleId: 'primary', label: 'Main campaign', branchIndex: 0, anchorOffset: null, axisOffset: 0, selectedTag: TAG,
  branch: { id: BRANCH, firstHour, lastHour },
  source: { id: 'source', revision: 'r1', metadata: { countries: [], playerCountryId: null } },
}) as unknown as BoundTrack;
// A branch longer than a 90-day block, and one shorter than its first block and week.
const TRACKS = [trackOf(10, 5000), trackOf(10, 100)];

/** A request value that waits on an earlier reply; only its key is checked. */
const DEPENDENT = Symbol('dependent');
/** The recording facts the probe binds (`Bindings::value`) for `track`'s branch. */
function bindings(track: BoundTrack): Record<string, unknown> {
  const { firstHour: first, lastHour: last } = track.branch;
  const dayStart = first + Math.floor((last - first) / 24) * 24;
  return {
    $branch: BRANCH, $tag: TAG, $tags: [TAG], $first: first, $last: last, $width: 800,
    $firstDayEnd: Math.min(first + 23, last), $lastDayStart: dayStart, $lastDayEnd: Math.min(dayStart + 23, last),
    $afterLast: last + 1, $firstWeekEnd: first + 167, $firstBlockEnd: Math.min(last, Math.floor(first / 2160) * 2160 + 2159),
    $firstBlockDays: Math.floor(Math.min(last, Math.floor(first / 2160) * 2160 + 2159) / 24) - Math.floor(first / 24) + 1,
  };
}
function bind(value: unknown, facts = bindings(TRACKS[0])): unknown {
  if (typeof value === 'string' && value.startsWith('$')) {
    if (/^\$(window|windowLast|experienceReads):\d+:(first|last)$|^\$(experienceTags|airDayEnd|airRegions):\d+$/.test(value)) return DEPENDENT;
    assert.ok(value in facts, `unknown binding ${value}`);
    return facts[value];
  }
  if (Array.isArray(value)) return value.map(item => bind(item, facts));
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, bind(item, facts)]));
  return value;
}

/** Widgets whose requests the UI builds in `widgetRequests`: what the widget sends at the opening hour. */
type Built = (widget: WidgetSpec, track: BoundTrack, window: HourWindow) => (ProductRequest | null)[];
const EXACT: Record<string, Built> = {
  'campaign-clock': (widget, track, window) => [built.clockRequest(widget, track, window)],
  'air-missions': (widget, track) => [built.airMissionsRequest(widget, track)],
  'air-statistics': (widget, track) => [built.airStatisticsRequest(widget, track)],
  minefields: (widget, track) => [built.minefieldsRequest(widget, track)],
  'naval-supremacy': (widget, track, window) => [built.navalSupremacyRequest(widget, track, built.windowExtent(track, window))],
  'weather-by-region': (widget, track, window) => [built.weatherRequest(widget, track, built.windowExtent(track, window), 800)],
  'doctrine-mastery': (widget, track) => [built.doctrineMasteryRequest(widget, track)],
  'occupation-reserve-manpower': (widget, track) => [built.occupationReserveRequest(widget, track)],
  'intel-network-coverage': (widget, track) => [built.intelCoverageRequest(widget, track)],
  'dockyards-and-repairs': (widget, track, window) => [built.yardsRequest(widget, track, built.shownHour(track, window))],
  refits: (widget, track, window) => [built.yardsRequest(widget, track, built.shownHour(track, window))],
  'governments-in-exile': (_widget, track, window) => [built.exilesRequest(track, built.shownHour(track, window))],
};

// Named exceptions, by widget kind.
/** Widgets the table lists with no requests: they issue none until the user picks something. */
const NO_REQUESTS = new Set(['map', 'general-career', 'division-service-record']);
/** Widgets whose table entry deliberately differs from the first render, as its note says. */
const DEVIATES = new Set(['equipment-produced']);
/** Keys a widget's request names whose value is undefined, so not sent, on its default first render. */
const UNSENT: Record<string, readonly string[]> = {
  // The ledger's start is the window's only in range time.
  'political-power-spending': ['from'],
  'political-power-spending-breakdown': ['from'],
  'command-power-spending': ['from'],
  // The widget passes the party history no alias runs.
  'ruling-party': ['within'],
};
/** Keys a widget fills from an earlier reply that the table has no binding for yet. */
const REPLY_BOUND: Record<string, readonly string[]> = {
  // The selected identity's alias runs, from the country-groups reply.
  'political-power-spending': ['within'],
  'political-power-spending-breakdown': ['within'],
  'command-power-spending': ['within'],
};
/** Widget requests whose product or params pass through a value the source does not show (a prop,
 * component state): matched against requests with a variable product or unknown keys too. */
const LOOSE: Record<string, { products: readonly string[]; why: string }> = {
  'dockyard-spending': { products: ['dockyard-spending'], why: 'its spending view takes the product and params from the kind the widget passes as a prop' },
  'military-factory-spending': { products: ['military-factory-spending'], why: 'its spending view takes the product and params from the kind the widget passes as a prop' },
  'land-battles': { products: ['land-battles-manifest', 'land-battles'], why: 'its battle log names its products from the adapter the widget passes as a prop' },
  'naval-battles': { products: ['naval-battles-manifest', 'naval-battles'], why: 'its battle log names its products from the adapter the widget passes as a prop' },
  'occupation-trend': { products: ['series-summary'], why: "its summary's hours are the view its chart reports, held in component state" },
};
const loose = (kind: string, product: string) => LOOSE[kind]?.products.includes(product) ?? false;
/** Params that carry the hour a widget shows at its default time (`shownHour`), as a request index then a
 * path into its params; a window placeholder names the hour whose window it reads. */
const SHOWN: Record<string, readonly string[]> = {
  'campaign-clock': ['0.lastHour'],
  'national-focus': ['0.hour'],
  'active-research': ['0.reads.0.hour'],
  'political-power': ['0.to', '1.reads.0.hour'],
  'political-power-spending': ['1.hour'],
  'political-power-spending-breakdown': ['1.hour'],
  'command-power': ['0.to', '1.reads.0.hour', '2.to', '3.reads.0.hour'],
  'command-power-spending': ['1.hour'],
  stability: ['0.to', '1.reads.0.hour', '2.to', '3.reads.0.hour'],
  'idea-timeline': ['0.through'],
  'national-spirits': ['0.through', '0.hour'],
  'spirit-history': ['0.through'],
  'ruling-party': ['0.to', '1.hours.0', '2.hour'],
  'resource-balance': ['0.from', '0.to', '0.hours.0'],
  'equipment-stockpile': ['1.hour'],
  'dockyards-and-repairs': ['0.hour'],
  refits: ['0.hour'],
  fuel: ['0.to', '1.reads.0.hour'],
  factories: ['0.from', '0.to'],
  territory: ['0.reads.0.hour'],
  'territory-breakdown': ['0.reads.0.hour'],
  'equipment-produced': ['0.hour'],
  'production-spending': ['0.hour'],
  'dockyard-spending': ['0.hour'],
  'military-factory-spending': ['0.hour'],
  'occupied-states': ['1.window'],
  'occupation-trend': ['0.reads.0.hour', '1.to'],
  'air-force-size': ['0.from', '0.to'],
  'navy-size': ['0.from', '0.to'],
  'air-superiority-timeline': ['1.lastHour', '2.firstHour'],
  experience: ['1.to', '2.to', '3.to'],
  'faction-strength': ['0.reads.0.hour'],
  'trade-flows': ['0.from', '0.to', '0.hours.0'],
  wars: ['1.reads.0.hour'],
  'war-goals': ['0.hour'],
  'world-tension': ['1.reads.0.hour'],
  autonomy: ['1.reads.0.hour'],
  'construction-log': ['1.window'],
  'construction-spending': ['0.hours.0'],
  'special-projects': ['1.hour'],
  'nuclear-arsenal': ['0.from', '0.to'],
  'naval-losses': ['1.window'],
  'ships-sunk': ['1.hour'],
  'espionage-log': ['1.window'],
  'game-score': ['0.reads.0.hour'],
  casualties: ['0.reads.0.hour'],
  'casualties-between': ['1.window'],
  'casualties-breakdown': ['1.window'],
  generals: ['0.hour'],
  'division-template': ['1.hour'],
  'training-queue': ['1.hour'],
  'most-decorated-divisions': ['0.hour'],
  'army-losses-by-month': ['1.hour'],
  aces: ['0.hour'],
  'army-size': ['1.window'],
  'supply-health': ['0.from', '0.to', '1.from', '1.to'],
  'division-log': ['1.window'],
  'land-battles': ['1.window'],
  'naval-battles': ['1.window'],
  'event-feed': ['0.lastHour'],
  'market-ledger': ['0.lastHour'],
  'event-choices': ['0.lastHour'],
  decisions: ['0.lastHour'],
};
/** Params that carry the first and last hours of the day holding the shown hour, counted from the branch start. */
const SHOWN_DAY: Record<string, readonly [string, string]> = {
  decryption: ['0.firstHour', '0.lastHour'],
  'agency-comparison': ['0.firstHour', '0.lastHour'],
  'operatives-roster': ['1.firstHour', '1.lastHour'],
  'who-spies-on-whom': ['0.firstHour', '0.lastHour'],
};
/** A table value: a request index, then keys or array indices into its params. */
function valueAt(requests: Template[] | undefined, path: string): unknown {
  const [index, ...keys] = path.split('.');
  let value: unknown = requests?.[Number(index)]?.params;
  for (const key of keys) value = (value as Record<string, unknown> | undefined)?.[key];
  assert.notEqual(value, undefined, `no readiness param at ${path}`);
  return value;
}

// ---- The UI source, as the requests each module builds ----
type Node = { type: string; start: number; [key: string]: unknown };
interface File { path: string; program: Node; parents: Map<Node, Node> }
interface Shape { required: Set<string>; optional: Set<string>; open: boolean; truncated: boolean; files: Set<string> }
interface Request { product: string | null; shape: Shape; file: string }
type Env = Map<Node, { args: Node[]; at: Scope }>;
/** Where a value is read: its module, the arguments of the calls it is inside, and the names an
 * enclosing `a && b && …` has already found truthy there. */
interface Scope { file: File; env: Env; truthy?: ReadonlySet<string> }
/** A value a node can take: a terminal expression where it was written, or one the source does not show. */
type Written = { node: Node; at: Scope; files: Set<string> };
type Value = Written | { open: true; truncated: boolean; files: Set<string> };
const MAX_DEPTH = 24;

const files = new Map<string, File>();
function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === 'generated' ? [] : sources(path);
    return /\.tsx?$/.test(entry.name) && !/\.(test|d)\.tsx?$/.test(entry.name) ? [path] : [];
  });
}
const children = (node: Node): Node[] => Object.entries(node).flatMap(([key, value]) => key === 'parent' ? []
  : (Array.isArray(value) ? value : [value]).filter((child): child is Node => !!child && typeof child === 'object' && typeof (child as Node).type === 'string'));
function walk(node: Node, visit: (node: Node, parent: Node | undefined) => void, parent?: Node) {
  visit(node, parent);
  for (const child of children(node)) walk(child, visit, node);
}
for (const path of sources(root)) {
  const program = parseAst(readFileSync(path, 'utf8'), { lang: path.endsWith('x') ? 'tsx' : 'ts' }) as unknown as Node;
  const parents = new Map<Node, Node>();
  walk(program, (node, parent) => { if (parent) parents.set(node, parent); });
  files.set(path, { path, program, parents });
}
function moduleAt(from: string, spec: string): File | null {
  if (!spec.startsWith('.')) return null;
  const base = resolve(dirname(from), spec);
  for (const candidate of [base, ...['.ts', '.tsx'].map(ext => base + ext), ...['.ts', '.tsx'].map(ext => join(base, `index${ext}`))]) {
    if (files.has(candidate)) return files.get(candidate)!;
  }
  return null;
}
/** The modules a module loads at run time (type-only imports load nothing). */
function imports(file: File): File[] {
  const found: File[] = [];
  walk(file.program, node => {
    const typeOnly = node.importKind === 'type' || node.exportKind === 'type';
    if ((node.type === 'ImportDeclaration' || node.type === 'ExportNamedDeclaration' || node.type === 'ExportAllDeclaration' || node.type === 'ImportExpression')
      && !typeOnly && typeof (node.source as Node | undefined)?.value === 'string') {
      const target = moduleAt(file.path, String((node.source as Node).value));
      if (target) found.push(target);
    }
  });
  return found;
}
const reach = new Map<string, Set<string>>();
function reachable(entry: File): Set<string> {
  if (reach.has(entry.path)) return reach.get(entry.path)!;
  const seen = new Set<string>([entry.path]);
  const queue = [entry];
  while (queue.length) for (const next of imports(queue.pop()!)) if (!seen.has(next.path)) { seen.add(next.path); queue.push(next); }
  reach.set(entry.path, seen);
  return seen;
}
/** Each widget kind's module, as the renderer loaders import it. */
const loaders = new Map<string, File>();
{
  const path = join(root, 'widgets/rendererLoaders.ts');
  for (const [, kind, spec] of readFileSync(path, 'utf8').matchAll(/'([a-z0-9-]+)': \(\) => import\('([^']+)'\)/g)) {
    const file = moduleAt(path, spec);
    assert.ok(file, `${kind}'s module ${spec} does not resolve`);
    loaders.set(kind, file);
  }
}

const keyName = (node: Node | undefined): string | null => !node ? null : node.type === 'Identifier' ? String(node.name)
  : node.type === 'Literal' && typeof node.value === 'string' ? node.value : null;
const isFunction = (node: Node) => node.type === 'FunctionDeclaration' || node.type === 'FunctionExpression' || node.type === 'ArrowFunctionExpression';
type Binding = { kind: 'value'; node: Node; file: File; key?: string } | { kind: 'param'; fn: Node; index: number; key?: string; fallback?: Node; file: File }
  | { kind: 'function'; fn: Node; file: File } | { kind: 'import'; file: File; source: string; name: string };
/** A name in an object pattern: the property it reads. */
function patternKey(pattern: Node, name: string): string | null | undefined {
  if (pattern.type === 'Identifier') return pattern.name === name ? null : undefined;
  if (pattern.type === 'AssignmentPattern') return patternKey(pattern.left as Node, name);
  if (pattern.type === 'ObjectPattern') for (const property of pattern.properties as Node[]) {
    if (property.type === 'Property' && patternKey(property.value as Node, name) !== undefined) return keyName(property.key as Node) ?? undefined;
  }
  return undefined;
}
function declared(statements: Node[], name: string, file: File): Binding | null {
  for (let statement of statements) {
    if ((statement.type === 'ExportNamedDeclaration' || statement.type === 'ExportDefaultDeclaration') && statement.declaration) statement = statement.declaration as Node;
    if (statement.type === 'VariableDeclaration') for (const declarator of statement.declarations as Node[]) {
      const key = patternKey(declarator.id as Node, name);
      if (key !== undefined && declarator.init) return { kind: 'value', node: declarator.init as Node, file, ...key === null ? {} : { key } };
    }
    if (statement.type === 'FunctionDeclaration' && (statement.id as Node | null)?.name === name) return { kind: 'function', fn: statement, file };
    if (statement.type === 'ImportDeclaration') for (const specifier of statement.specifiers as Node[]) {
      if ((specifier.local as Node).name !== name) continue;
      const imported = specifier.type === 'ImportNamespaceSpecifier' ? '*' : specifier.type === 'ImportDefaultSpecifier' ? 'default' : keyName(specifier.imported as Node)!;
      return { kind: 'import', file, source: String((statement.source as Node).value), name: imported };
    }
  }
  return null;
}
/** The binding `name` has where `at` is written. */
function lookup(name: string, at: Node, file: File): Binding | null {
  for (let node: Node | undefined = at; node; node = file.parents.get(node)) {
    if (isFunction(node)) {
      const params = node.params as Node[];
      for (let index = 0; index < params.length; index++) {
        const param = params[index];
        const key = patternKey(param, name);
        if (key !== undefined) return { kind: 'param', fn: node, index, file, ...key === null ? {} : { key },
          ...param.type === 'AssignmentPattern' ? { fallback: param.right as Node } : {} };
      }
    }
    const body = node.type === 'Program' || node.type === 'BlockStatement' ? node.body as Node[] : null;
    if (body) {
      const found = declared(body, name, file);
      if (found) return found;
    }
  }
  return null;
}
/** A module's export `name`, as the value or function it declares. */
function exported(file: File, name: string): Binding | null {
  for (const statement of file.program.body as Node[]) {
    if (statement.type === 'ExportNamedDeclaration' && !statement.source) {
      if (statement.declaration) {
        const found = declared([statement.declaration as Node], name, file);
        if (found) return found;
      }
      for (const specifier of statement.specifiers as Node[] ?? []) {
        if (keyName(specifier.exported as Node) === name) return lookup(String((specifier.local as Node).name), statement, file);
      }
    }
  }
  return null;
}
function follow(binding: Binding | null): Binding | null {
  for (let depth = 0; binding?.kind === 'import' && depth < MAX_DEPTH; depth++) {
    const target = moduleAt(binding.file.path, binding.source);
    binding = target && binding.name !== '*' ? exported(target, binding.name) : null;
  }
  return binding;
}
/** The function a call reaches, with its module. */
function callee(node: Node, at: Scope): { fn: Node; file: File } | null {
  if (node.type === 'Identifier') {
    const binding = follow(lookup(String(node.name), node, at.file));
    if (binding?.kind === 'function') return binding;
    if (binding?.kind === 'value' && !binding.key && isFunction(binding.node)) return { fn: binding.node, file: binding.file };
    return null;
  }
  if (node.type === 'MemberExpression' && !node.computed && (node.object as Node).type === 'Identifier') {
    const space = lookup(String((node.object as Node).name), node, at.file);
    if (space?.kind !== 'import' || space.name !== '*') return null;
    const target = moduleAt(space.file.path, space.source);
    const binding = target && follow(exported(target, String((node.property as Node).name)));
    if (binding?.kind === 'function') return binding;
    if (binding?.kind === 'value' && !binding.key && isFunction(binding.node)) return { fn: binding.node, file: binding.file };
  }
  return null;
}
/** Every call of `fn`, by the name it is declared under in its module. */
const callsOf = new Map<Node, { call: Node; file: File }[]>();
{
  for (const file of files.values()) walk(file.program, node => {
    if (node.type !== 'CallExpression') return;
    const target = callee(node.callee as Node, { file, env: new Map() });
    if (!target) return;
    if (!callsOf.has(target.fn)) callsOf.set(target.fn, []);
    callsOf.get(target.fn)!.push({ call: node, file });
  });
}
const open = (files: Set<string>, truncated = false): Value => ({ open: true, truncated, files });
const plus = (files: Set<string>, file: string) => new Set([...files, file]);

/** The client the analysis follows: one with prepared queries, or one without. */
type Route = 'prepared' | 'product';
let route: Route = 'prepared';
const memos: Record<Route, Map<Node, Value[]>> = { prepared: new Map(), product: new Map() };
const pending = new Set<Node>();
/** Whether `test` holds exactly when the client has prepared queries (or exactly when it has not). */
function routed(test: Node, at: Scope, depth: number): boolean | null {
  if (depth > 4) return null;
  if (test.type === 'UnaryExpression' && test.operator === '!') {
    const inner = routed(test.argument as Node, at, depth + 1);
    return inner === null ? null : !inner;
  }
  if (test.type === 'ChainExpression') return routed(test.expression as Node, at, depth + 1);
  if (test.type === 'MemberExpression') return !test.computed && (test.property as Node).name === 'preparedQuery' ? true : null;
  if (test.type === 'LogicalExpression' && test.operator === '&&') {
    for (const side of [test.left, test.right] as Node[]) { const found = routed(side, at, depth + 1); if (found !== null) return found; }
    return null;
  }
  if (test.type === 'Identifier') {
    const binding = lookup(String(test.name), test, at.file);
    return binding?.kind === 'value' && !binding.key ? routed(binding.node, { file: binding.file, env: new Map() }, depth + 1) : null;
  }
  return null;
}
/** `at`, with the names the guard `test` finds truthy. */
function guarded(test: Node, at: Scope): Scope {
  const names = new Set(at.truthy);
  const collect = (node: Node) => {
    if (node.type === 'LogicalExpression' && node.operator === '&&') { collect(node.left as Node); collect(node.right as Node); }
    else if (node.type === 'Identifier') names.add(String(node.name));
  };
  collect(test);
  return names.size ? { ...at, truthy: names } : at;
}
/** The values `node` can take; a value defined through itself is one the source does not show. */
function values(node: Node, at: Scope, depth: number, seen: Set<string>): Value[] {
  if (at.env.size || at.truthy?.size || depth > MAX_DEPTH) return evaluate(node, at, depth, seen);
  if (pending.has(node)) return [open(plus(seen, at.file.path))];
  const memo = memos[route];
  if (!memo.has(node)) {
    pending.add(node);
    try { memo.set(node, evaluate(node, at, depth, new Set())); } finally { pending.delete(node); }
  }
  return memo.get(node)!.map(value => ({ ...value, files: new Set([...seen, ...value.files]) }));
}
function evaluate(node: Node, at: Scope, depth: number, seen: Set<string>): Value[] {
  const files = plus(seen, at.file.path);
  if (depth > MAX_DEPTH) return [open(files, true)];
  switch (node.type) {
    case 'TSAsExpression': case 'TSSatisfiesExpression': case 'TSNonNullExpression': case 'TSTypeAssertion': case 'ChainExpression':
      return values(node.expression as Node, at, depth + 1, files);
    case 'ConditionalExpression': {
      // A choice on the client's prepared queries takes the branch the template's route takes.
      const prepared = routed(node.test as Node, at, 0);
      if (prepared !== null) return values((prepared === (route === 'prepared') ? node.consequent : node.alternate) as Node, at, depth + 1, files);
      return [...values(node.consequent as Node, at, depth + 1, files), ...values(node.alternate as Node, at, depth + 1, files)];
    }
    case 'LogicalExpression': return node.operator === '&&' ? values(node.right as Node, guarded(node.left as Node, at), depth + 1, files)
      : [...values(node.left as Node, at, depth + 1, files), ...values(node.right as Node, at, depth + 1, files)];
    case 'Identifier': {
      if (node.name === 'undefined') return [{ node, at, files }];
      const binding = follow(lookup(String(node.name), node, at.file));
      if (!binding) return [open(files)];
      if (binding.kind === 'value') {
        const scope = { file: binding.file, env: binding.file === at.file ? at.env : new Map() };
        const found = binding.key ? property(values(binding.node, scope, depth + 1, files), binding.key, depth + 1) : values(binding.node, scope, depth + 1, files);
        // A name its guard found truthy is not one of its falsy values here.
        return at.truthy?.has(String(node.name)) ? found.filter(value => !absent(value) && !('node' in value && value.node.type === 'Literal' && !value.node.value)) : found;
      }
      if (binding.kind === 'param') {
        const bound = at.env.get(binding.fn);
        const argsOf = (args: Node[], scope: Scope, through: Set<string>): Value[] => {
          const arg = args[binding.index];
          const found = arg?.type === 'SpreadElement' ? [open(through)] : arg && !(arg.type === 'Identifier' && arg.name === 'undefined') ? values(arg, scope, depth + 1, through)
            : binding.fallback ? values(binding.fallback, { file: binding.file, env: at.env }, depth + 1, through) : [UNSET({ at: scope, files: through })];
          return binding.key ? property(found, binding.key, depth + 1) : found;
        };
        if (bound) return argsOf(bound.args, bound.at, files);
        const sites = callsOf.get(binding.fn) ?? [];
        if (!sites.length) return [open(files)];
        return sites.flatMap(site => argsOf(site.call.arguments as Node[], { file: site.file, env: new Map() }, plus(files, site.file.path)));
      }
      return [open(files)];
    }
    case 'CallExpression': {
      const memoised = (node.arguments as Node[])[0];
      // A memo's value is its factory's.
      if ((node.callee as Node).type === 'Identifier' && (node.callee as Node).name === 'useMemo' && memoised && isFunction(memoised)) {
        const body = memoised.body as Node;
        if (body.type !== 'BlockStatement') return values(body, at, depth + 1, files);
        const returned: Value[] = [];
        walk(body, inner => { if (inner.type === 'ReturnStatement' && inner.argument) returned.push(...values(inner.argument as Node, at, depth + 1, files)); });
        return returned;
      }
      const target = callee(node.callee as Node, at);
      if (!target) return [open(files)];
      const env: Env = new Map(at.env);
      env.set(target.fn, { args: node.arguments as Node[], at });
      const scope = { file: target.file, env };
      const body = target.fn.body as Node;
      if (body.type !== 'BlockStatement') return values(body, scope, depth + 1, files);
      const returned: Value[] = [];
      walk(body, inner => { if (inner.type === 'ReturnStatement' && inner.argument) returned.push(...values(inner.argument as Node, scope, depth + 1, files)); });
      return returned;
    }
    default: return [{ node, at, files }];
  }
}
/** The values of property `key` of each value. */
function property(found: Value[], key: string, depth: number): Value[] {
  return found.flatMap(value => {
    if ('open' in value) return [value];
    if (value.node.type !== 'ObjectExpression') return [open(value.files)];
    return objects(value, depth + 1).flatMap(object => {
      const field = object.fields.get(key);
      return field ? values(field.node, field.at, depth + 1, plus(object.files, field.at.file.path))
        : object.open ? [open(object.files, object.truncated)] : [UNSET({ at: value.at, files: object.files })];
    });
  });
}
/** One property's value expression where it is written. */
interface Field { node: Node; at: Scope }
/** One way an object literal can come out: its properties after its spreads, and the modules that choice read. */
interface ObjectValue { fields: Map<string, Field>; open: boolean; truncated: boolean; files: Set<string> }
const UNDEFINED: Node = { type: 'Identifier', name: 'undefined', start: -1 };
const UNSET = (value: { at: Scope; files: Set<string> }): Value => ({ node: UNDEFINED, at: value.at, files: value.files });
const MAX_ALTERNATIVES = 64;
/** The objects an object literal can be, one per choice of what each spread adds. */
function objects(value: Written, depth: number): ObjectValue[] {
  let found: ObjectValue[] = [{ fields: new Map(), open: false, truncated: false, files: new Set(value.files) }];
  for (const item of value.node.properties as Node[]) {
    if (item.type !== 'SpreadElement') {
      const key = item.computed ? null : keyName(item.key as Node);
      for (const object of found) if (key === null) object.open = true; else object.fields.set(key, { node: item.value as Node, at: value.at });
      continue;
    }
    const spreads: ObjectValue[] = values(item.argument as Node, value.at, depth + 1, value.files).flatMap(spread => {
      if ('open' in spread) return [{ fields: new Map(), open: true, truncated: spread.truncated, files: spread.files }];
      if (absent(spread) || spread.node.type === 'Literal' && spread.node.value === null) return [{ fields: new Map(), open: false, truncated: false, files: spread.files }];
      if (spread.node.type !== 'ObjectExpression') return [{ fields: new Map(), open: true, truncated: false, files: spread.files }];
      return objects(spread, depth + 1);
    });
    const next = found.flatMap(object => spreads.map(spread => ({ fields: new Map([...object.fields, ...spread.fields]),
      open: object.open || spread.open, truncated: object.truncated || spread.truncated, files: new Set([...object.files, ...spread.files]) })));
    found = next.length > MAX_ALTERNATIVES
      ? [{ fields: new Map(), open: true, truncated: true, files: new Set(next.flatMap(object => [...object.files])) }] : next;
  }
  return found;
}
const mayBeUndefined = (node: Node): boolean => node.type === 'Identifier' && node.name === 'undefined' || node.type === 'LogicalExpression'
  || node.type === 'ConditionalExpression' && (mayBeUndefined(node.consequent as Node) || mayBeUndefined(node.alternate as Node));
const absent = (value: Value) => !('open' in value) && (value.node.type === 'Identifier' && value.node.name === 'undefined');
/** The keys one params value sends. */
function shapes(found: Value[], depth: number): Shape[] {
  return found.flatMap(value => {
    if ('open' in value) return [{ required: new Set<string>(), optional: new Set<string>(), open: true, truncated: value.truncated, files: value.files }];
    if (absent(value) || value.node.type === 'Literal' && value.node.value === null) return [];
    if (value.node.type !== 'ObjectExpression') return [{ required: new Set<string>(), optional: new Set<string>(), open: true, truncated: false, files: value.files }];
    return objects(value, depth + 1).map(object => {
      const shape: Shape = { required: new Set(), optional: new Set(), open: object.open, truncated: object.truncated, files: new Set(object.files) };
      for (const [key, field] of object.fields) {
        // A name that is only ever undefined is not sent; any other key is sent unless written to be undefined at times.
        const taken = field.node.type === 'Identifier' ? values(field.node, field.at, depth + 1, new Set()) : [];
        if (taken.length && taken.every(absent)) continue;
        (mayBeUndefined(field.node) ? shape.optional : shape.required).add(key);
      }
      return shape;
    });
  });
}
const products = (found: Value[]): (string | null)[] => found.map(value => !('open' in value) && value.node.type === 'Literal' && typeof value.node.value === 'string'
  ? value.node.value : !('open' in value) && value.node.type === 'TemplateLiteral' && !(value.node.expressions as Node[]).length
    ? String(((value.node.quasis as Node[])[0].value as { cooked: string }).cooked) : null);

/** The requests each module writes: objects with a product and params, and calls passing a product then its params. */
/** Whether `node` is a widget's availability check: the dashboard completes and sends those itself. */
function availability(node: Node, file: File): boolean {
  const parent = file.parents.get(node);
  return parent?.type === 'Property' && keyName(parent.key as Node) === 'availability'
    || parent?.type === 'TSAsExpression' && file.parents.get(parent)?.type === 'VariableDeclarator' && /AVAILABILITY/.test(String(((file.parents.get(parent)!).id as Node).name));
}
/** The scopes a request written at `node` is built in: one per call of the function holding it, so
 * a request helper's product and params come from the same call. */
function scopesAt(node: Node, file: File): Scope[] {
  let fn: Node | undefined = file.parents.get(node);
  // Guards between the request and its function: `a && b && request`, `a ? request : …`.
  let guards: Scope = { file, env: new Map() };
  for (let child = node; fn && !isFunction(fn); child = fn, fn = file.parents.get(fn)) {
    if (fn.type === 'LogicalExpression' && fn.operator === '&&' && fn.right === child) guards = guarded(fn.left as Node, guards);
    if (fn.type === 'ConditionalExpression' && fn.consequent === child) guards = guarded(fn.test as Node, guards);
  }
  const sites = fn && (fn.params as Node[]).length ? callsOf.get(fn) ?? [] : [];
  return sites.length ? sites.map(site => ({ ...guards, env: new Map([[fn!, { args: site.call.arguments as Node[], at: { file: site.file, env: new Map() } }]]) }))
    : [guards];
}
function writtenFor(followed: Route): Map<string, Request[]> {
  route = followed;
  const written = new Map<string, Request[]>();
  for (const file of files.values()) {
  const found: Request[] = [];
  walk(file.program, node => {
    if (node.type !== 'ObjectExpression' && node.type !== 'CallExpression' || availability(node, file)) return;
    for (const at of scopesAt(node, file)) {
    const through = new Set([file.path, ...[...at.env.values()].map(bound => bound.at.file.path)]);
    if (node.type === 'ObjectExpression') {
      for (const object of objects({ node, at, files: through }, 0)) {
        const product = object.fields.get('product');
        const params = object.fields.get('params');
        if (!product || !params) continue;
        const named = values(product.node, product.at, 1, object.files);
        const names = products(named);
        for (const shape of shapes(values(params.node, params.at, 1, object.files), 0)) {
          names.forEach((name, index) => found.push({ product: name, shape: { ...shape, files: new Set([...shape.files, ...named[index].files]) }, file: file.path }));
        }
      }
    } else if (node.type === 'CallExpression') {
      const args = node.arguments as Node[];
      args.forEach((arg, index) => {
        if (arg.type !== 'Literal' || typeof arg.value !== 'string' || index + 1 >= args.length || args[index + 1].type === 'SpreadElement') return;
        for (const shape of shapes(values(args[index + 1], at, 0, through), 0)) found.push({ product: arg.value, shape, file: file.path });
      });
    }
    }
  });
  written.set(file.path, found);
  }
  return written;
}
const written: Record<Route, Map<string, Request[]>> = { prepared: writtenFor('prepared'), product: writtenFor('product') };
/** The requests a widget's own modules write, each only through modules the widget loads. */
function widgetRequests(kind: string, followed: Route): Request[] {
  const entry = loaders.get(kind);
  assert.ok(entry, `${kind} has no renderer loader`);
  const modules = reachable(entry);
  return [...modules].flatMap(path => written[followed].get(path) ?? []).filter(request => [...request.shape.files].every(file => modules.has(file)));
}

function fits(shape: Shape, keys: readonly string[], unsent: readonly string[] = []): boolean {
  return (shape.open || keys.every(key => shape.required.has(key) || shape.optional.has(key)))
    && [...shape.required].every(key => keys.includes(key) || unsent.includes(key));
}
/** Why `template` matches none of `kind`'s requests, or null when one sends exactly its keys. */
function mismatch(kind: string, template: Template, requests: Request[]): string | null {
  const keys = Object.keys(template.params);
  const unsent = [...UNSENT[kind] ?? [], ...REPLY_BOUND[kind] ?? []];
  const named = requests.filter(request => request.product === template.product);
  const closed = named.filter(request => !request.shape.open);
  if (named.some(request => request.shape.truncated)) return `${kind}: ${template.product}'s requests reach the resolution depth cap`;
  if (closed.some(request => fits(request.shape, keys, unsent))) return null;
  if (loose(kind, template.product) && [...named, ...requests.filter(request => request.product === null)].some(request => fits(request.shape, keys, unsent))) return null;
  const listed = named.map(request => `{${[...request.shape.required].join(', ')}${request.shape.optional.size ? ` | ${[...request.shape.optional].join(', ')}` : ''}${request.shape.open ? ', …' : ''}}`);
  return `${kind}: ${template.product} {${keys.join(', ')}} ${named.length ? `matches no request its modules write: ${listed.join(' ')}` : 'is never requested by its modules'}`;
}
const keyChecked = () => table.widgets.filter(widget => !DEVIATES.has(widget.kind)).flatMap(widget =>
  [...EXACT[widget.kind] ? [] : widget.requests ?? [], ...widget.auxiliary ?? []].map(template => ({ kind: widget.kind, template })));

test('every readiness entry lists requests unless named, and binds only recording facts the probe knows', () => {
  for (const widget of table.widgets) {
    const listed = [...widget.requests ?? [], ...widget.auxiliary ?? []];
    assert.equal(listed.length > 0, !NO_REQUESTS.has(widget.kind), `${widget.kind} ${listed.length ? 'lists requests but is named as having none' : 'lists no requests'}`);
    for (const template of listed) bind(template.params);
  }
  for (const kind of [...NO_REQUESTS, ...DEVIATES, ...Object.keys(EXACT), ...Object.keys(UNSENT), ...Object.keys(REPLY_BOUND), ...Object.keys(LOOSE),
    ...Object.keys(SHOWN), ...Object.keys(SHOWN_DAY)]) {
    assert.ok(table.widgets.some(widget => widget.kind === kind), `${kind} is named but has no readiness entry`);
  }
});

test('builder-made default requests match the readiness table exactly', () => {
  for (const track of TRACKS) {
    for (const [kind, requests] of Object.entries(EXACT)) {
      const entry = table.widgets.find(widget => widget.kind === kind)!;
      const widget = createWidget(kind as WidgetSpec['kind'], track.id);
      const window = resolveWindow(widget, track, [track], track.branch.firstHour);
      assert.ok(window, `${kind} has no window at the opening hour`);
      const sent = requests(widget, track, window).map(request => {
        assert.ok(request, `${kind} sends no request on its first render`);
        return JSON.parse(JSON.stringify({ product: request.product, params: request.params }));
      });
      const listed = (entry.requests ?? []).map(template => ({ product: template.product, params: bind(template.params, bindings(track)) }));
      assert.deepEqual(listed, sent, `${kind}'s readiness requests differ from its first render on a ${track.branch.lastHour - track.branch.firstHour + 1}-hour branch`);
    }
  }
});

test('readiness requests read the hour and day each widget shows at its default time', () => {
  for (const track of TRACKS) {
    const facts = bindings(track);
    const { firstHour: first, lastHour: last } = track.branch;
    const shown = (kind: string) => built.shownHour(track, resolveWindow(createWidget(kind, track.id), track, [track], first));
    const at = (kind: string, path: string) => {
      const raw = valueAt(table.widgets.find(widget => widget.kind === kind)!.requests, path);
      const window = typeof raw === 'string' ? /^\$window(?:Last)?:\d+:(first|last)$/.exec(raw) : null;
      return window ? facts[`$${window[1]}`] : bind(raw, facts);
    };
    const branch = `on a ${last - first + 1}-hour branch`;
    for (const [kind, paths] of Object.entries(SHOWN)) {
      for (const path of paths) assert.equal(at(kind, path), shown(kind), `${kind}'s ${path} is not the hour it shows ${branch}`);
    }
    for (const [kind, [from, to]] of Object.entries(SHOWN_DAY)) {
      const start = first + Math.floor((shown(kind) - first) / 24) * 24;
      assert.deepEqual([at(kind, from), at(kind, to)], [start, Math.min(start + 23, last)], `${kind} does not read the day it shows ${branch}`);
    }
  }
});

test('every other readiness request is one its widget\'s own modules write, with the same param keys', () => {
  const failures = keyChecked().flatMap(({ kind, template }) => mismatch(kind, template, widgetRequests(kind, template.route as Route)) ?? []);
  assert.deepEqual(failures, []);
});

test('the key check refuses every template it passes, other than a loose one, once it gains a key', () => {
  const lax = keyChecked().filter(({ kind, template }) => {
    const requests = widgetRequests(kind, template.route as Route);
    return !loose(kind, template.product) && !mismatch(kind, template, requests) && !mismatch(kind, { ...template, params: { ...template.params, unsentDrift: 1 } }, requests);
  }).map(({ kind, template }) => `${kind}: ${template.product}`);
  assert.deepEqual(lax, []);
});

test('every reserve race mode uses the same existence-filtered product contract', () => {
  const widget = createWidget('occupation-reserve-manpower', 'main');
  const selections: WidgetSpec['countries'][] = [
    { kind: 'all' }, { kind: 'selected' }, { kind: 'tags', tags: ['GER'] },
    { kind: 'group', group: { type: 'top', count: 10 }, membership: 'hour', aggregate: false },
    { kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false },
  ];
  for (const countries of selections) {
    const request = built.occupationReserveRequest({ ...widget, countries }, TRACKS[0]);
    assert.deepEqual(request?.params, { branchId: TRACKS[0].branch.id, view: 'race',
      ...countries.kind === 'tags' ? { tags: ['GER'] } : {} });
  }
});
