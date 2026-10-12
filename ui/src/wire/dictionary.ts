import { UNKNOWN_TOTAL, WireError } from './runtime.ts';
import type { DictionaryReference } from './runtime.ts';

const IV = new Uint32Array([0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19]);
const PERMUTATION = [2,6,3,10,7,0,4,13,1,11,12,5,9,14,15,8];
const CHUNK_START = 1, CHUNK_END = 2, PARENT = 4, ROOT = 8;
interface HashOutput { cv: Uint32Array; words: Uint32Array; counter: number; length: number; flags: number }
const rotate = (n: number, bits: number) => (n >>> bits) | (n << (32 - bits));
function compress(output: HashOutput, root = false): Uint32Array {
  const state = new Uint32Array(16); state.set(output.cv);
  state.set(IV.subarray(0, 4), 8);
  const counter = root ? 0 : output.counter;
  state[12] = counter >>> 0; state[13] = Math.floor(counter / 0x100000000);
  state[14] = output.length; state[15] = output.flags | (root ? ROOT : 0);
  let message = output.words;
  function mix(a: number, b: number, c: number, d: number, x: number, y: number): void {
    state[a] = state[a] + state[b] + x; state[d] = rotate(state[d] ^ state[a], 16);
    state[c] = state[c] + state[d]; state[b] = rotate(state[b] ^ state[c], 12);
    state[a] = state[a] + state[b] + y; state[d] = rotate(state[d] ^ state[a], 8);
    state[c] = state[c] + state[d]; state[b] = rotate(state[b] ^ state[c], 7);
  }
  for (let round = 0; round < 7; ++round) {
    mix(0,4,8,12,message[0],message[1]); mix(1,5,9,13,message[2],message[3]);
    mix(2,6,10,14,message[4],message[5]); mix(3,7,11,15,message[6],message[7]);
    mix(0,5,10,15,message[8],message[9]); mix(1,6,11,12,message[10],message[11]);
    mix(2,7,8,13,message[12],message[13]); mix(3,4,9,14,message[14],message[15]);
    if (round !== 6) message = Uint32Array.from(PERMUTATION, i => message[i]);
  }
  for (let i = 0; i < 8; ++i) { state[i] ^= state[i + 8]; state[i + 8] ^= output.cv[i]; }
  return state;
}
function parentOutput(left: Uint32Array, right: Uint32Array): HashOutput {
  const words = new Uint32Array(16); words.set(left); words.set(right, 8);
  return {cv: IV, words, counter: 0, length: 64, flags: PARENT};
}
/** Standard unkeyed BLAKE3-256 for content-addressed dictionary shards. */
export function blake3(bytes: Uint8Array): Uint8Array {
  const stack: Uint32Array[] = [];
  const chunks = Math.max(1, Math.ceil(bytes.length / 1024));
  let output: HashOutput | undefined;
  for (let chunk = 0; chunk < chunks; ++chunk) {
    let cv = IV;
    const chunkStart = chunk * 1024, chunkEnd = Math.min(bytes.length, chunkStart + 1024);
    const blocks = Math.max(1, Math.ceil((chunkEnd - chunkStart) / 64));
    for (let block = 0; block < blocks; ++block) {
      const start = chunkStart + block * 64, length = Math.min(64, chunkEnd - start);
      const words = new Uint32Array(16);
      for (let i = 0; i < length; ++i) words[i >>> 2] |= bytes[start + i] << ((i & 3) * 8);
      output = {cv, words, counter: chunk, length,
        flags: (block === 0 ? CHUNK_START : 0) | (block === blocks - 1 ? CHUNK_END : 0)};
      if (block !== blocks - 1) cv = compress(output).slice(0, 8);
    }
    if (chunk !== chunks - 1) {
      let chaining = compress(output!).slice(0, 8), total = chunk + 1;
      while ((total & 1) === 0) {
        chaining = compress(parentOutput(stack.pop()!, chaining)).slice(0, 8); total /= 2;
      }
      stack.push(chaining);
    }
  }
  while (stack.length > 0) output = parentOutput(stack.pop()!, compress(output!).slice(0, 8));
  const words = compress(output!, true), result = new Uint8Array(32), view = new DataView(result.buffer);
  for (let i = 0; i < 8; ++i) view.setUint32(i * 4, words[i], true);
  return result;
}
function valid(condition: unknown, code: string): asserts condition {
  if (!condition) throw new WireError(code);
}
function coordinate(value: bigint, maximum: number, code = 'dictionary-page'): number {
  valid(value <= BigInt(maximum), code); return Number(value);
}
export interface DictionaryPage {
  readonly firstEntry: bigint; readonly entryCount: bigint;
  readonly offsets: BigUint64Array; readonly bytes: Uint8Array;
  entry(canonicalIndex: bigint): Uint8Array;
}
export interface DictionaryShard {
  readonly kind: number; readonly firstEntry: bigint; readonly entryCount: bigint;
  readonly identity: Uint8Array; readonly pages: readonly DictionaryPage[];
  entry(localCode: bigint): Uint8Array;
}
/** Page v1: entry_count:u64, offsets:u64[count+1], exact entry bytes. */
export function decodeDictionaryPage(buffer: ArrayBuffer, firstEntry: bigint,
  offset = 0, length = buffer.byteLength - offset, kind = 0): DictionaryPage {
  valid(Number.isInteger(kind) && kind >= 0 && kind <= 5, 'dictionary-kind');
  valid(offset % 8 === 0 && length >= 16 && offset >= 0 && length >= 0 && offset + length <= buffer.byteLength, 'dictionary-page');
  const view = new DataView(buffer, offset, length), entryCount = view.getBigUint64(0, true);
  valid(entryCount > 0n, 'dictionary-page');
  valid((entryCount + 1n) * 8n <= BigInt(length - 8), 'dictionary-page');
  const count = coordinate(entryCount, Math.floor((length - 16) / 8));
  const offsets = new BigUint64Array(buffer, offset + 8, count + 1);
  const dataOffset = 8 + (count + 1) * 8;
  const bytes = new Uint8Array(buffer, offset + dataOffset, length - dataOffset);
  valid(offsets[0] === 0n && offsets[count] === BigInt(bytes.length), 'dictionary-page');
  for (let i = 1; i <= count; ++i) valid(offsets[i] >= offsets[i - 1], 'dictionary-page');
  if (entryCount > 0n) valid(firstEntry + entryCount - 1n <= UNKNOWN_TOTAL, 'dictionary-overflow');
  const page: DictionaryPage = {firstEntry, entryCount, offsets, bytes, entry(canonicalIndex) {
    valid(canonicalIndex >= firstEntry && canonicalIndex - firstEntry < entryCount, 'dictionary-code');
    const code = coordinate(canonicalIndex - firstEntry, count - 1, 'dictionary-code');
    return bytes.subarray(coordinate(offsets[code], bytes.length), coordinate(offsets[code + 1], bytes.length));
  }};
  if (kind !== 0) {
    const decoder = new TextDecoder('utf-8', {fatal: true});
    for (let i = 0; i < count; ++i) {
      try { decoder.decode(page.entry(firstEntry + BigInt(i))); } catch { throw new WireError('invalid-utf8'); }
    }
  }
  return page;
}
/** Checks the content address and all pages before returning zero-copy entry views. */
export function decodeDictionaryShard(buffer: ArrayBuffer, reference: DictionaryReference): DictionaryShard {
  valid(buffer.byteLength >= 32, 'invalid-dictionary');
  const view = new DataView(buffer), bytes = new Uint8Array(buffer);
  valid(bytes[0] === 72 && bytes[1] === 52 && bytes[2] === 82 && bytes[3] === 68, 'invalid-dictionary');
  valid(view.getUint16(4, true) === 1, 'dictionary-version');
  const identity = blake3(bytes);
  valid(reference.shardIdentity.length === 32 && identity.every((byte, i) => byte === reference.shardIdentity[i]), 'dictionary-identity');
  const kind = view.getUint16(6, true), firstEntry = view.getBigUint64(8, true),
    entryCount = view.getBigUint64(16, true), pageCount = view.getBigUint64(24, true);
  valid(kind <= 5, 'dictionary-kind');
  valid(kind === reference.kind && firstEntry === reference.firstEntry && entryCount === reference.entryCount &&
    reference.flags === 0, 'dictionary-reference');
  if (entryCount > 0n) valid(firstEntry + entryCount - 1n <= UNKNOWN_TOTAL, 'dictionary-overflow');
  valid(pageCount <= BigInt(Math.floor((buffer.byteLength - 32) / 24)), 'dictionary-page');
  const pages: DictionaryPage[] = [];
  let nextEntry = firstEntry, priorEnd = 32 + Number(pageCount) * 24;
  for (let i = 0; i < Number(pageCount); ++i) {
    const p = 32 + i * 24, first = view.getBigUint64(p, true), offset64 = view.getBigUint64(p + 8, true),
      length64 = view.getBigUint64(p + 16, true);
    valid(offset64 % 8n === 0n && offset64 >= BigInt(priorEnd) && offset64 <= BigInt(buffer.byteLength) &&
      length64 <= BigInt(buffer.byteLength) - offset64 && first === nextEntry, 'dictionary-page');
    const offset = coordinate(offset64, buffer.byteLength), length = coordinate(length64, buffer.byteLength);
    for (let pad = priorEnd; pad < offset; ++pad) valid(bytes[pad] === 0, 'nonzero-padding');
    const page = decodeDictionaryPage(buffer, first, offset, length, kind); pages.push(page);
    nextEntry += page.entryCount; priorEnd = offset + length;
  }
  valid(nextEntry === firstEntry + entryCount, 'dictionary-page');
  for (let pad = priorEnd; pad < bytes.length; ++pad) valid(bytes[pad] === 0, 'dictionary-page');
  return {kind, firstEntry, entryCount, identity, pages, entry(localCode) {
    valid(localCode >= 0n && localCode < entryCount && firstEntry + localCode <= UNKNOWN_TOTAL, 'dictionary-code');
    const canonical = firstEntry + localCode;
    let low = 0, high = pages.length;
    while (low < high) { const middle = Math.floor((low + high) / 2);
      if (pages[middle].firstEntry + pages[middle].entryCount <= canonical) low = middle + 1; else high = middle; }
    valid(low < pages.length, 'dictionary-code'); return pages[low].entry(canonical);
  }};
}
export interface DictionaryByteExtent { first: bigint; end: bigint; total: bigint; final: boolean; bytes: Uint8Array }
/** Validates a fetched entry byte extent without treating a prefix as a complete entry. */
export function validateDictionaryExtent(extent: DictionaryByteExtent): DictionaryByteExtent {
  valid(extent.first >= 0n && extent.first <= extent.end && extent.end <= extent.total && extent.total <= UNKNOWN_TOTAL &&
    extent.end - extent.first === BigInt(extent.bytes.length) &&
    (extent.total === UNKNOWN_TOTAL || extent.final === (extent.end === extent.total)), 'dictionary-extents');
  return extent;
}
