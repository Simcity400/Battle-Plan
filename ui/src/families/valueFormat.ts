import type { ValueStateProps } from '../components/ValueState';
import { formatDateTime, formatDay, gameDate, monthNames } from '../dashboard/calendar.ts';
import { InvalidFamilyValueError, assertValue, type Scalar, type Value } from './value.ts';

function assertFiniteScalar(raw: unknown, context: string): asserts raw is Scalar {
  if (raw === null || raw === undefined) {
    throw new InvalidFamilyValueError(`${context}: bare ${String(raw)} is not a valid scalar`);
  }
  if (typeof raw === 'number') {
    if (!Number.isFinite(raw)) {
      throw new InvalidFamilyValueError(`${context}: non-finite number (${String(raw)}) is not a valid scalar`);
    }
    return;
  }
  if (typeof raw === 'bigint' || typeof raw === 'string') return;
  throw new InvalidFamilyValueError(`${context}: unsupported scalar type ${typeof raw}`);
}

function assertFiniteNumber(raw: unknown, context: string): asserts raw is number {
  if (typeof raw !== 'number' || !Number.isFinite(raw)) {
    throw new InvalidFamilyValueError(`${context}: expected a finite number, received ${String(raw)}`);
  }
}

function exactTruncBigInt(value: number | bigint): bigint {
  if (typeof value === 'bigint') return value;
  const whole = Math.trunc(value);
  return BigInt(whole === 0 ? 0 : whole);
}

function decimalOf(value: number): { readonly units: bigint; readonly scale: number } {
  if (value === 0) return { units: 0n, scale: 0 };
  if (Number.isInteger(value)) return { units: BigInt(value), scale: 0 };
  const [mantissa = '', exponent = '0'] = String(value).split('e');
  const negative = mantissa.startsWith('-');
  const [whole = '', fraction = ''] = (negative ? mantissa.slice(1) : mantissa).split('.');
  let units = BigInt(`${whole}${fraction}`);
  let scale = fraction.length - Number(exponent);
  if (scale < 0) {
    units *= 10n ** BigInt(-scale);
    scale = 0;
  }
  while (scale > 0 && units % 10n === 0n) {
    units /= 10n;
    scale--;
  }
  return { units: negative ? -units : units, scale };
}

function groupDigits(digits: string): string {
  return digits.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

function smallestUnitStep(maxDecimals: number): string {
  return maxDecimals > 0 ? `0.${'0'.repeat(maxDecimals - 1)}1` : '1';
}

function formatRationalCut(
  numerator: bigint,
  denominator: bigint,
  maxDecimals: number,
  fixed: boolean,
  signed: boolean,
  percent: boolean,
  suffix: string,
): string {
  if (denominator <= 0n) {
    throw new InvalidFamilyValueError(`formatRational: non-positive denominator (${denominator}) is not valid`);
  }
  const pct = percent ? '%' : '';
  if (numerator === 0n) {
    const zero = fixed && maxDecimals > 0 ? `0.${'0'.repeat(maxDecimals)}` : '0';
    return `${zero}${pct}${suffix}`;
  }
  const negative = numerator < 0n;
  const abs = negative ? -numerator : numerator;
  const cut = (abs * (10n ** BigInt(maxDecimals))) / denominator;
  const sign = negative ? '\u2212' : signed ? '+' : '';
  if (cut === 0n) {
    return `${sign}<${smallestUnitStep(maxDecimals)}${pct}${suffix}`;
  }
  const digits = cut.toString().padStart(maxDecimals + 1, '0');
  const whole = groupDigits(digits.slice(0, digits.length - maxDecimals));
  const fracRaw = maxDecimals > 0 ? digits.slice(digits.length - maxDecimals) : '';
  const frac = fixed ? fracRaw : fracRaw.replace(/0+$/, '');
  return `${sign}${whole}${frac ? `.${frac}` : ''}${pct}${suffix}`;
}

function formatTruncatedFixed(
  units: bigint,
  scale: number,
  maxDecimals: number,
  fixed: boolean,
  signed: boolean,
  percent: boolean,
  suffix: string,
): string {
  return formatRationalCut(units, 10n ** BigInt(scale), maxDecimals, fixed, signed, percent, suffix);
}

export interface RationalFormatOptions {
  readonly maxDecimals?: number;
  readonly fixedDecimals?: boolean;
  readonly signed?: boolean;
  readonly percent?: boolean;
  readonly unit?: string;
}

export function formatRational(
  numerator: bigint,
  denominator: bigint,
  options?: RationalFormatOptions,
): string {
  const suffix = options?.unit ? ` ${options.unit}` : '';
  return formatRationalCut(
    numerator,
    denominator,
    options?.maxDecimals ?? 0,
    options?.fixedDecimals === true,
    options?.signed === true,
    options?.percent === true,
    suffix,
  );
}

export function formatScaledPercent(raw: bigint, scale: bigint, options?: { readonly signed?: boolean }): string {
  const decimals = scale > 1n ? (scale - 1n).toString().length : 0;
  return formatRationalCut(raw, scale, decimals, false, options?.signed === true, true, '');
}

export function formatShareRatio(numerator: bigint, denominator: bigint): string {
  if (denominator <= 0n) {
    throw new InvalidFamilyValueError(`formatShareRatio: non-positive denominator (${denominator}) is not valid`);
  }
  if (numerator < 0n) {
    throw new InvalidFamilyValueError(`formatShareRatio: negative share (${numerator}/${denominator}) is not valid`);
  }
  if (numerator > denominator) {
    throw new InvalidFamilyValueError(`formatShareRatio: a share above one (${numerator}/${denominator}) is not valid`);
  }
  if (numerator === 0n) return '0%';
  if (numerator === denominator) return '100%';
  if (numerator * 1000n < denominator) return '<0.1%';
  if (numerator * 10n >= denominator) {
    const whole = (numerator * 100n) / denominator;
    return `${whole > 99n ? 99n : whole}%`;
  }
  const tenths = (numerator * 1000n) / denominator;
  const clamped = tenths > 99n ? 99n : tenths < 1n ? 1n : tenths;
  return `${clamped / 10n}.${clamped % 10n}%`;
}

function toFixedDecimal(raw: number | bigint, decimals: number): { readonly units: bigint; readonly scale: number } {
  if (decimals > 0) {
    return { units: exactTruncBigInt(raw), scale: decimals };
  }
  if (typeof raw === 'bigint') {
    return { units: raw, scale: 0 };
  }
  return decimalOf(raw);
}

export type ValueFormat =
  | 'integer'
  | 'progress'
  | 'fixed5Percent'
  | 'fixed5Amount'
  | 'fixed5Width'
  | 'clockSeconds'
  | 'decimal1'
  | 'percent2'
  | 'share'
  | 'recordedPercent'
  | { readonly decimals: number; readonly percent?: boolean; readonly unit?: string; readonly signed?: boolean; readonly maxDecimals?: number; readonly fixedDecimals?: boolean };

export interface FormatValueOptions {
  readonly figure?: string;
  readonly gameWindow?: boolean;
  readonly compact?: boolean;
  readonly loading?: boolean;
}

export type FormattedValue =
  | { readonly kind: 'known'; readonly text: string; readonly exact: string; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'zero'; readonly text: string; readonly exact: string; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'notRecorded'; readonly label: string; readonly reason: string; readonly gameWindow?: boolean; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'notApplicable'; readonly label: '\u2014'; readonly reason: string; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'partial'; readonly text: string; readonly exact: string; readonly reason: string; readonly spoken: string; readonly audit: string };

export const NOT_RECORDED_LABEL = 'Not recorded';
export const NOT_APPLICABLE_LABEL = '\u2014';
export const GAME_WINDOW_MISSING_LABEL = 'n/a';
export const LOADING_LABEL = 'Loading…';
export const PARTIAL_LABEL = 'partial';

export function notRecordedLabel(figure?: string): string {
  const clean = figure?.trim();
  return clean ? `${clean} not recorded` : NOT_RECORDED_LABEL;
}

function formatClockSeconds(seconds: number | bigint): string {
  const floored = typeof seconds === 'bigint' ? seconds : BigInt(Math.floor(seconds));
  const total = floored < 0n ? 0n : floored;
  const minutes = (total / 60n) % 60n;
  const secs = total % 60n;
  return `${total / 3600n}:${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
}

export function formatFixed5Amount(value: number | bigint): string {
  assertFiniteScalar(value, 'formatFixed5Amount');
  const big = exactTruncBigInt(value);
  if (big === 0n) return '0';
  if (big < 0n) return '!';
  return formatTruncatedFixed(big, 5, 2, false, false, false, '');
}

export function formatScalar(raw: Scalar, format?: ValueFormat): string {
  assertFiniteScalar(raw, 'formatScalar');
  if (typeof raw === 'string') return raw;
  if (!format || format === 'integer') {
    const { units, scale } = toFixedDecimal(raw, 0);
    return formatTruncatedFixed(units, scale, 0, false, false, false, '');
  }
  if (format === 'progress') return formatProgress(Number(raw) / 100);
  if (format === 'clockSeconds') return formatClockSeconds(raw);
  if (format === 'share') return formatShare(Number(raw));
  if (format === 'recordedPercent') return formatRecordedPercent(raw);
  if (format === 'decimal1') {
    const { units, scale } = toFixedDecimal(raw, 0);
    return formatTruncatedFixed(units, scale, 1, false, false, false, '');
  }
  if (format === 'percent2') {
    const { units, scale } = toFixedDecimal(raw, 0);
    return formatTruncatedFixed(units, scale, 2, units !== 0n, false, true, '');
  }
  if (format === 'fixed5Percent') {
    const big = exactTruncBigInt(raw);
    if (big <= 0n) return '0%';
    if (big >= 10_000_000n) return '100%';
    return formatTruncatedFixed(big, 5, 0, false, false, true, '');
  }
  if (format === 'fixed5Amount') return formatFixed5Amount(raw);
  if (format === 'fixed5Width') {
    const big = exactTruncBigInt(raw);
    return formatTruncatedFixed(big, 5, 1, false, false, false, '');
  }
  const suffix = format.unit ? ` ${format.unit}` : '';
  const fixed = format.fixedDecimals === true;
  const signed = format.signed === true;
  const percent = format.percent === true;
  if (format.decimals > 0) {
    let units = exactTruncBigInt(raw);
    if (percent) units *= 100n;
    const shownDecimals = format.maxDecimals ?? format.decimals;
    return formatTruncatedFixed(units, format.decimals, shownDecimals, fixed, signed, percent, suffix);
  }
  const { units, scale } = toFixedDecimal(raw, 0);
  const shownDecimals = format.maxDecimals ?? 0;
  return formatTruncatedFixed(units, scale, shownDecimals, fixed, signed, percent, suffix);
}

export function formatExactScalar(raw: Scalar, format?: ValueFormat): string {
  assertFiniteScalar(raw, 'formatExactScalar');
  if (typeof raw === 'string') return raw;
  if (format === 'clockSeconds') return formatClockSeconds(raw);
  if (format === 'share') {
    if (typeof raw === 'bigint') {
      if (raw < 0n || raw > 1n) {
        throw new InvalidFamilyValueError(`formatExactScalar: share out of [0, 1] (${raw})`);
      }
      return raw === 0n ? '0%' : '100%';
    }
    assertFiniteNumber(raw, 'formatExactScalar');
    const { units, scale } = decimalOf(raw);
    if (units < 0n || units > 10n ** BigInt(scale)) {
      throw new InvalidFamilyValueError(`formatExactScalar: share out of [0, 1] (${raw})`);
    }
    return formatTruncatedFixed(units * 100n, scale, scale, false, false, true, '');
  }
  if (format === 'progress' || format === 'recordedPercent') {
    const { units, scale } = toFixedDecimal(raw, 0);
    return formatTruncatedFixed(units, scale, scale, false, false, true, '');
  }
  if (format === 'fixed5Percent') return formatTruncatedFixed(exactTruncBigInt(raw), 5, 5, false, false, true, '');
  if (format === 'fixed5Amount') {
    const big = exactTruncBigInt(raw);
    return big < 0n ? formatFixed5Amount(big) : formatTruncatedFixed(big, 5, 5, false, false, false, '');
  }
  if (format === 'fixed5Width') return formatTruncatedFixed(exactTruncBigInt(raw), 5, 5, false, false, false, '');
  if (typeof format === 'object' && format.decimals > 0) {
    const units = exactTruncBigInt(raw) * (format.percent ? 100n : 1n);
    const suffix = format.unit ? ` ${format.unit}` : '';
    return formatTruncatedFixed(units, format.decimals, format.decimals, false, format.signed === true, format.percent === true, suffix);
  }
  const { units, scale } = toFixedDecimal(raw, 0);
  const percent = format === 'percent2' || (typeof format === 'object' && format.percent === true);
  const signed = typeof format === 'object' && format.signed === true;
  const suffix = typeof format === 'object' && format.unit ? ` ${format.unit}` : '';
  return formatTruncatedFixed(units, scale, scale, false, signed, percent, suffix);
}

export function formatShare(share: number): string {
  assertFiniteNumber(share, 'formatShare');
  const { units, scale } = decimalOf(share);
  return formatShareRatio(units, 10n ** BigInt(scale));
}

export function formatProgress(fraction: number): string {
  assertFiniteNumber(fraction, 'formatProgress');
  if (fraction <= 0) return '0%';
  const { units, scale } = decimalOf(fraction);
  return `${(units * 100n) / 10n ** BigInt(scale)}%`;
}

export function formatRecordedPercent(
  value: number | bigint,
  options?: { readonly signed?: boolean; readonly decimals?: number },
): string {
  assertFiniteScalar(value, 'formatRecordedPercent');
  if (typeof value === 'string') {
    throw new InvalidFamilyValueError('formatRecordedPercent: expected a numeric value');
  }
  if (options?.decimals !== undefined && options.decimals > 0) {
    return formatTruncatedFixed(exactTruncBigInt(value), options.decimals, options.decimals, false, options.signed === true, true, '');
  }
  const { units, scale } = toFixedDecimal(value, 0);
  return formatTruncatedFixed(units, scale, Math.min(5, scale), false, options?.signed === true, true, '');
}

export interface CompactScalar {
  readonly text: string;
  readonly exact: string;
  readonly compacted: boolean;
}

const COMPACT_TIERS = [
  { exp: 12, threshold: 10n ** 12n, suffix: 'T' },
  { exp: 9, threshold: 10n ** 9n, suffix: 'B' },
  { exp: 6, threshold: 10n ** 6n, suffix: 'M' },
  { exp: 3, threshold: 10n ** 3n, suffix: 'K' },
] as const;

export function formatCompactRational(
  numerator: bigint,
  denominator: bigint,
  options?: Omit<RationalFormatOptions, 'percent'>,
): CompactScalar {
  const exact = formatRational(numerator, denominator, options);
  const abs = numerator < 0n ? -numerator : numerator;
  const whole = abs / denominator;
  if (whole < 1000n) {
    return { text: exact, exact, compacted: false };
  }
  const tier = COMPACT_TIERS.find(entry => whole >= entry.threshold);
  if (!tier) return { text: exact, exact, compacted: false };
  const wholeInTier = whole / tier.threshold;
  const compactDecimals = wholeInTier >= 100n ? 0 : wholeInTier >= 10n ? 1 : 2;
  const unitSuffix = options?.unit ? ` ${options.unit}` : '';
  const text = formatRationalCut(
    numerator,
    denominator * tier.threshold,
    compactDecimals,
    false,
    options?.signed === true,
    false,
    `${tier.suffix}${unitSuffix}`,
  );
  return { text, exact, compacted: true };
}

export function formatCompactScalar(raw: Scalar, format?: ValueFormat): CompactScalar {
  const shown = formatScalar(raw, format);
  const exact = formatExactScalar(raw, format);
  if (typeof raw === 'string') return { text: shown, exact, compacted: false };
  if (
    format === 'clockSeconds'
    || format === 'fixed5Percent'
    || format === 'percent2'
    || format === 'progress'
    || format === 'share'
    || format === 'recordedPercent'
  ) {
    return { text: shown, exact, compacted: false };
  }
  if (typeof format === 'object' && format.percent) {
    return { text: shown, exact, compacted: false };
  }
  const { units, scale } = format === 'fixed5Amount' || format === 'fixed5Width'
    ? { units: exactTruncBigInt(raw), scale: 5 }
    : typeof format === 'object' && format.decimals > 0
      ? { units: exactTruncBigInt(raw), scale: format.decimals }
      : toFixedDecimal(raw, 0);
  const negative = units < 0n;
  const absUnits = negative ? -units : units;
  const whole = absUnits / (10n ** BigInt(scale));
  if (whole < 1000n) {
    return { text: shown, exact, compacted: false };
  }
  const tier = COMPACT_TIERS.find(entry => whole >= entry.threshold);
  if (!tier) return { text: shown, exact, compacted: false };
  const wholeInTier = whole / tier.threshold;
  const compactDecimals = wholeInTier >= 100n ? 0 : wholeInTier >= 10n ? 1 : 2;
  const signed = typeof format === 'object' && format.signed === true;
  const unitSuffix = typeof format === 'object' && format.unit ? ` ${format.unit}` : '';
  const text = formatTruncatedFixed(units, scale + tier.exp, compactDecimals, false, signed, false, `${tier.suffix}${unitSuffix}`);
  return { text, exact, compacted: true };
}

export function formatValueState(value: Value, format?: ValueFormat, options?: FormatValueOptions): FormattedValue {
  assertValue(value, 'formatValueState');
  switch (value.kind) {
    case 'known': {
      const text = options?.compact ? formatCompactScalar(value.value, format).text : formatScalar(value.value, format);
      const exact = formatExactScalar(value.value, format);
      return { kind: 'known', text, exact, spoken: exact, audit: exact };
    }
    case 'zero': {
      const text = formatScalar(value.value, format);
      return { kind: 'zero', text, exact: text, spoken: text, audit: text };
    }
    case 'notRecorded': {
      const label = options?.loading ? LOADING_LABEL : options?.gameWindow ? GAME_WINDOW_MISSING_LABEL : notRecordedLabel(options?.figure);
      const lead = options?.loading ? 'Loading' : options?.figure?.trim() ? `${options.figure.trim()} not recorded` : NOT_RECORDED_LABEL;
      const spoken = `${lead}: ${value.reason}`;
      return {
        kind: 'notRecorded',
        label,
        reason: value.reason,
        ...(options?.gameWindow ? { gameWindow: true } : {}),
        spoken,
        audit: spoken,
      };
    }
    case 'notApplicable': {
      const spoken = `Not applicable: ${value.reason}`;
      return { kind: 'notApplicable', label: NOT_APPLICABLE_LABEL, reason: value.reason, spoken, audit: spoken };
    }
    case 'partial': {
      const text = options?.compact ? formatCompactScalar(value.value, format).text : formatScalar(value.value, format);
      const exact = formatExactScalar(value.value, format);
      const spoken = `${exact}, partial: ${value.reason}`;
      return {
        kind: 'partial',
        text,
        exact,
        reason: value.reason,
        spoken,
        audit: spoken,
      };
    }
  }
}

export interface DateColumnParts {
  readonly dayMonth: string;
  readonly year: string;
  readonly full: string;
  readonly hover: string;
}

export function formatDateOnly(hour: number): string {
  return formatDay(Math.floor(hour));
}

export function formatDateHover(hour: number): string {
  return formatDateTime(Math.floor(hour));
}

export function formatDateParts(hour: number): DateColumnParts | null {
  const whole = Math.floor(hour);
  const date = gameDate(whole);
  if (!date) return null;
  const month = monthNames[date.month - 1]!.slice(0, 3);
  const dayMonth = `${date.day} ${month}`;
  const year = String(date.year);
  return {
    dayMonth,
    year,
    full: `${dayMonth} ${year}`,
    hover: formatDateTime(whole),
  };
}

export function valueStateProps(value: Value, format?: ValueFormat, narrow = false): ValueStateProps {
  const formatted = formatValueState(value, format);
  switch (formatted.kind) {
    case 'known':
    case 'zero':
      return { kind: formatted.kind, text: formatted.exact, ...(narrow ? { narrow: true } : {}) };
    case 'notRecorded':
      return { kind: 'missing', reason: formatted.reason, ...(narrow ? { narrow: true } : {}) };
    case 'notApplicable':
      return { kind: 'notApplicable', reason: formatted.reason, ...(narrow ? { narrow: true } : {}) };
    case 'partial':
      return {
        kind: 'partial',
        text: formatted.exact,
        reason: formatted.reason,
        ...(narrow ? { narrow: true } : {}),
      };
  }
}

export function spokenValue(value: Value, format?: ValueFormat): string {
  return formatValueState(value, format).spoken;
}

export function defaultWidestSamples(format?: ValueFormat): readonly string[] {
  if (!format) return [];
  if (format === 'fixed5Percent' || format === 'progress' || format === 'share' || format === 'recordedPercent') return ['100%'];
  if (format === 'fixed5Amount') return ['999.99'];
  if (format === 'fixed5Width') return ['99.9'];
  if (format === 'clockSeconds') return ['99:59:59'];
  if (format === 'decimal1') return ['999.9'];
  if (format === 'percent2') return ['100.00%'];
  if (format === 'integer') return ['9,999,999'];
  if (format.percent) return ['100.00%'];
  const frac = format.decimals > 0 ? `.${'9'.repeat(Math.min(format.decimals, 2))}` : '';
  const suffix = format.unit ? ` ${format.unit}` : '';
  return [`9,999,999${frac}${suffix}`];
}
