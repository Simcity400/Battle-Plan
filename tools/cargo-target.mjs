import { readFile, stat } from 'node:fs/promises';
import path from 'node:path';

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

function parseToml(input, source, { requireEntry = false } = {}) {
  const text = input.startsWith('\uFEFF') ? input.slice(1) : input;
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
          while (pos < len) {
            if (text[pos] === ' ' || text[pos] === '\t') pos++;
            else if (!consumeNewline()) break;
          }
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
    const dateTime = DATE_TIME.exec(text);
    if (dateTime) { pos = DATE_TIME.lastIndex; return { tomlDateTime: dateTime[0] }; }
    LOCAL_TIME.lastIndex = pos;
    const localTime = LOCAL_TIME.exec(text);
    if (localTime) { pos = LOCAL_TIME.lastIndex; return { tomlDateTime: localTime[0] }; }
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
  let current = root;
  let statements = 0;
  while (true) {
    skipWsAndComments();
    if (pos >= len) break;
    statements++;
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
  if (requireEntry && statements === 0) fail('empty override');
  return root;
}

function named(text, source, options) {
  const document = parseToml(text, source, options);
  if (!('build' in document)) return undefined;
  const build = document.build;
  if (typeof build !== 'object' || build === null || Object.getPrototypeOf(build) !== null) {
    throw new Error(`${source} names build targets this command cannot place; pass --target.`);
  }
  if (!('target' in build)) return undefined;
  const target = build.target;
  if (typeof target !== 'string' || target.length === 0) {
    throw new Error(`${source} names build targets this command cannot place; pass --target.`);
  }
  return target;
}

async function readOverrideFile(cwd, override) {
  if (!override || /[\r\n\0]/.test(override)) return null;
  const file = path.resolve(cwd, override);
  try {
    if (!(await stat(file)).isFile()) return null;
    return { file, text: await readFile(file, 'utf8') };
  } catch (error) {
    if (OVERRIDE_FS_ERRORS.has(error?.code)) return null;
    throw error;
  }
}

async function folderTarget(folder) {
  for (const name of ['config', 'config.toml']) {
    const file = path.join(folder, name);
    try {
      const text = await readFile(file, 'utf8');
      return named(text, file);
    } catch (error) {
      if (error?.code === 'ENOENT' || error?.code === 'ENOTDIR') continue;
      throw error;
    }
  }
  return undefined;
}

export async function cargoTarget({ cwd, home, env, cargoArgs }) {
  const overrides = [];
  for (let index = 0; index < cargoArgs.length; index++) {
    const arg = cargoArgs[index];
    if (arg === '--') break;
    if (arg === '--config' && index + 1 < cargoArgs.length) overrides.push(cargoArgs[++index]);
    else if (arg.startsWith('--config=')) overrides.push(arg.slice('--config='.length));
  }
  for (const override of overrides.reverse()) {
    const loaded = await readOverrideFile(cwd, override);
    const target = loaded === null
      ? named(override, '--config', { requireEntry: true })
      : named(loaded.text, loaded.file);
    if (target !== undefined) return target;
  }
  if (env.CARGO_BUILD_TARGET) return env.CARGO_BUILD_TARGET;
  for (let at = path.resolve(cwd); ; at = path.dirname(at)) {
    const target = await folderTarget(path.join(at, '.cargo'));
    if (target !== undefined) return target;
    if (path.dirname(at) === at) break;
  }
  return home ? folderTarget(home) : undefined;
}
