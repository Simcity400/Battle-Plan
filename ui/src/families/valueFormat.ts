import type { ValueStateProps } from '../components/ValueState';
import { formatNativeFixed5 } from '../charts/valueAxis.ts';
import { formatDateTime, formatDay, gameDate, monthNames } from '../dashboard/calendar.ts';
import { canonicalAuditValue } from './audit.ts';
import { assertValue, type Scalar, type Value } from './value.ts';

function percentCell(raw: string | bigint): string {
  const value = BigInt(raw);
  const rounded = BigInt.asIntN(32, (value + (value < 0n ? -50000n : 50000n)) / 100000n);
  const clamped = rounded < 0n ? 0n : rounded > 100n ? 100n : rounded;
  return `${formatNativeFixed5(clamped * 100000n, 0)}%`;
}

function exactTruncBigInt(value: number | bigint): bigint {
  if (typeof value === 'bigint') return value;
  const whole = Math.trunc(value);
  return BigInt(whole === 0 ? 0 : whole);
}

function groupDigits(digits: string): string {
  return digits.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
}

function formatFixedDecimal(raw: number | bigint, maxDecimals: number, fixed: boolean, group: boolean, minus: string, plus = false): string {
  if (typeof raw === 'bigint' || (Number.isFinite(raw) && Math.abs(raw) >= 2 ** 53)) {
    const big = exactTruncBigInt(raw);
    if (big === 0n) return '0';
    const negative = big < 0n;
    const digits = (negative ? -big : big).toString();
    const whole = group ? groupDigits(digits) : digits;
    const frac = fixed && maxDecimals > 0 ? `.${'0'.repeat(maxDecimals)}` : '';
    return `${negative ? minus : plus ? '+' : ''}${whole}${frac}`;
  }
  if (raw === 0) return '0';
  const negative = raw < 0;
  const rounded = Math.abs(raw).toFixed(maxDecimals);
  const dot = rounded.indexOf('.');
  const wholeDigits = dot < 0 ? rounded : rounded.slice(0, dot);
  const fracRaw = dot < 0 ? '' : rounded.slice(dot + 1);
  const frac = fixed ? fracRaw : fracRaw.replace(/0+$/, '');
  const isZero = /^0+$/.test(wholeDigits) && (!frac || /^0+$/.test(frac));
  if (isZero) return fixed && maxDecimals > 0 ? `0.${'0'.repeat(maxDecimals)}` : '0';
  const whole = group ? groupDigits(wholeDigits) : wholeDigits;
  return `${negative ? minus : plus ? '+' : ''}${whole}${frac ? `.${frac}` : ''}`;
}

function amountText(value: number | bigint, decimals: number, percent = false): string {
  let raw: number | bigint = typeof value === 'bigint'
    ? value
    : Math.abs(value) >= 2 ** 53
      ? exactTruncBigInt(value)
      : Math.trunc(value);
  if (percent) raw = typeof raw === 'number' && Math.abs(raw) < 2 ** 46 ? raw * 100 : BigInt(raw) * 100n;
  const negative = raw < 0;
  const abs = negative ? -raw : raw;
  const digits = (typeof abs === 'bigint' ? abs.toString() : abs >= 2 ** 53 ? BigInt(abs).toString() : String(abs)).padStart(decimals + 1, '0');
  const whole = groupDigits(digits.slice(0, digits.length - decimals));
  const fraction = decimals ? digits.slice(digits.length - decimals).replace(/0+$/, '') : '';
  return `${negative ? '−' : ''}${whole}${fraction ? `.${fraction}` : ''}${percent ? '%' : ''}`;
}

export type ValueFormat =
  | 'integer'
  | 'fixed5Percent'
  | 'fixed5Amount'
  | 'fixed5Width'
  | 'clockSeconds'
  | 'decimal1'
  | 'percent2'
  | { readonly decimals: number; readonly percent?: boolean; readonly unit?: string; readonly signed?: boolean; readonly maxDecimals?: number };

export interface FormatValueOptions {
  readonly figure?: string;
  readonly gameWindow?: boolean;
  readonly compact?: boolean;
}

export type FormattedValue =
  | { readonly kind: 'known'; readonly text: string; readonly exact?: string; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'zero'; readonly text: string; readonly exact?: string; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'notRecorded'; readonly label: string; readonly reason: string; readonly gameWindow?: boolean; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'notApplicable'; readonly label: '\u2014'; readonly reason: string; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'partial'; readonly text: string; readonly exact?: string; readonly reason: string; readonly atLeast?: boolean; readonly spoken: string; readonly audit: string };

export const NOT_RECORDED_LABEL = 'Not recorded';
export const NOT_APPLICABLE_LABEL = '\u2014';
export const GAME_WINDOW_MISSING_LABEL = 'n/a';
export const PARTIAL_LABEL = 'partial';

export function notRecordedLabel(figure?: string): string {
  const clean = figure?.trim();
  return clean ? `${clean} not recorded` : NOT_RECORDED_LABEL;
}

function formatGroupedBigInt(raw: bigint, plus = false): string {
  if (raw === 0n) return '0';
  const negative = raw < 0n;
  const digits = (negative ? -raw : raw).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${negative ? '\u2212' : plus ? '+' : ''}${digits}`;
}

export function formatClockSeconds(seconds: number | bigint): string {
  if (typeof seconds === 'bigint' || (Number.isFinite(seconds) && Math.abs(seconds) >= 2 ** 53)) {
    const floored = typeof seconds === 'bigint' ? seconds : BigInt(Math.floor(seconds));
    const total = floored < 0n ? 0n : floored;
    const hours = total / 3600n;
    const minutes = (total / 60n) % 60n;
    const secs = total % 60n;
    return `${hours.toString().padStart(2, '0')}:${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
  }
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  return `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`;
}

export function formatFixed5Amount(value: number | bigint): string {
  const big = exactTruncBigInt(value);
  if (big === 0n) return '0';
  if (big < 0n) return '!';
  return formatNativeFixed5(big, 2);
}

export function formatScalar(raw: Scalar, format?: ValueFormat): string {
  if (typeof raw === 'string') return raw;
  if (!format || format === 'integer') {
    if (typeof raw === 'bigint' || (Number.isFinite(raw) && Math.abs(raw) >= 2 ** 53)) {
      return formatGroupedBigInt(exactTruncBigInt(raw));
    }
    const whole = Math.trunc(raw);
    if (whole === 0) return '0';
    const negative = whole < 0;
    const digits = groupDigits(String(Math.abs(whole)));
    return `${negative ? '\u2212' : ''}${digits}`;
  }
  if (format === 'clockSeconds') return formatClockSeconds(raw);
  if (format === 'decimal1') {
    return formatFixedDecimal(raw, 1, false, true, '\u2212');
  }
  if (format === 'percent2') {
    if (raw === 0 || raw === 0n) return '0%';
    return `${formatFixedDecimal(raw, 2, true, false, '-')}%`;
  }
  if (format === 'fixed5Percent') {
    if (raw === 0 || raw === 0n) return '0%';
    if (typeof raw === 'bigint' || Math.abs(raw) >= 2 ** 40) {
      return percentCell(exactTruncBigInt(raw));
    }
    const int = Math.trunc(raw);
    const pct = Math.min(100, Math.max(0, Math.trunc((int + (int < 0 ? -50000 : 50000)) / 100000)));
    return `${pct}%`;
  }
  if (format === 'fixed5Amount') return formatFixed5Amount(raw);
  if (format === 'fixed5Width') {
    const big = exactTruncBigInt(raw);
    if (big === 0n) return '0';
    return formatNativeFixed5(big, 1);
  }
  const suffix = format.unit ? ` ${format.unit}` : '';
  const sign = format.signed && raw > 0 ? '+' : '';
  if (raw === 0 || raw === 0n) {
    return `${format.percent ? '0%' : '0'}${suffix}`;
  }
  if (format.maxDecimals !== undefined && format.decimals === 0) {
    const formatted = formatFixedDecimal(raw, format.maxDecimals, false, true, '\u2212', format.signed === true);
    return `${formatted}${format.percent ? '%' : ''}${suffix}`;
  }
  return `${sign}${amountText(raw, format.decimals, format.percent)}${suffix}`;
}

export interface CompactScalar {
  readonly text: string;
  readonly exact: string;
  readonly compacted: boolean;
}

const COMPACT_UNITS = [
  { divisor: 1e12, suffix: 'T' },
  { divisor: 1e9, suffix: 'B' },
  { divisor: 1e6, suffix: 'M' },
  { divisor: 1e3, suffix: 'K' },
] as const;

export function formatCompactScalar(raw: Scalar, format?: ValueFormat): CompactScalar {
  const exact = formatScalar(raw, format);
  if (typeof raw === 'string') return { text: exact, exact, compacted: false };
  if (format === 'clockSeconds' || format === 'fixed5Percent' || format === 'percent2') {
    return { text: exact, exact, compacted: false };
  }
  if (typeof format === 'object' && format.percent) {
    return { text: exact, exact, compacted: false };
  }
  let scaled: number;
  if (format === 'fixed5Amount' || format === 'fixed5Width') {
    scaled = Number(raw) / 100000;
  } else if (typeof format === 'object' && format.decimals > 0) {
    scaled = Number(raw) / (10 ** format.decimals);
  } else {
    scaled = Number(raw);
  }
  if (!Number.isFinite(scaled) || Math.abs(scaled) < 1000) {
    return { text: exact, exact, compacted: false };
  }
  const negative = scaled < 0;
  const abs = Math.abs(scaled);
  const unit = COMPACT_UNITS.find(entry => abs >= entry.divisor);
  if (!unit) return { text: exact, exact, compacted: false };
  const quotient = abs / unit.divisor;
  const decimals = quotient >= 100 ? 0 : quotient >= 10 ? 1 : 2;
  const digits = quotient.toFixed(decimals).replace(/\.0+$/, '').replace(/(\.\d*[1-9])0+$/, '$1');
  const sign = negative ? '\u2212' : (typeof format === 'object' && format.signed && scaled > 0 ? '+' : '');
  const suffix = typeof format === 'object' && format.unit ? ` ${format.unit}` : '';
  return { text: `${sign}${digits}${unit.suffix}${suffix}`, exact, compacted: true };
}

export function formatValueState(value: Value, format?: ValueFormat, options?: FormatValueOptions): FormattedValue {
  assertValue(value, 'formatValueState');
  switch (value.kind) {
    case 'known': {
      if (options?.compact) {
        const compact = formatCompactScalar(value.value, format);
        return { kind: 'known', text: compact.text, ...(compact.compacted ? { exact: compact.exact } : {}), spoken: compact.exact, audit: compact.exact };
      }
      const text = formatScalar(value.value, format);
      return { kind: 'known', text, spoken: text, audit: text };
    }
    case 'zero': {
      const text = formatScalar(value.value, format);
      return { kind: 'zero', text, spoken: text, audit: text };
    }
    case 'notRecorded': {
      const label = options?.gameWindow ? GAME_WINDOW_MISSING_LABEL : notRecordedLabel(options?.figure);
      const lead = options?.figure?.trim() ? `${options.figure.trim()} not recorded` : NOT_RECORDED_LABEL;
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
      const compact = options?.compact ? formatCompactScalar(value.value, format) : null;
      const text = compact ? compact.text : formatScalar(value.value, format);
      const exact = compact ? compact.exact : text;
      const lead = value.atLeast ? `at least ${exact}` : exact;
      const spoken = `${lead}, partial: ${value.reason}`;
      return {
        kind: 'partial',
        text,
        ...(compact?.compacted ? { exact } : {}),
        reason: value.reason,
        ...(value.atLeast ? { atLeast: true } : {}),
        spoken,
        audit: spoken,
      };
    }
  }
}

export interface FormattedOutOf {
  readonly kind: FormattedValue['kind'];
  readonly text: string;
  readonly exact: string;
  readonly spoken: string;
  readonly missingReason?: string;
  readonly partialReason?: string;
  readonly gameWindow?: boolean;
}

export function formatOutOf(
  value: Value,
  max: Value,
  format?: ValueFormat,
  options?: FormatValueOptions,
): FormattedOutOf {
  const formattedValue = formatValueState(value, format, options);
  if (formattedValue.kind === 'notRecorded' || formattedValue.kind === 'notApplicable') {
    return {
      kind: formattedValue.kind,
      text: formattedValue.label,
      exact: formattedValue.label,
      spoken: formattedValue.spoken,
      missingReason: formattedValue.reason,
      ...(formattedValue.kind === 'notRecorded' && formattedValue.gameWindow ? { gameWindow: true } : {}),
    };
  }
  const formattedMax = formatValueState(max, format, options);
  const valueExact = formattedValue.exact ?? formattedValue.text;
  if (formattedMax.kind === 'notApplicable') {
    return {
      kind: formattedValue.kind,
      text: formattedValue.text,
      exact: valueExact,
      spoken: formattedValue.spoken,
      ...(formattedValue.kind === 'partial' ? { partialReason: formattedValue.reason } : {}),
    };
  }
  if (formattedMax.kind === 'notRecorded') {
    const reason = formattedValue.kind === 'partial'
      ? `${formattedValue.reason}; maximum not recorded: ${formattedMax.reason}`
      : `Maximum not recorded: ${formattedMax.reason}`;
    return {
      kind: 'partial',
      text: formattedValue.text,
      exact: valueExact,
      spoken: `${valueExact}, ${reason}`,
      partialReason: reason,
    };
  }
  const maxExact = formattedMax.exact ?? formattedMax.text;
  const text = `${formattedValue.text} / ${formattedMax.text}`;
  const exact = `${valueExact} / ${maxExact}`;
  const reasons: string[] = [];
  if (formattedValue.kind === 'partial') reasons.push(formattedValue.reason);
  if (formattedMax.kind === 'partial') reasons.push(formattedMax.reason);
  if (reasons.length) {
    const partialReason = reasons.join('; ');
    return { kind: 'partial', text, exact, spoken: `${exact}, partial: ${partialReason}`, partialReason };
  }
  return {
    kind: formattedValue.kind === 'zero' && formattedMax.kind === 'zero' ? 'zero' : 'known',
    text,
    exact,
    spoken: exact,
  };
}

export interface DateColumnParts {
  readonly dayMonth: string;
  readonly year: string;
  readonly full: string;
  readonly hover: string;
}

export type DateBoundKind = 'by' | 'since' | 'after' | 'before';

const DATE_BOUND_PREFIX: Readonly<Record<DateBoundKind, string>> = {
  by: 'By',
  since: 'Since',
  after: 'After',
  before: 'Before',
};

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

export interface FormattedBoundDate {
  readonly display: string;
  readonly hover: string;
  readonly exact: string;
  readonly reason: string;
}

export function formatBoundDate(hour: number, kind: DateBoundKind, reason: string): FormattedBoundDate {
  const whole = Math.floor(hour);
  const display = formatDateOnly(whole);
  const exact = formatDateHover(whole);
  return {
    display,
    hover: `${DATE_BOUND_PREFIX[kind]} ${exact}`,
    exact,
    reason,
  };
}

export function formatDateValue(
  hour: Value<number>,
  options?: { readonly figure?: string; readonly gameWindow?: boolean; readonly bound?: { readonly kind: DateBoundKind; readonly reason: string } },
): FormattedValue & { readonly hover?: string; readonly parts?: DateColumnParts | null } {
  assertValue(hour, 'formatDateValue');
  if (hour.kind === 'notRecorded') {
    return formatValueState(hour, undefined, { figure: options?.figure ?? 'Date', gameWindow: options?.gameWindow });
  }
  if (hour.kind === 'notApplicable') {
    return formatValueState(hour);
  }
  const whole = Math.floor(Number(hour.value));
  const display = formatDateOnly(whole);
  const parts = formatDateParts(whole);
  const bound = options?.bound ? formatBoundDate(whole, options.bound.kind, options.bound.reason) : null;
  const hover = bound ? bound.hover : formatDateHover(whole);
  if (hour.kind === 'partial') {
    const spoken = `${display}, partial: ${hour.reason}`;
    return { kind: 'partial', text: display, exact: hover, reason: hour.reason, spoken, audit: spoken, hover, parts };
  }
  return { kind: 'known', text: display, exact: hover, spoken: display, audit: display, hover, parts };
}

export function valueStateProps(value: Value, format?: ValueFormat, narrow = false): ValueStateProps {
  const formatted = formatValueState(value, format);
  switch (formatted.kind) {
    case 'known':
    case 'zero':
      return { kind: formatted.kind, text: formatted.text, ...(narrow ? { narrow: true } : {}) };
    case 'notRecorded':
      return { kind: 'missing', reason: formatted.reason, ...(narrow ? { narrow: true } : {}) };
    case 'notApplicable':
      return { kind: 'notApplicable', reason: formatted.reason, ...(narrow ? { narrow: true } : {}) };
    case 'partial':
      return {
        kind: 'partial',
        text: formatted.text,
        reason: formatted.reason,
        ...(formatted.atLeast ? { atLeast: true } : {}),
        ...(narrow ? { narrow: true } : {}),
      };
  }
}

export function spokenValue(value: Value, format?: ValueFormat): string {
  return formatValueState(value, format).spoken;
}

export function auditValue(value: Value, _format?: ValueFormat): string {
  assertValue(value, 'auditValue');
  return canonicalAuditValue(value);
}

export function defaultWidestSamples(format?: ValueFormat): readonly string[] {
  if (!format) return [];
  if (format === 'fixed5Percent') return ['100%'];
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
