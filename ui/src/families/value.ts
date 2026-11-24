import { compareTags } from '../dashboard/tagOrder.ts';
import type { LiveValues } from '../charts/live';
import { NO_VALUE, NOT_READ, NOT_RECORDED } from '../dashboard/follow/liveCells.ts';
import { CELL, cellNumber, cellState, cellValue, rowName, rowNameText, rowReason } from '../dashboard/follow/message.ts';
import type { FollowedAnswer, FollowedRows } from '../dashboard/follow/message';

export type Scalar = number | bigint | string;
export type NumericScalar = number | bigint;

export type Value<T extends Scalar = Scalar> =
  | { readonly kind: 'known'; readonly value: T }
  | { readonly kind: 'zero'; readonly value: 0 | 0n }
  | { readonly kind: 'notRecorded'; readonly reason: string }
  | { readonly kind: 'notApplicable'; readonly reason: string }
  | { readonly kind: 'partial'; readonly value: T; readonly reason: string; readonly atLeast?: boolean };

export type NumericValue = Value<NumericScalar>;
export type TextValue = Value<string>;

export class InvalidFamilyValueError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'InvalidFamilyValueError';
  }
}

function validScalar(value: unknown, context: string): asserts value is Scalar {
  if (value === null || value === undefined) throw new InvalidFamilyValueError(`${context}: bare ${String(value)} is not a valid family value`);
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) throw new InvalidFamilyValueError(`${context}: non-finite number (${String(value)}) is not a valid family value`);
    return;
  }
  if (typeof value === 'bigint') return;
  if (typeof value === 'string') {
    if (!value.trim()) throw new InvalidFamilyValueError(`${context}: blank text is not a valid family value`);
    return;
  }
  throw new InvalidFamilyValueError(`${context}: unsupported scalar type ${typeof value}`);
}

function validReason(reason: unknown, context: string): asserts reason is string {
  if (typeof reason !== 'string' || !reason.trim()) {
    throw new InvalidFamilyValueError(`${context}: a non-empty reason string is required`);
  }
}

export function knownValue<T extends Scalar>(value: T): Value<T> {
  validScalar(value, 'knownValue');
  if (value === 0 || value === 0n) return { kind: 'zero', value: value as 0 | 0n };
  return { kind: 'known', value };
}

export function zeroValue(value?: 0): Value<number>;
export function zeroValue(value: 0n): Value<bigint>;
export function zeroValue(value: 0 | 0n = 0): Value<NumericScalar> {
  if (value !== 0 && value !== 0n) throw new InvalidFamilyValueError(`zeroValue: expected 0 or 0n, received ${String(value)}`);
  return { kind: 'zero', value };
}

export function notRecorded(reason: string): Value<never> {
  validReason(reason, 'notRecorded');
  return { kind: 'notRecorded', reason };
}

export function notApplicable(reason: string): Value<never> {
  validReason(reason, 'notApplicable');
  return { kind: 'notApplicable', reason };
}

export function partialValue<T extends Scalar>(value: T, reason: string, options?: { atLeast?: boolean }): Value<T> {
  validScalar(value, 'partialValue');
  validReason(reason, 'partialValue');
  return options?.atLeast ? { kind: 'partial', value, reason, atLeast: true } : { kind: 'partial', value, reason };
}

export function assertValue(value: unknown, context = 'family value'): asserts value is Value {
  if (value === null || value === undefined || typeof value !== 'object') {
    throw new InvalidFamilyValueError(`${context}: expected a 5-state family Value, received ${value === null ? 'null' : typeof value}`);
  }
  const candidate = value as Record<string, unknown>;
  const kind = candidate.kind;
  if (kind === 'known') {
    const v = candidate.value;
    if (typeof v === 'number') {
      if (v === 0) throw new InvalidFamilyValueError(`${context}: numeric zero must use the 'zero' state, not 'known'`);
      if (Number.isFinite(v)) return;
    } else if (typeof v === 'bigint') {
      if (v === 0n) throw new InvalidFamilyValueError(`${context}: numeric zero must use the 'zero' state, not 'known'`);
      return;
    }
    validScalar(v, `${context} (known)`);
    return;
  }
  if (kind === 'zero') {
    if (candidate.value !== 0 && candidate.value !== 0n) {
      throw new InvalidFamilyValueError(`${context} (zero): expected 0 or 0n`);
    }
    return;
  }
  if (kind === 'notRecorded') {
    validReason(candidate.reason, `${context} (notRecorded)`);
    return;
  }
  if (kind === 'notApplicable') {
    validReason(candidate.reason, `${context} (notApplicable)`);
    return;
  }
  if (kind === 'partial') {
    validScalar(candidate.value, `${context} (partial)`);
    validReason(candidate.reason, `${context} (partial)`);
    return;
  }
  throw new InvalidFamilyValueError(`${context}: unknown family value state ${String(kind)}`);
}

export function hasMeasuredValue(value: Value): value is Extract<Value, { kind: 'known' | 'zero' | 'partial' }> {
  return value.kind === 'known' || value.kind === 'zero' || value.kind === 'partial';
}

export function valueScalar(value: Value): Scalar | undefined {
  return hasMeasuredValue(value) ? value.value : undefined;
}

export function valueNumber(value: Value): number | undefined {
  if (!hasMeasuredValue(value)) return undefined;
  const raw = value.value;
  return typeof raw === 'number' ? raw : typeof raw === 'bigint' ? Number(raw) : undefined;
}

export function isAbsentValue(value: Value): value is Extract<Value, { kind: 'notRecorded' | 'notApplicable' }> {
  return value.kind === 'notRecorded' || value.kind === 'notApplicable';
}

export interface NullableMeaning {
  readonly kind: 'notRecorded' | 'notApplicable';
  readonly reason: string;
}

export function textValue(raw: string | null | undefined, reason: string): Value<string> {
  return raw === null || raw === undefined || !raw.trim() ? notRecorded(reason) : knownValue(raw);
}

export function valueFromNullable<T extends Scalar>(
  raw: T | null | undefined,
  whenNull: NullableMeaning | (() => NullableMeaning),
  options?: { partial?: string | null; atLeast?: boolean },
): Value<T> {
  if (raw === null || raw === undefined) {
    const meaning = typeof whenNull === 'function' ? whenNull() : whenNull;
    return meaning.kind === 'notApplicable' ? notApplicable(meaning.reason) : notRecorded(meaning.reason);
  }
  validScalar(raw, 'valueFromNullable');
  if (options?.partial) return partialValue(raw, options.partial, { atLeast: options.atLeast });
  return knownValue(raw);
}

export function valueFromRawBigInt(
  raw: string | null | undefined,
  whenNull: NullableMeaning | (() => NullableMeaning),
  options?: { partial?: string | null; atLeast?: boolean },
): Value<bigint> {
  if (raw === null || raw === undefined) {
    const meaning = typeof whenNull === 'function' ? whenNull() : whenNull;
    return meaning.kind === 'notApplicable' ? notApplicable(meaning.reason) : notRecorded(meaning.reason);
  }
  if (typeof raw !== 'string' || !/^-?\d+$/.test(raw)) {
    throw new InvalidFamilyValueError(`valueFromRawBigInt: invalid integer string "${String(raw)}"`);
  }
  const parsed = BigInt(raw);
  if (options?.partial) return partialValue(parsed, options.partial, { atLeast: options.atLeast });
  return knownValue(parsed);
}

export interface WireCellOptions {
  readonly missingAs?: 'notRecorded' | 'notApplicable';
  readonly missingReason?: string;
  readonly unrecordedReason?: string;
  readonly absentReason?: string;
  readonly partialReason?: string | null;
  readonly atLeast?: boolean;
}

const floatView = new DataView(new ArrayBuffer(8));

export function wireFloat(answer: FollowedRows, at: number, options?: WireCellOptions): Value<number> {
  const state = cellState(answer, at);
  const row = Math.floor(at / Math.max(1, answer.columns));
  if (state === CELL.value) {
    floatView.setBigInt64(0, cellValue(answer, at), true);
    const num = floatView.getFloat64(0, true);
    validScalar(num, 'wireFloat');
    return options?.partialReason ? partialValue(num, options.partialReason, { atLeast: options.atLeast }) : knownValue(num);
  }
  const reason = rowReason(answer, row);
  if (state === CELL.missing) {
    const text = reason ?? options?.missingReason ?? NO_VALUE;
    return options?.missingAs === 'notApplicable' ? notApplicable(text) : notRecorded(text);
  }
  return notRecorded(reason ?? options?.unrecordedReason ?? NOT_RECORDED);
}

export function wireWhole(answer: FollowedRows, at: number, options?: WireCellOptions): Value<bigint> {
  const state = cellState(answer, at);
  const row = Math.floor(at / Math.max(1, answer.columns));
  if (state === CELL.value) {
    const raw = cellValue(answer, at);
    return options?.partialReason ? partialValue(raw, options.partialReason, { atLeast: options.atLeast }) : knownValue(raw);
  }
  const reason = rowReason(answer, row);
  if (state === CELL.missing) {
    const text = reason ?? options?.missingReason ?? NO_VALUE;
    return options?.missingAs === 'notApplicable' ? notApplicable(text) : notRecorded(text);
  }
  return notRecorded(reason ?? options?.unrecordedReason ?? NOT_RECORDED);
}

export function valueFromWireCell(answer: FollowedAnswer | undefined, row: number, col: number, options?: WireCellOptions): Value<NumericScalar> {
  if (!answer) return notRecorded(options?.absentReason ?? NOT_READ);
  if (answer.kind === 'notRecorded') return notRecorded(answer.reason);
  if (row < 0 || row >= answer.rows || col < 0 || col >= answer.columns) {
    return notRecorded(options?.absentReason ?? 'Not in the answer');
  }
  const at = row * answer.columns + col;
  const state = cellState(answer, at);
  if (state === CELL.value) {
    const num = cellNumber(answer, at);
    const raw: NumericScalar = num !== null ? num : cellValue(answer, at);
    validScalar(raw, 'valueFromWireCell');
    return options?.partialReason ? partialValue(raw, options.partialReason, { atLeast: options.atLeast }) : knownValue(raw);
  }
  const reason = rowReason(answer, row);
  if (state === CELL.missing) {
    const text = reason ?? options?.missingReason ?? NO_VALUE;
    return options?.missingAs === 'notApplicable' ? notApplicable(text) : notRecorded(text);
  }
  return notRecorded(reason ?? options?.unrecordedReason ?? NOT_RECORDED);
}

const rowIndexes = new WeakMap<object, ReadonlyMap<string, number>>();

interface HeldNames {
  readonly bytes: Uint8Array;
  readonly starts: Int32Array;
  readonly lengths: Int32Array;
  readonly index: ReadonlyMap<string, number>;
}

const HELD_PER_SIZE = 8;
const HELD_SIZES = 64;
const heldNames = new Map<number, HeldNames[]>();

function sameNames(answer: FollowedRows, held: HeldNames): boolean {
  const u8 = answer.views.u8;
  for (let row = 0; row < answer.rows; row++) {
    const text = rowNameText(answer, row);
    const length = held.lengths[row]!;
    if (text < 0) {
      if (length !== -1) return false;
      continue;
    }
    const start = answer.text + answer.ends[text]!;
    if (answer.ends[text + 1]! - answer.ends[text]! !== length) return false;
    const from = held.starts[row]!;
    for (let i = 0; i < length; i++) if (u8[start + i] !== held.bytes[from + i]) return false;
  }
  return true;
}

function holdNames(answer: FollowedRows, index: ReadonlyMap<string, number>): HeldNames {
  const starts = new Int32Array(answer.rows);
  const lengths = new Int32Array(answer.rows);
  let total = 0;
  for (let row = 0; row < answer.rows; row++) {
    const text = rowNameText(answer, row);
    lengths[row] = text < 0 ? -1 : answer.ends[text + 1]! - answer.ends[text]!;
    if (text >= 0) total += lengths[row]!;
  }
  const bytes = new Uint8Array(total);
  let at = 0;
  for (let row = 0; row < answer.rows; row++) {
    starts[row] = at;
    const text = rowNameText(answer, row);
    if (text < 0) continue;
    const start = answer.text + answer.ends[text]!;
    bytes.set(answer.views.u8.subarray(start, start + lengths[row]!), at);
    at += lengths[row]!;
  }
  return { bytes, starts, lengths, index };
}

export function answerRowIndex(answer: FollowedRows): ReadonlyMap<string, number> {
  const cached = rowIndexes.get(answer.decoded);
  if (cached) return cached;
  const held = heldNames.get(answer.rows) ?? [];
  let index = held.find(names => sameNames(answer, names))?.index;
  if (!index) {
    const built = new Map<string, number>();
    for (let row = 0; row < answer.rows; row++) {
      const name = rowName(answer, row);
      if (name !== null && !built.has(name)) built.set(name, row);
    }
    index = built;
    held.unshift(holdNames(answer, built));
    if (held.length > HELD_PER_SIZE) held.length = HELD_PER_SIZE;
    heldNames.delete(answer.rows);
    heldNames.set(answer.rows, held);
    if (heldNames.size > HELD_SIZES) heldNames.delete(heldNames.keys().next().value!);
  }
  rowIndexes.set(answer.decoded, index);
  return index;
}

export function valueFromNamedAnswer(answer: FollowedAnswer | undefined, name: string, options?: WireCellOptions): Value<NumericScalar> {
  if (!answer) return notRecorded(options?.absentReason ?? NOT_READ);
  if (answer.kind === 'notRecorded') return notRecorded(answer.reason);
  const row = answerRowIndex(answer).get(name);
  if (row === undefined) return notRecorded(options?.absentReason ?? 'Not in the answer');
  return valueFromWireCell(answer, row, 0, options);
}

export interface LiveCellOptions {
  readonly missingAs?: 'notRecorded' | 'notApplicable';
  readonly overrideMissing?: string | null;
  readonly partialReason?: string | null;
  readonly atLeast?: boolean;
}

const SAFE_INT_BOUND = 9_007_199_254_740_991;

export function valueFromLive(live: LiveValues<number | bigint | string> | null | undefined, key: string, options?: LiveCellOptions): Value<NumericScalar> {
  if (!live) return notRecorded(options?.overrideMissing ?? NOT_READ);
  const got = live.get(key);
  let raw: number | bigint | string | undefined = got;
  if (typeof got === 'number') {
    if (Math.abs(got) >= SAFE_INT_BOUND && live.raw) {
      const exact = live.raw(key);
      if (exact !== undefined) raw = exact;
    }
  } else if (got === undefined && live.raw) {
    raw = live.raw(key);
  }
  if (raw === undefined || typeof raw === 'string') {
    const reason = options?.overrideMissing ?? (typeof raw === 'string' && raw ? raw : live.missing(key)) ?? NOT_RECORDED;
    return options?.missingAs === 'notApplicable' ? notApplicable(reason || NOT_RECORDED) : notRecorded(reason || NOT_RECORDED);
  }
  if (typeof raw === 'number' && !Number.isFinite(raw)) {
    validScalar(raw, `valueFromLive(${key})`);
  }
  if (options?.partialReason) return partialValue(raw, options.partialReason, { atLeast: options.atLeast });
  return raw === 0 || raw === 0n ? { kind: 'zero', value: raw } : { kind: 'known', value: raw };
}

const textCollator = new Intl.Collator(undefined, { numeric: true });

export function compareValues(a: Value, b: Value, direction: 'ascending' | 'descending'): number {
  const aAbsent = isAbsentValue(a), bAbsent = isAbsentValue(b);
  if (aAbsent || bAbsent) return aAbsent === bAbsent ? 0 : aAbsent ? 1 : -1;
  const av = a.value, bv = b.value;
  const sign = direction === 'ascending' ? 1 : -1;
  if (typeof av === 'string' || typeof bv === 'string') {
    return av === bv ? 0 : sign * textCollator.compare(String(av), String(bv));
  }
  const order = av < bv ? -1 : av > bv ? 1 : 0;
  return sign * order;
}

export function compareTie(nameA: string, keyA: string, nameB: string, keyB: string): number {
  return (nameA === nameB ? 0 : textCollator.compare(nameA, nameB)) || compareTags(keyA, keyB);
}

export function compareValueRows(
  a: { readonly value: Value; readonly name: string; readonly key: string },
  b: { readonly value: Value; readonly name: string; readonly key: string },
  direction: 'ascending' | 'descending',
): number {
  return compareValues(a.value, b.value, direction) || compareTie(a.name, a.key, b.name, b.key);
}
