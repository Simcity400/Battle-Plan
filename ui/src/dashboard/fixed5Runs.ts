import { runAt } from '../charts/series.ts';
import { int, object } from './validation.ts';
import type { NativeFixed5Run } from '../generated/dashboard';
export type { NativeFixed5Run } from '../generated/dashboard';

const SCALE = 100000n;
const MIN = -(1n << 63n);
const MAX = (1n << 63n) - 1n;
const integer = /^(?:0|-?[1-9]\d*)$/;
const gapInteger = /^[1-9]\d*$/;
interface Changes { hours: Float64Array; values: BigInt64Array }
const decoded = new WeakMap<NativeFixed5Run, Changes>();

function signed(value: unknown): value is string {
  if (typeof value !== 'string' || !integer.test(value)) return false;
  const raw = BigInt(value);
  return raw >= MIN && raw <= MAX;
}

/** The writer emits one positive elapsed-hour/nonzero-delta pair per change. */
function pairs(run: NativeFixed5Run): { hours: number[]; deltas: bigint[] } | null {
  const hours: number[] = [];
  const deltas: bigint[] = [];
  if (run.deltas === undefined) return { hours, deltas };
  if (typeof run.deltas !== 'string' || !run.deltas.length) return null;
  const scale = run.deltaScale === undefined ? SCALE : BigInt(run.deltaScale);
  let hour = run.firstHour;
  for (const pair of run.deltas.split(' ')) {
    const fields = pair.split(':');
    if (fields.length !== 2 || !gapInteger.test(fields[0])) return null;
    const gap = Number(fields[0]);
    if (!Number.isSafeInteger(gap)) return null;
    hour += gap;
    if (!Number.isSafeInteger(hour) || hour > run.lastHour) return null;
    const rawToken = fields[1].startsWith('r');
    const token = rawToken ? fields[1].slice(1) : fields[1];
    if (!signed(token) || BigInt(token) === 0n) return null;
    // The first change establishes the scale; only MIN cannot use its magnitude.
    if (!hours.length && (rawToken || (token !== '1' && token !== '-1'
      && !(scale === 1n && token === MIN.toString())))) return null;
    const delta = rawToken ? BigInt(token) : BigInt(token) * scale;
    // Quotient tokens must encode an exact signed raw delta; divisible raw
    // tokens are not produced by the writer and have no second wire meaning.
    if (delta < MIN || delta > MAX || (rawToken && delta % scale === 0n)) return null;
    hours.push(hour);
    deltas.push(delta);
  }
  return { hours, deltas };
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
    if (known && pairs(run as unknown as NativeFixed5Run) === null) return false;
    last = run.lastHour;
  }
  return true;
}

function changes(run: NativeFixed5Run): Changes {
  const cached = decoded.get(run);
  if (cached) return cached;
  const parsed = pairs(run);
  if (!parsed) throw new Error('Malformed native fixed5 changes.');
  const values = new BigInt64Array(parsed.deltas.length + 1);
  const hours = new Float64Array(parsed.hours.length + 1);
  let raw = BigInt(run.raw!);
  values[0] = raw;
  hours[0] = run.firstHour;
  parsed.deltas.forEach((delta, index) => {
    raw = BigInt.asIntN(64, raw + delta);
    values[index + 1] = raw;
    hours[index + 1] = parsed.hours[index];
  });
  const result = { hours, values };
  decoded.set(run, result);
  return result;
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
