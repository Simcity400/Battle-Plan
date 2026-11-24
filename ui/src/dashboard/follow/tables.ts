import { rowName, rowReason, textAt } from './message.ts';
import type { FollowedAnswer, FollowedRows, PatchNode, RowChunk, RowPatches } from './message';

const NO_TEXT = 0xffff_ffff;
const CHUNK_ROWS = 8;
const BRANCH = 8;
const encoder = new TextEncoder();
const aligned = (at: number) => (at + 7) & ~7;

export interface PatchStats { cellsCopied: number; indexSlotsAllocated?: number }

function treeShift(rows: number): number {
  const chunks = Math.ceil(rows / CHUNK_ROWS);
  let shift = 0, cap = BRANCH;
  while (cap < chunks) { cap *= BRANCH; shift += 3; }
  return shift;
}

function getChunk(patches: RowPatches | undefined, chunkIndex: number): RowChunk | undefined {
  if (!patches) return undefined;
  let node: PatchNode | undefined = patches.root;
  for (let shift = patches.shift; shift > 0; shift -= 3) {
    node = node[(chunkIndex >>> shift) & 7] as PatchNode | undefined;
    if (!node) return undefined;
  }
  return node[chunkIndex & 7] as RowChunk | undefined;
}

const keyAt = (rows: FollowedRows, row: number, word: number) => rows.views.u32[rows.keys * 2 + (word < 2 ? row * 2 + word : (rows.rows + row) * 2 + word - 2)]!;

function compareKeys(a: FollowedRows, i: number, b: FollowedRows, j: number) {
  for (let word = 3; word >= 0; word--) {
    const x = keyAt(a, i, word), y = keyAt(b, j, word);
    if (x !== y) return x < y ? -1 : 1;
  }
  return 0;
}

function findRow(base: FollowedRows, from: number, changed: FollowedRows, row: number): number {
  const rows = base.rows;
  if (from >= rows) throw new Error('A changed row names a key the table does not hold.');
  const first = compareKeys(base, from, changed, row);
  if (first === 0) return from;
  if (first > 0) throw new Error('A changed row names a key the table does not hold.');
  let lo = from, step = 1, hi = from + 1;
  while (hi < rows && compareKeys(base, hi, changed, row) < 0) {
    lo = hi;
    step <<= 1;
    hi = Math.min(rows, lo + step);
  }
  hi = Math.min(hi, rows - 1);
  while (lo <= hi) {
    const mid = (lo + hi) >>> 1;
    const cmp = compareKeys(base, mid, changed, row);
    if (cmp === 0) return mid;
    if (cmp < 0) lo = mid + 1;
    else hi = mid - 1;
  }
  throw new Error('A changed row names a key the table does not hold.');
}

function makeChunk(base: FollowedRows, chunkIndex: number, stats?: PatchStats): RowChunk {
  const start = chunkIndex * CHUNK_ROWS;
  const len = Math.min(CHUNK_ROWS, base.rows - start);
  const columns = base.columns;
  const existing = getChunk(base.patches, chunkIndex);
  if (stats) stats.cellsCopied += len * columns;
  if (existing) {
    return { values: existing.values.slice(), states: existing.states.slice(), reasons: existing.reasons.slice() };
  }
  const cells = len * columns;
  const values = new BigInt64Array(base.views.i64.buffer, base.views.i64.byteOffset + (base.values + start * columns) * 8, cells).slice();
  const states = base.views.u8.slice(base.states + start * columns, base.states + start * columns + cells);
  const reasons = new Array<string | null>(len);
  for (let i = 0; i < len; i++) {
    const at = base.views.u32[base.reasons + start + i]!;
    reasons[i] = at === NO_TEXT ? null : textAt(base, at);
  }
  return { values, states, reasons };
}

export function patchAnswer(base: FollowedAnswer, changed: FollowedRows, stats?: PatchStats): FollowedRows {
  if (base.kind !== 'rows') throw new Error('Changed rows arrived for an answer that is not a whole table.');
  if (changed.columns !== base.columns) throw new Error(`Changed rows of ${changed.columns} columns for an answer of ${base.columns}`);
  if (!changed.canonical || !base.canonical) throw new Error('Changed rows arrived out of key order: they are merged only by walking keys.');
  if (!changed.rows) return base;
  const { rows, columns } = base;
  const shift = base.patches ? base.patches.shift : treeShift(rows);
  const nextRoot: PatchNode = base.patches ? base.patches.root.slice() : new Array(BRANCH);
  if (stats) stats.indexSlotsAllocated = (stats.indexSlotsAllocated ?? 0) + BRANCH;
  const cloned = new Set<PatchNode>([nextRoot]);
  const clonedChunks = new Map<number, RowChunk>();
  let at = 0;
  for (let row = 0; row < changed.rows; row++) {
    at = findRow(base, at, changed, row);
    const chunkIndex = at >>> 3, offset = at & 7;
    let chunk = clonedChunks.get(chunkIndex);
    if (!chunk) {
      let node = nextRoot;
      for (let s = shift; s > 0; s -= 3) {
        const idx = (chunkIndex >>> s) & 7;
        let child = node[idx] as PatchNode | undefined;
        if (!child || !cloned.has(child)) {
          child = child ? child.slice() : new Array(BRANCH);
          if (stats) stats.indexSlotsAllocated = (stats.indexSlotsAllocated ?? 0) + BRANCH;
          cloned.add(child);
          node[idx] = child;
        }
        node = child;
      }
      chunk = makeChunk(base, chunkIndex, stats);
      node[chunkIndex & 7] = chunk;
      clonedChunks.set(chunkIndex, chunk);
    }
    const dst = offset * columns, src = row * columns;
    for (let column = 0; column < columns; column++) {
      chunk.values[dst + column] = changed.views.i64[changed.values + src + column]!;
      chunk.states[dst + column] = changed.views.u8[changed.states + src + column]!;
    }
    const reason = changed.views.u32[changed.reasons + row]!;
    chunk.reasons[offset] = reason === NO_TEXT ? null : textAt(changed, reason)!;
    at++;
  }
  return { ...base, patches: { root: nextRoot, shift } };
}

function visitChunks(node: PatchNode, shift: number, baseChunk: number, visit: (chunkIndex: number, chunk: RowChunk) => void) {
  for (let i = 0; i < BRANCH; i++) {
    const child = node[i];
    if (!child) continue;
    const at = baseChunk + (i << shift);
    if (shift === 0) visit(at, child as RowChunk);
    else visitChunks(child as PatchNode, shift - 3, at, visit);
  }
}

export function serializeAnswer(answer: FollowedRows): Uint8Array {
  const { rows, columns } = answer, cells = rows * columns;
  const statesAt = 16 + rows * 16 + cells * 8, reasonsAt = aligned(statesAt + cells);
  let reasonChanged = answer.kind !== 'rows';
  if (!reasonChanged && answer.patches) {
    visitChunks(answer.patches.root, answer.patches.shift, 0, (chunkIndex, chunk) => {
      if (reasonChanged) return;
      const start = chunkIndex * CHUNK_ROWS;
      for (let i = 0; i < chunk.reasons.length; i++) {
        if (chunk.reasons[i] !== null || answer.views.u32[answer.reasons + start + i] !== NO_TEXT) {
          reasonChanged = true;
          return;
        }
      }
    });
  }
  const writeCells = (out: Uint8Array) => {
    out.set(answer.views.u8.subarray(answer.keys * 8, answer.keys * 8 + rows * 16), 16);
    const valuesOut = new BigInt64Array(out.buffer, 16 + rows * 16, cells);
    valuesOut.set(answer.views.i64.subarray(answer.values, answer.values + cells));
    out.set(answer.views.u8.subarray(answer.states, answer.states + cells), statesAt);
    if (answer.patches) {
      visitChunks(answer.patches.root, answer.patches.shift, 0, (chunkIndex, chunk) => {
        const cellOffset = chunkIndex * CHUNK_ROWS * columns;
        valuesOut.set(chunk.values, cellOffset);
        out.set(chunk.states, statesAt + cellOffset);
      });
    }
  };
  if (!reasonChanged) {
    const count = answer.ends.length - 1;
    const tailStart = answer.reasons * 4;
    const tailEnd = aligned(answer.text + answer.ends[count]!);
    const out = new Uint8Array(reasonsAt + (tailEnd - tailStart)), data = new DataView(out.buffer);
    data.setUint8(0, 1); data.setUint8(1, answer.canonical ? 1 : 0);
    data.setUint32(4, rows, true); data.setUint32(8, columns, true); data.setUint32(12, count, true);
    writeCells(out);
    out.set(answer.views.u8.subarray(tailStart, tailEnd), reasonsAt);
    return out;
  }
  const added: Uint8Array[] = [], index = new Map<string, number>();
  const add = (text: string) => {
    let at = index.get(text);
    if (at === undefined) { at = added.push(encoder.encode(text)) - 1; index.set(text, at); }
    return at;
  };
  const reasons = new Uint32Array(rows), names = new Uint32Array(rows);
  for (let row = 0; row < rows; row++) {
    const reason = rowReason(answer, row), name = rowName(answer, row);
    reasons[row] = reason === null ? NO_TEXT : add(reason);
    names[row] = name === null ? NO_TEXT : add(name);
  }
  const count = added.length, textBytes = added.reduce((sum, text) => sum + text.length, 0);
  const namesAt = aligned(reasonsAt + rows * 4), endsAt = aligned(namesAt + rows * 4);
  const textsAt = aligned(endsAt + (count + 1) * 4), size = aligned(textsAt + textBytes);
  const out = new Uint8Array(size), data = new DataView(out.buffer);
  data.setUint8(0, 1); data.setUint8(1, answer.canonical ? 1 : 0);
  data.setUint32(4, rows, true); data.setUint32(8, columns, true); data.setUint32(12, count, true);
  writeCells(out);
  new Uint32Array(out.buffer, reasonsAt, rows).set(reasons);
  new Uint32Array(out.buffer, namesAt, rows).set(names);
  const ends = new Uint32Array(out.buffer, endsAt, count + 1);
  let end = 0, put = textsAt;
  added.forEach((text, at) => { out.set(text, put); put += text.length; end += text.length; ends[at + 1] = end; });
  return out;
}

function diffPatchNodes(
  a: PatchNode | undefined,
  b: PatchNode | undefined,
  shift: number,
  baseChunk: number,
  rows: number,
  out: number[],
): void {
  if (a === b) return;
  for (let i = 0; i < BRANCH; i++) {
    const ca = a?.[i];
    const cb = b?.[i];
    if (ca === cb) continue;
    const at = baseChunk + (i << shift);
    if (shift === 0) {
      const start = at * CHUNK_ROWS;
      const end = Math.min(rows, start + CHUNK_ROWS);
      for (let r = start; r < end; r++) out.push(r);
    } else {
      diffPatchNodes(ca as PatchNode | undefined, cb as PatchNode | undefined, shift - 3, at, rows, out);
    }
  }
}

export function changedAnswerRows(prev: FollowedRows, next: FollowedRows): number[] | null {
  if (prev === next) return [];
  if (
    prev.views.u8 !== next.views.u8
    || prev.values !== next.values
    || prev.states !== next.states
    || prev.rows !== next.rows
    || prev.columns !== next.columns
  ) return null;
  const pa = prev.patches;
  const pb = next.patches;
  if (pa === pb) return [];
  if (pa && pb && pa.shift !== pb.shift) return null;
  const shift = pb ? pb.shift : pa!.shift;
  const out: number[] = [];
  diffPatchNodes(pa?.root, pb?.root, shift, 0, next.rows, out);
  return out;
}

export function mergeChanged(base: FollowedAnswer, changed: FollowedRows, _held = Infinity): Uint8Array {
  return serializeAnswer(patchAnswer(base, changed));
}
