import { compareTags } from '../dashboard/tagOrder.ts';
import type { LiveValues } from '../charts/live';
import { NO_VALUE, NOT_READ, NOT_RECORDED } from '../dashboard/follow/liveCells.ts';
import { CELL, cellNumber, cellState, cellValue, rowName, rowReason } from '../dashboard/follow/message.ts';
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
  switch (candidate.kind) {
    case 'known':
      validScalar(candidate.value, `${context} (known)`);
      if (candidate.value === 0 || candidate.value === 0n) {
        throw new InvalidFamilyValueError(`${context}: numeric zero must use the 'zero' state, not 'known'`);
      }
      return;
    case 'zero':
      if (candidate.value !== 0 && candidate.value !== 0n) {
        throw new InvalidFamilyValueError(`${context} (zero): expected 0 or 0n`);
      }
      return;
    case 'notRecorded':
      validReason(candidate.reason, `${context} (notRecorded)`);
      return;
    case 'notApplicable':
      validReason(candidate.reason, `${context} (notApplicable)`);
      return;
    case 'partial':
      validScalar(candidate.value, `${context} (partial)`);
      validReason(candidate.reason, `${context} (partial)`);
      return;
    default:
      throw new InvalidFamilyValueError(`${context}: unknown family value state ${String(candidate.kind)}`);
  }
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

const rowIndexes = new WeakMap<FollowedRows, Map<string, number>>();

export function answerRowIndex(answer: FollowedRows): ReadonlyMap<string, number> {
  let index = rowIndexes.get(answer);
  if (!index) {
    index = new Map<string, number>();
    for (let row = 0; row < answer.rows; row++) {
      const name = rowName(answer, row);
      if (name !== null && !index.has(name)) index.set(name, row);
    }
    rowIndexes.set(answer, index);
  }
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

export function valueFromLive(live: LiveValues<number | bigint | string> | null | undefined, key: string, options?: LiveCellOptions): Value<NumericScalar> {
  if (!live) return notRecorded(options?.overrideMissing ?? NOT_READ);
  const rawExact = live.raw?.(key);
  const raw = rawExact !== undefined ? rawExact : live.get(key);
  if (raw === undefined || typeof raw === 'string') {
    const reason = options?.overrideMissing ?? (typeof raw === 'string' && raw ? raw : live.missing(key)) ?? NOT_RECORDED;
    return options?.missingAs === 'notApplicable' ? notApplicable(reason || NOT_RECORDED) : notRecorded(reason || NOT_RECORDED);
  }
  validScalar(raw, `valueFromLive(${key})`);
  if (options?.partialReason) return partialValue(raw, options.partialReason, { atLeast: options.atLeast });
  return knownValue(raw);
}

const textCollator = new Intl.Collator(undefined, { numeric: true });

export function compareValues(a: Value, b: Value, direction: 'ascending' | 'descending'): number {
  const aAbsent = isAbsentValue(a), bAbsent = isAbsentValue(b);
  if (aAbsent || bAbsent) return aAbsent === bAbsent ? 0 : aAbsent ? 1 : -1;
  const av = a.value, bv = b.value;
  const sign = direction === 'ascending' ? 1 : -1;
  if (typeof av === 'string' || typeof bv === 'string') {
    return sign * textCollator.compare(String(av), String(bv));
  }
  const order = av < bv ? -1 : av > bv ? 1 : 0;
  return sign * order;
}

export function compareTie(nameA: string, keyA: string, nameB: string, keyB: string): number {
  return textCollator.compare(nameA, nameB) || compareTags(keyA, keyB);
}

export function compareValueRows(
  a: { readonly value: Value; readonly name: string; readonly key: string },
  b: { readonly value: Value; readonly name: string; readonly key: string },
  direction: 'ascending' | 'descending',
): number {
  return compareValues(a.value, b.value, direction) || compareTie(a.name, a.key, b.name, b.key);
}
