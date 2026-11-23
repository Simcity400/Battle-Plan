import { parseAnswer, textAt } from './message.ts';
import type { FollowedAnswer, FollowedRows } from './message';

const NO_TEXT = 0xffff_ffff;
const encoder = new TextEncoder();
const aligned = (at: number) => (at + 7) & ~7;

const keyAt = (rows: FollowedRows, row: number, word: number) => rows.views.u32[rows.keys * 2 + (word < 2 ? row * 2 + word : (rows.rows + row) * 2 + word - 2)]!;

function compareKeys(a: FollowedRows, i: number, b: FollowedRows, j: number) {
  for (let word = 3; word >= 0; word--) {
    const x = keyAt(a, i, word), y = keyAt(b, j, word);
    if (x !== y) return x < y ? -1 : 1;
  }
  return 0;
}

interface Texts { base: Uint8Array; ends: Uint32Array; added: Uint8Array[]; index: Map<string, number>; count: number }

function textsOf(base: FollowedRows, compact: boolean): { texts: Texts; reasons: Uint32Array; names: Uint32Array } {
  const rows = base.rows;
  const reasons = new Uint32Array(rows), names = new Uint32Array(rows);
  if (!compact) {
    reasons.set(base.views.u32.subarray(base.reasons, base.reasons + rows));
    names.set(base.views.u32.subarray(base.names, base.names + rows));
    const count = base.ends.length - 1;
    return { texts: { base: base.views.u8.subarray(base.text, base.text + base.ends[count]!), ends: base.ends, added: [], index: new Map(), count }, reasons, names };
  }
  const texts: Texts = { base: new Uint8Array(), ends: new Uint32Array([0]), added: [], index: new Map(), count: 0 };
  for (let row = 0; row < rows; row++) {
    const reason = base.views.u32[base.reasons + row]!, name = base.views.u32[base.names + row]!;
    reasons[row] = reason === NO_TEXT ? NO_TEXT : add(texts, textAt(base, reason)!);
    names[row] = name === NO_TEXT ? NO_TEXT : add(texts, textAt(base, name)!);
  }
  return { texts, reasons, names };
}

function add(texts: Texts, text: string): number {
  let at = texts.index.get(text);
  if (at === undefined) { at = texts.count + texts.added.push(encoder.encode(text)) - 1; texts.index.set(text, at); }
  return at;
}

export function mergeChanged(base: FollowedAnswer, changed: FollowedRows, held = Infinity): Uint8Array {
  if (base.kind !== 'rows') throw new Error('Changed rows arrived for an answer that is not a whole table.');
  if (changed.columns !== base.columns) throw new Error(`Changed rows of ${changed.columns} columns for an answer of ${base.columns}`);
  if (!changed.canonical || !base.canonical) throw new Error('Changed rows arrived out of key order: they are merged only by walking keys.');
  const merged = merge(base, changed, base.ends.length - 1 > base.rows * 2 + 16);
  return merged.length > held ? merge(parseAnswer(merged) as FollowedRows, { ...changed, rows: 0 }, true) : merged;
}

function merge(base: FollowedRows, changed: FollowedRows, compact: boolean): Uint8Array {
  const { rows, columns } = base, cells = rows * columns;
  const { texts, reasons, names } = textsOf(base, compact);
  const values = new BigInt64Array(base.views.i64.buffer, base.views.i64.byteOffset + base.values * 8, cells).slice();
  const states = base.views.u8.slice(base.states, base.states + cells);
  let at = 0;
  for (let row = 0; row < changed.rows; row++) {
    while (at < rows && compareKeys(base, at, changed, row) < 0) at++;
    if (at === rows || compareKeys(base, at, changed, row) !== 0) throw new Error('A changed row names a key the table does not hold.');
    for (let column = 0; column < columns; column++) {
      values[at * columns + column] = changed.views.i64[changed.values + row * columns + column]!;
      states[at * columns + column] = changed.views.u8[changed.states + row * columns + column]!;
    }
    const reason = changed.views.u32[changed.reasons + row]!;
    reasons[at] = reason === NO_TEXT ? NO_TEXT : add(texts, textAt(changed, reason)!);
  }
  const count = texts.count + texts.added.length;
  const added = texts.added.reduce((sum, text) => sum + text.length, 0), baseBytes = texts.base.length;
  const statesAt = 16 + rows * 16 + cells * 8, reasonsAt = aligned(statesAt + cells), namesAt = aligned(reasonsAt + rows * 4), endsAt = aligned(namesAt + rows * 4);
  const textsAt = aligned(endsAt + (count + 1) * 4), size = aligned(textsAt + baseBytes + added);
  const out = new Uint8Array(size), data = new DataView(out.buffer);
  data.setUint8(0, 1); data.setUint8(1, 1);
  data.setUint32(4, rows, true); data.setUint32(8, columns, true); data.setUint32(12, count, true);
  out.set(base.views.u8.subarray(base.keys * 8, base.keys * 8 + rows * 16), 16);
  new BigInt64Array(out.buffer, 16 + rows * 16, cells).set(values);
  out.set(states, statesAt);
  new Uint32Array(out.buffer, reasonsAt, rows).set(reasons);
  new Uint32Array(out.buffer, namesAt, rows).set(names);
  const ends = new Uint32Array(out.buffer, endsAt, count + 1);
  ends.set(texts.ends.subarray(0, texts.count + 1));
  out.set(texts.base, textsAt);
  let end = baseBytes, put = textsAt + baseBytes;
  texts.added.forEach((text, index) => { out.set(text, put); put += text.length; end += text.length; ends[texts.count + index + 1] = end; });
  return out;
}
