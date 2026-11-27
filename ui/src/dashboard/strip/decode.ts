import { decode } from '@msgpack/msgpack';
import type { StripWork } from './scrub.ts';

type Value = null | boolean | number | bigint | string | Value[] | { [key: string]: Value };
export type StripState = { answer: Value } | { refused: string };
type Path = (number | string)[];
type Edit = { replace: [Path, Value] } | { remove: Path } | { splice: [Path, number, number, Value[]] };

function invalid(): never { throw new Error('Invalid dashboard strip'); }

export function readStrip(bytes: Uint8Array) {
  const [b, p, s, t, version, flags, count, reserved] = bytes;
  if (bytes.length < 8 || b !== 66 || p !== 80 || s !== 83 || t !== 84 || version !== 1 ||
      (flags !== 0 && flags !== 1) || !count || count > 17 || reserved !== 0) invalid();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let at = 8;
  const part = () => {
    if (at + 4 > bytes.length) invalid();
    const length = view.getUint32(at, true);
    at += 4;
    if (length === 0 || at + length > bytes.length) invalid();
    const value = bytes.subarray(at, at + length);
    at += length;
    return value;
  };
  const whole = flags ? part() : null;
  const patches = Array.from({ length: count - 1 }, () => [part(), part()] as const);
  if (at !== bytes.length) invalid();
  return { whole, patches };
}

function numbers(value: Value): Value {
  if (typeof value === 'bigint') return Number.isSafeInteger(Number(value)) ? Number(value) : value;
  if (value === null || typeof value !== 'object') return value;
  if (Array.isArray(value)) return value.map(numbers);
  for (const key of Object.keys(value)) value[key] = numbers(value[key]!);
  return value;
}

function unpack(bytes: Uint8Array): Value {
  return numbers(decode(bytes, { useBigInt64: true }) as Value);
}

export function decodeState(bytes: Uint8Array, work?: StripWork): StripState {
  if (work) work.bytes += bytes.length;
  return unpack(bytes) as StripState;
}

function member(value: Value, key: number | string, inserting = false): Record<string | number, Value> {
  if (typeof key === 'number') {
    if (!Array.isArray(value) || !Number.isSafeInteger(key) || key < 0 || key >= value.length) invalid();
  } else if (typeof key !== 'string' || !value || typeof value !== 'object' || Array.isArray(value) || (!inserting && !Object.hasOwn(value, key))) invalid();
  return value as Record<string | number, Value>;
}

function target(state: Value, path: Path): Value {
  for (const key of path) state = member(state, key)[key]!;
  return state;
}

export function applyStripPatch(state: StripState, bytes: Uint8Array, work?: StripWork): StripState {
  const edits = unpack(bytes) as Edit[];
  if (work) { work.bytes += bytes.length; work.leaves += edits.length; }
  for (const edit of edits) {
    if ('replace' in edit) {
      const [path, value] = edit.replace;
      if (!path.length) state = value as StripState;
      else {
        const key = path.at(-1)!;
        Object.defineProperty(member(target(state, path.slice(0, -1)), key, true), key,
          { value, enumerable: true, configurable: true, writable: true });
      }
    } else if ('remove' in edit) {
      const key = edit.remove.at(-1);
      if (typeof key !== 'string') invalid();
      delete member(target(state, edit.remove.slice(0, -1)), key)[key];
    } else if ('splice' in edit) {
      const [path, at, count, values] = edit.splice;
      const list = target(state, path);
      if (!Array.isArray(list) || !Array.isArray(values) || !Number.isSafeInteger(at) || !Number.isSafeInteger(count) || at < 0 || count < 0 || at + count > list.length) invalid();
      list.splice(at, count);
      for (let start = 0; start < values.length; start += 8192) list.splice(at + start, 0, ...values.slice(start, start + 8192));
    } else invalid();
  }
  return state;
}
