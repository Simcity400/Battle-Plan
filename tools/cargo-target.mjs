import { execFile } from 'node:child_process';
import { readFile, realpath, stat } from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';

const execFileAsync = promisify(execFile);
const UTF8 = new TextDecoder('utf-8', { fatal: true });
const BARE_KEY = /[A-Za-z0-9_-]+/y;
const DATE_TIME = /\d{4}-\d{2}-\d{2}(?:[Tt ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:[Zz]|[+-]\d{2}:\d{2})?)?(?=$|[\t \r\n#,\}\]])/y;
const LOCAL_TIME = /\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?=$|[\t \r\n#,\}\]])/y;
const RAW_VALUE = /[^\t \r\n#,\}\]]+/y;
const HEX_INT = /^0x[0-9A-Fa-f](?:_?[0-9A-Fa-f])*$/;
const OCT_INT = /^0o[0-7](?:_?[0-7])*$/;
const BIN_INT = /^0b[01](?:_?[01])*$/;
const DEC_INT = /^[+-]?(?:0|[1-9](?:_?[0-9])*)$/;
const SPECIAL_FLOAT = /^[+-]?(?:inf|nan)$/;
const DEC_FLOAT = /^[+-]?(?:0|[1-9](?:_?[0-9])*)(?:\.[0-9](?:_?[0-9])*(?:[eE][+-]?[0-9](?:_?[0-9])*)?|[eE][+-]?[0-9](?:_?[0-9])*)$/;
const HEX_DIGITS = /^[0-9A-Fa-f]+$/;
const OVERRIDE_FS_ERRORS = new Set(['ENOENT', 'ENOTDIR', 'EINVAL', 'ENAMETOOLONG']);
const SKIP_VALUE_FLAGS = new Set(['--profile', '--target', '-t', '--target-dir']);

function isTomlTable(value) {
  return typeof value === 'object' && value !== null && Object.getPrototypeOf(value) === null;
}

function parseToml(input, source, { inlineOverride = false } = {}) {
  const text = !inlineOverride && input.startsWith('\uFEFF') ? input.slice(1) : input;
  const len = text.length;
  let pos = 0;
  const tableMeta = new WeakMap();
  const arrayTables = new WeakSet();

  function fail(message) {
    throw new Error(`Invalid TOML in ${source}: ${message}`);
  }

  function newTable(flags = {}) {
    const table = Object.create(null);
    tableMeta.set(table, { explicit: false, dotted: false, inline: false, ...flags });
    return table;
  }

  function skipWs() {
    while (pos < len && (text[pos] === ' ' || text[pos] === '\t')) pos++;
  }

  function consumeNewline() {
    if (text[pos] === '\n') { pos++; return true; }
    if (text[pos] === '\r') {
      if (text[pos + 1] === '\n') { pos += 2; return true; }
      fail('bare carriage return');
    }
    return false;
  }

  function skipWsAndNewlines() {
    while (pos < len) {
      if (text[pos] === ' ' || text[pos] === '\t') pos++;
      else if (!consumeNewline()) break;
    }
  }

  function skipComment() {
    if (text[pos] !== '#') return;
    pos++;
    while (pos < len && text[pos] !== '\n' && text[pos] !== '\r') {
      const code = text.charCodeAt(pos++);
      if ((code < 0x20 && code !== 0x09) || code === 0x7F) fail('control character in comment');
    }
  }

  function skipWsAndComments() {
    while (pos < len) {
      skipWs();
      if (text[pos] === '#') { skipComment(); continue; }
      if (consumeNewline()) continue;
      break;
    }
  }

  function expectLineEnd() {
    skipWs();
    skipComment();
    if (pos < len && !consumeNewline()) fail('unexpected trailing token');
  }

  function parseUnicodeEscape(digits) {
    const hex = text.slice(pos, pos + digits);
    if (hex.length !== digits || !HEX_DIGITS.test(hex)) fail('invalid unicode escape');
    pos += digits;
    const codePoint = parseInt(hex, 16);
    if (codePoint > 0x10FFFF || (codePoint >= 0xD800 && codePoint <= 0xDFFF)) fail('invalid unicode scalar value');
    return String.fromCodePoint(codePoint);
  }

  function parseEscape() {
    if (pos >= len) fail('unterminated escape sequence');
    const ch = text[pos++];
    switch (ch) {
      case 'b': return '\b';
      case 't': return '\t';
      case 'n': return '\n';
      case 'f': return '\f';
      case 'r': return '\r';
      case 'e': return '\u001B';
      case '"': return '"';
      case '\\': return '\\';
      case 'x': return parseUnicodeEscape(2);
      case 'u': return parseUnicodeEscape(4);
      case 'U': return parseUnicodeEscape(8);
      default: fail(`invalid escape \\${ch}`);
    }
  }

  function parseBasicString() {
    pos++;
    let out = '';
    while (pos < len) {
      const ch = text[pos++];
      if (ch === '"') return out;
      if (ch === '\\') { out += parseEscape(); continue; }
      const code = ch.charCodeAt(0);
      if ((code < 0x20 && code !== 0x09) || code === 0x7F) fail('control character in string');
      out += ch;
    }
    fail('unterminated basic string');
  }

  function parseLiteralString() {
    pos++;
    let out = '';
    while (pos < len) {
      const ch = text[pos++];
      if (ch === "'") return out;
      const code = ch.charCodeAt(0);
      if ((code < 0x20 && code !== 0x09) || code === 0x7F) fail('control character in literal string');
      out += ch;
    }
    fail('unterminated literal string');
  }

  function parseMultilineBasicString() {
    pos += 3;
    consumeNewline();
    let out = '';
    while (pos < len) {
      if (text[pos] === '"') {
        let quotes = 0;
        while (text[pos + quotes] === '"') quotes++;
        pos += quotes;
        if (quotes < 3) { out += '"'.repeat(quotes); continue; }
        if (quotes <= 5) return out + '"'.repeat(quotes - 3);
        fail('too many consecutive quotes in multiline string');
      }
      if (text[pos] === '\\') {
        pos++;
        let look = pos;
        while (look < len && (text[look] === ' ' || text[look] === '\t')) look++;
        if (text[look] === '\n' || (text[look] === '\r' && text[look + 1] === '\n')) {
          pos = look;
          skipWsAndNewlines();
          continue;
        }
        if (look > pos) fail('invalid whitespace after backslash');
        out += parseEscape();
        continue;
      }
      if (text[pos] === '\r') {
        if (text[pos + 1] !== '\n') fail('bare carriage return in string');
        out += '\r\n';
        pos += 2;
        continue;
      }
      const ch = text[pos++];
      const code = ch.charCodeAt(0);
      if ((code < 0x20 && code !== 0x09 && code !== 0x0A) || code === 0x7F) fail('control character in multiline string');
      out += ch;
    }
    fail('unterminated multiline basic string');
  }

  function parseMultilineLiteralString() {
    pos += 3;
    consumeNewline();
    let out = '';
    while (pos < len) {
      if (text[pos] === "'") {
        let quotes = 0;
        while (text[pos + quotes] === "'") quotes++;
        pos += quotes;
        if (quotes < 3) { out += "'".repeat(quotes); continue; }
        if (quotes <= 5) return out + "'".repeat(quotes - 3);
        fail('too many consecutive quotes in multiline literal string');
      }
      if (text[pos] === '\r') {
        if (text[pos + 1] !== '\n') fail('bare carriage return in literal string');
        out += '\r\n';
        pos += 2;
        continue;
      }
      const ch = text[pos++];
      const code = ch.charCodeAt(0);
      if ((code < 0x20 && code !== 0x09 && code !== 0x0A) || code === 0x7F) fail('control character in multiline literal string');
      out += ch;
    }
    fail('unterminated multiline literal string');
  }

  function parseSimpleKey() {
    if (text.startsWith('"""', pos) || text.startsWith("'''", pos)) fail('multiline strings are not valid keys');
    if (text[pos] === '"') return parseBasicString();
    if (text[pos] === "'") return parseLiteralString();
    BARE_KEY.lastIndex = pos;
    const match = BARE_KEY.exec(text);
    if (!match) fail('expected key');
    pos = BARE_KEY.lastIndex;
    return match[0];
  }

  function parseKeyPath() {
    const keys = [parseSimpleKey()];
    while (true) {
      skipWs();
      if (text[pos] !== '.') break;
      pos++;
      skipWs();
      keys.push(parseSimpleKey());
    }
    return keys;
  }

  function assignKey(table, keys, value) {
    let current = table;
    for (let index = 0; index < keys.length - 1; index++) {
      const key = keys[index];
      if (!(key in current)) {
        current[key] = newTable({ dotted: true });
      } else {
        const state = tableMeta.get(current[key]);
        if (!state || state.inline || state.explicit) fail(`cannot extend key ${key}`);
        state.dotted = true;
      }
      current = current[key];
    }
    const leaf = keys[keys.length - 1];
    if (leaf in current) fail(`duplicate key ${leaf}`);
    current[leaf] = value;
  }

  function parseArray() {
    pos++;
    const items = [];
    skipWsAndComments();
    if (text[pos] === ']') { pos++; return items; }
    while (true) {
      items.push(parseValue());
      skipWsAndComments();
      if (text[pos] === ',') {
        pos++;
        skipWsAndComments();
        if (text[pos] === ']') { pos++; return items; }
        continue;
      }
      if (text[pos] === ']') { pos++; return items; }
      fail('expected comma or closing bracket in array');
    }
  }

  function parseInlineTable() {
    pos++;
    const table = newTable({ inline: true });
    skipWsAndComments();
    if (text[pos] === '}') { pos++; return table; }
    while (true) {
      const keys = parseKeyPath();
      skipWs();
      if (text[pos] !== '=') fail('expected = in inline table');
      pos++;
      skipWs();
      assignKey(table, keys, parseValue());
      skipWsAndComments();
      if (text[pos] === ',') {
        pos++;
        skipWsAndComments();
        if (text[pos] === '}') { pos++; return table; }
        continue;
      }
      if (text[pos] === '}') { pos++; return table; }
      fail('expected comma or closing brace in inline table');
    }
  }

  function parsePrimitive() {
    DATE_TIME.lastIndex = pos;
    if (DATE_TIME.exec(text)) fail('date/time values are not supported in Cargo configuration');
    LOCAL_TIME.lastIndex = pos;
    if (LOCAL_TIME.exec(text)) fail('date/time values are not supported in Cargo configuration');
    RAW_VALUE.lastIndex = pos;
    const raw = RAW_VALUE.exec(text);
    if (!raw) fail('expected value');
    pos = RAW_VALUE.lastIndex;
    const token = raw[0];
    if (token === 'true') return true;
    if (token === 'false') return false;
    if (HEX_INT.test(token) || OCT_INT.test(token) || BIN_INT.test(token)) return BigInt(token.replaceAll('_', ''));
    if (DEC_INT.test(token)) return Number(token.replaceAll('_', ''));
    if (SPECIAL_FLOAT.test(token)) return token.endsWith('nan') ? Number.NaN : token.startsWith('-') ? -Infinity : Infinity;
    if (DEC_FLOAT.test(token)) return Number(token.replaceAll('_', ''));
    fail(`invalid value ${token}`);
  }

  function parseValue() {
    if (text.startsWith('"""', pos)) return parseMultilineBasicString();
    if (text[pos] === '"') return parseBasicString();
    if (text.startsWith("'''", pos)) return parseMultilineLiteralString();
    if (text[pos] === "'") return parseLiteralString();
    if (text[pos] === '[') return parseArray();
    if (text[pos] === '{') return parseInlineTable();
    return parsePrimitive();
  }

  function openTableHeader(keys, isArray) {
    let current = root;
    for (let index = 0; index < keys.length - 1; index++) {
      const key = keys[index];
      if (!(key in current)) {
        current[key] = newTable();
        current = current[key];
      } else if (arrayTables.has(current[key])) {
        const list = current[key];
        current = list[list.length - 1];
      } else {
        const state = tableMeta.get(current[key]);
        if (!state || state.inline) fail(`cannot define subtable under ${key}`);
        current = current[key];
      }
    }
    const leaf = keys[keys.length - 1];
    if (isArray) {
      if (!(leaf in current)) {
        const list = [];
        arrayTables.add(list);
        current[leaf] = list;
      } else if (!arrayTables.has(current[leaf])) {
        fail(`cannot redefine ${leaf} as an array of tables`);
      }
      const entry = newTable({ explicit: true });
      current[leaf].push(entry);
      return entry;
    }
    if (!(leaf in current)) {
      current[leaf] = newTable({ explicit: true });
      return current[leaf];
    }
    const existing = current[leaf];
    const state = tableMeta.get(existing);
    if (!state || state.explicit || state.dotted || state.inline) fail(`duplicate table ${leaf}`);
    state.explicit = true;
    return existing;
  }

  const root = newTable({ explicit: true });
  if (inlineOverride) {
    skipWsAndNewlines();
    if (pos >= len) fail('empty override');
    if (text[pos] === '[') fail('table headers are not accepted in --config');
    if (text[pos] === '#') fail('comments are not accepted in --config');
    const keys = parseKeyPath();
    skipWs();
    if (text[pos] !== '=') fail('expected =');
    pos++;
    skipWs();
    if (text[pos] === '{') fail('inline tables are not accepted in --config');
    assignKey(root, keys, parseValue());
    skipWsAndNewlines();
    if (pos < len) fail('unexpected trailing token in --config');
    return root;
  }

  let current = root;
  while (true) {
    skipWsAndComments();
    if (pos >= len) break;
    if (text[pos] === '[') {
      const isArray = text[pos + 1] === '[';
      pos += isArray ? 2 : 1;
      skipWs();
      const keys = parseKeyPath();
      skipWs();
      if (isArray) {
        if (text[pos] !== ']' || text[pos + 1] !== ']') fail('expected ]]');
        pos += 2;
      } else {
        if (text[pos] !== ']') fail('expected ]');
        pos++;
      }
      expectLineEnd();
      current = openTableHeader(keys, isArray);
      continue;
    }
    const keys = parseKeyPath();
    skipWs();
    if (text[pos] !== '=') fail('expected =');
    pos++;
    skipWs();
    assignKey(current, keys, parseValue());
    expectLineEnd();
  }
  return root;
}

function unplaceable(source) {
  return new Error(`${source} names build targets this command cannot place; pass --target.`);
}

function normalizeTarget(value, source) {
  if (typeof value !== 'string') throw unplaceable(source);
  const trimmed = value.trim();
  if (trimmed.length === 0) throw unplaceable(source);
  return trimmed;
}

async function readUtf8File(file) {
  const bytes = await readFile(file);
  try {
    return UTF8.decode(bytes);
  } catch (error) {
    throw new Error(`Invalid UTF-8 in ${file}: ${error.message}`);
  }
}

async function canonicalPathKey(file) {
  const resolved = await realpath(file).catch(() => path.resolve(file));
  return process.platform === 'win32' ? resolved.toLowerCase() : resolved;
}

function extractDocumentTarget(document, source) {
  if (!('build' in document)) return null;
  const build = document.build;
  if (!isTomlTable(build)) throw unplaceable(source);
  if (!('target' in build)) return null;
  const target = build.target;
  if (typeof target === 'string') return { kind: 'string', value: target, source };
  if (Array.isArray(target)) return { kind: 'array', values: target, source };
  if (isTomlTable(target)) throw unplaceable(source);
  return { kind: 'scalar', source };
}

function isScalarTarget(entry) {
  return entry.kind === 'string' || entry.kind === 'scalar';
}

function mergeTarget(previous, next) {
  if (next === null) return previous;
  if (previous === null) return next;
  if (isScalarTarget(previous) && isScalarTarget(next)) return next;
  if (previous.kind === 'array' && next.kind === 'array') {
    return { kind: 'array', values: [...previous.values, ...next.values], source: next.source };
  }
  throw unplaceable(next.source);
}

async function loadIncludes(includeValue, baseDir, source, state, visitedIncludes) {
  if (!Array.isArray(includeValue)) {
    throw new Error(`${source} has an invalid include directive; expected an array of paths or include tables.`);
  }
  let merged = state;
  let defined = false;
  for (const entry of includeValue) {
    let includePath;
    let optional = false;
    if (typeof entry === 'string') {
      includePath = entry;
    } else if (isTomlTable(entry)) {
      if (typeof entry.path !== 'string') {
        throw new Error(`${source} has an invalid include entry; expected a string path.`);
      }
      if ('optional' in entry && typeof entry.optional !== 'boolean') {
        throw new Error(`${source} has an invalid include entry; optional must be a boolean.`);
      }
      includePath = entry.path;
      optional = entry.optional ?? false;
    } else {
      throw new Error(`${source} has an invalid include entry; expected a path string or table.`);
    }
    if (includePath.length === 0) {
      throw new Error(`${source} has an empty include path.`);
    }
    if (!includePath.endsWith('.toml')) {
      throw new Error(`${source} has an invalid include path ${includePath}; Cargo config includes must end with .toml.`);
    }
    const result = await loadFileLayer(path.resolve(baseDir, includePath), merged, visitedIncludes, { optional });
    merged = result.state;
    defined = defined || result.defined;
  }
  return { state: merged, defined };
}

async function loadFileLayer(file, state, visitedIncludes, { optional = false } = {}) {
  const resolved = path.resolve(file);
  let text;
  try {
    text = await readUtf8File(resolved);
  } catch (error) {
    if (optional && error?.code === 'ENOENT') return { state, defined: false };
    throw error;
  }
  const key = await canonicalPathKey(resolved);
  if (visitedIncludes.has(key)) {
    throw new Error(`Duplicate or circular Cargo config include involving ${resolved}.`);
  }
  const document = parseToml(text, resolved);
  visitedIncludes.add(key);
  let merged = state;
  let defined = false;
  if ('include' in document) {
    const included = await loadIncludes(document.include, path.dirname(resolved), resolved, merged, visitedIncludes);
    merged = included.state;
    defined = included.defined;
  }
  const own = extractDocumentTarget(document, resolved);
  if (own !== null) defined = true;
  return { state: mergeTarget(merged, own), defined };
}

async function findFolderConfig(folder) {
  for (const name of ['config', 'config.toml']) {
    const file = path.join(folder, name);
    try {
      if ((await stat(file)).isFile()) return file;
    } catch (error) {
      if (error?.code === 'ENOENT' || error?.code === 'ENOTDIR') continue;
      throw error;
    }
  }
  return null;
}

async function findOverrideFile(cwd, override) {
  if (!override || /[\r\n\0]/.test(override)) return null;
  const file = path.resolve(cwd, override);
  try {
    return (await stat(file)).isFile() ? file : null;
  } catch (error) {
    if (OVERRIDE_FS_ERRORS.has(error?.code)) return null;
    throw error;
  }
}

export async function resolveCargoTarget({ cwd, home, env, cargoArgs }) {
  const folders = [];
  const seenFolders = new Set();
  const addFolder = (folder) => {
    const resolved = path.resolve(folder);
    const key = process.platform === 'win32' ? resolved.toLowerCase() : resolved;
    if (seenFolders.has(key)) return;
    seenFolders.add(key);
    folders.push(resolved);
  };
  const resolvedCwd = path.resolve(cwd);
  for (let at = resolvedCwd; ; at = path.dirname(at)) {
    addFolder(path.join(at, '.cargo'));
    if (path.dirname(at) === at) break;
  }
  if (home) addFolder(path.resolve(resolvedCwd, home));

  let tomlState = null;
  for (const folder of folders.reverse()) {
    const file = await findFolderConfig(folder);
    if (file !== null) {
      tomlState = (await loadFileLayer(file, tomlState, new Set())).state;
    }
  }

  const overrides = [];
  for (let index = 0; index < cargoArgs.length; index++) {
    const arg = cargoArgs[index];
    if (arg === '--') break;
    if (arg === '--config') {
      if (index + 1 >= cargoArgs.length) throw new Error('--config needs a value.');
      overrides.push(cargoArgs[++index]);
    } else if (arg.startsWith('--config=')) {
      overrides.push(arg.slice('--config='.length));
    } else if (SKIP_VALUE_FLAGS.has(arg) && index + 1 < cargoArgs.length) {
      index++;
    }
  }

  let cliSetTarget = false;
  for (const override of overrides) {
    const file = await findOverrideFile(resolvedCwd, override);
    if (file !== null) {
      const result = await loadFileLayer(file, tomlState, new Set());
      tomlState = result.state;
      cliSetTarget = cliSetTarget || result.defined;
      continue;
    }
    const document = parseToml(override, '--config', { inlineOverride: true });
    if ('include' in document) {
      const included = await loadIncludes(document.include, resolvedCwd, '--config', tomlState, new Set());
      tomlState = included.state;
      cliSetTarget = cliSetTarget || included.defined;
    }
    const own = extractDocumentTarget(document, '--config');
    if (own !== null) cliSetTarget = true;
    tomlState = mergeTarget(tomlState, own);
  }

  const envTarget = Object.hasOwn(env, 'CARGO_BUILD_TARGET') && env.CARGO_BUILD_TARGET !== undefined
    ? env.CARGO_BUILD_TARGET
    : undefined;

  if (cliSetTarget) {
    if (tomlState.kind !== 'string') throw unplaceable(tomlState.source);
    return { target: normalizeTarget(tomlState.value, tomlState.source), source: tomlState.source };
  }
  if (envTarget !== undefined) {
    if (tomlState?.kind === 'array') throw unplaceable(tomlState.source);
    return { target: normalizeTarget(envTarget, 'CARGO_BUILD_TARGET'), source: 'CARGO_BUILD_TARGET' };
  }
  if (tomlState === null) return undefined;
  if (tomlState.kind !== 'string') throw unplaceable(tomlState.source);
  return { target: normalizeTarget(tomlState.value, tomlState.source), source: tomlState.source };
}

export async function cargoTarget(options) {
  return (await resolveCargoTarget(options))?.target;
}

export function parseHostTuple(rustcVersionOutput) {
  const match = rustcVersionOutput.match(/^host:\s*(\S+)\s*$/m);
  if (!match) throw new Error('rustc -vV did not report a host target triple.');
  return match[1];
}

export async function readHostTuple(env = process.env) {
  const rustc = env.RUSTC || 'rustc';
  const { stdout } = await execFileAsync(rustc, ['-vV'], { env, windowsHide: true });
  return parseHostTuple(stdout);
}

export async function resolveTargetPlacement({ target, source, hostTarget = () => readHostTuple() }) {
  const normalized = normalizeTarget(target, source);
  if (normalized === 'host-tuple') {
    const host = await hostTarget();
    if (typeof host !== 'string' || host.trim().length === 0) {
      throw new Error(`Could not resolve host-tuple target from ${source}.`);
    }
    const triple = host.trim();
    return { targetArg: triple, outputDirName: triple };
  }
  if (normalized.endsWith('.json')) {
    throw new Error(`${source} names a JSON target spec (${normalized}), which desktop staging does not support; pass a built-in target triple with --target.`);
  }
  return { targetArg: normalized, outputDirName: normalized };
}
