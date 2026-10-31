import { runAt } from '../charts/series.ts';
import { int, object } from './validation.ts';
import type { NativeFixed5Run } from '../generated/dashboard';
export type { NativeFixed5Run } from '../generated/dashboard';

const SCALE = 100000n;
const MIN = -(1n << 63n);
const MAX = (1n << 63n) - 1n;
const integer = /^(?:0|-?[1-9]\d*)$/;
/** A run's values at each change, and the fields they were decoded from. */
interface Changes {
  hours: Float64Array; values: BigInt64Array;
  from: Pick<NativeFixed5Run, 'firstHour' | 'lastHour' | 'raw' | 'deltas' | 'deltaScale'>;
}
const decoded = new WeakMap<NativeFixed5Run, Changes>();

function signed(value: unknown): value is string {
  if (typeof value !== 'string' || !integer.test(value)) return false;
  const raw = BigInt(value);
  return raw >= MIN && raw <= MAX;
}

/** A canonical signed decimal at `text[from..to)` (no sign before 0, no leading zeros), or null. */
function canonical(text: string, from: number, to: number): bigint | null {
  const negative = text.charCodeAt(from) === 45;
  const digits = negative ? from + 1 : from;
  if (digits >= to) return null;
  const lead = text.charCodeAt(digits);
  if (lead === 48 ? to - digits !== 1 || negative : lead < 49 || lead > 57) return null;
  let small = 0;
  for (let at = digits; at < to; at++) {
    const code = text.charCodeAt(at);
    if (code < 48 || code > 57) return null;
    small = small * 10 + (code - 48);
  }
  // Fifteen digits are exact as a number; longer tokens are read as text.
  const value = to - digits <= 15 ? BigInt(negative ? -small : small) : BigInt(text.slice(from, to));
  return value >= MIN && value <= MAX ? value : null;
}

/** A run's values at each change, decoded once and kept for the run, or null when its pairs are
 * malformed. The writer emits one positive elapsed-hour/nonzero-delta pair per change. */
function decode(run: NativeFixed5Run): Changes | null {
  const cached = decoded.get(run)?.from;
  // A run changed since it was decoded is decoded, and so validated, again.
  if (cached && cached.firstHour === run.firstHour && cached.lastHour === run.lastHour && cached.raw === run.raw
    && cached.deltas === run.deltas && cached.deltaScale === run.deltaScale) return decoded.get(run)!;
  let raw = BigInt(run.raw!);
  const hours = [run.firstHour];
  const values = [raw];
  if (run.deltas !== undefined) {
    const text = run.deltas;
    if (typeof text !== 'string' || !text.length) return null;
    const scale = run.deltaScale === undefined ? SCALE : BigInt(run.deltaScale);
    let hour = run.firstHour;
    let at = 0;
    while (true) {
      // The elapsed hours: a positive decimal without leading zeros.
      const gapStart = at;
      while (at < text.length && text.charCodeAt(at) !== 58) at++;
      if (at === gapStart || at === text.length || text.charCodeAt(gapStart) === 48) return null;
      let gap = 0;
      for (let digit = gapStart; digit < at; digit++) {
        const code = text.charCodeAt(digit);
        if (code < 48 || code > 57) return null;
        gap = gap * 10 + (code - 48);
      }
      if (!Number.isSafeInteger(gap)) return null;
      hour += gap;
      if (!Number.isSafeInteger(hour) || hour > run.lastHour) return null;
      // The change: a nonzero signed token, raw when marked `r`.
      at++;
      const rawToken = text.charCodeAt(at) === 114;
      const tokenStart = rawToken ? at + 1 : at;
      at = tokenStart;
      while (at < text.length && text.charCodeAt(at) !== 32) at++;
      const token = canonical(text, tokenStart, at);
      if (token === null || token === 0n) return null;
      // The first change establishes the scale; only MIN cannot use its magnitude.
      if (values.length === 1 && (rawToken || (token !== 1n && token !== -1n && !(scale === 1n && token === MIN)))) return null;
      const delta = rawToken ? token : token * scale;
      // Quotient tokens must encode an exact signed raw delta; divisible raw
      // tokens are not produced by the writer and have no second wire meaning.
      if (delta < MIN || delta > MAX || (rawToken && delta % scale === 0n)) return null;
      raw = BigInt.asIntN(64, raw + delta);
      hours.push(hour);
      values.push(raw);
      if (at === text.length) break;
      // One space between pairs, none trailing.
      at++;
      if (at === text.length) return null;
    }
  }
  const from = { firstHour: run.firstHour, lastHour: run.lastHour, raw: run.raw, deltas: run.deltas, deltaScale: run.deltaScale };
  const result = { hours: Float64Array.from(hours), values: BigInt64Array.from(values), from };
  decoded.set(run, result);
  return result;
}

/** Refuse malformed sparse pairs and overlaps without repairing the reply. */
export function validateFixed5Runs(value: unknown, reasonsLength = Number.MAX_SAFE_INTEGER): value is NativeFixed5Run[] {
  if (!Array.isArray(value)) return false;
  let last: number | null = null;
  for (const run of value) {
    if (!object(run) || !int(run.firstHour) || !int(run.lastHour) || run.firstHour > run.lastHour
      || (last !== null && run.firstHour <= last) || typeof run.readEachHour !== 'boolean') return false;
    const known = run.raw !== undefined;
    if (known ? !signed(run.raw) || run.missing !== undefined
      : !int(run.missing) || run.missing < 0 || run.missing >= reasonsLength || run.deltas !== undefined || run.deltaScale !== undefined) return false;
    if (run.deltaScale !== undefined && (!signed(run.deltaScale) || BigInt(run.deltaScale) <= 0n
      || BigInt(run.deltaScale) === SCALE || run.deltas === undefined)) return false;
    // Decoded here, the changes are kept for every later read of the run.
    if (known && decode(run as unknown as NativeFixed5Run) === null) return false;
    last = run.lastHour;
  }
  return true;
}

function changes(run: NativeFixed5Run): Changes {
  const parsed = decode(run);
  if (!parsed) throw new Error('Malformed native fixed5 changes.');
  return parsed;
}

export interface Fixed5At { raw: string | null; missing: number | null; readEachHour: boolean }

/** Equal rereads hold exactly between change pairs; recording gaps remain gaps.
 * The caller dates the supplying read through reads.ts, never the last change. */
export function fixed5At(runs: readonly NativeFixed5Run[], hour: number): Fixed5At | null {
  const run = runAt(runs, hour);
  if (!run) return null;
  if (run.raw === undefined) return { raw: null, missing: run.missing ?? null, readEachHour: run.readEachHour };
  const at = changes(run);
  let low = 0;
  let high = at.hours.length - 1;
  while (low < high) {
    const middle = Math.floor((low + high + 1) / 2);
    if (at.hours[middle] <= hour) low = middle; else high = middle - 1;
  }
  return { raw: at.values[low].toString(), missing: null, readEachHour: run.readEachHour };
}
