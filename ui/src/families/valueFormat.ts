import { formatNativeFixed5 } from '../charts/valueAxis.ts';
import { assertValue, type Scalar, type Value } from './value.ts';

function percentCell(raw: string | bigint): string {
  const value = BigInt(raw);
  const rounded = BigInt.asIntN(32, (value + (value < 0n ? -50000n : 50000n)) / 100000n);
  const clamped = rounded < 0n ? 0n : rounded > 100n ? 100n : rounded;
  return `${formatNativeFixed5(clamped * 100000n, 0)}%`;
}

function amountText(value: number | bigint, decimals: number, percent = false): string {
  let raw: number | bigint = typeof value === 'number' ? Math.trunc(value) : value;
  if (percent) raw = typeof raw === 'number' && Math.abs(raw) < 2 ** 46 ? raw * 100 : BigInt(raw) * 100n;
  const negative = raw < 0;
  const digits = String(negative ? -raw : raw).padStart(decimals + 1, '0');
  const whole = digits.slice(0, digits.length - decimals).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
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

export type FormattedValue =
  | { readonly kind: 'known'; readonly text: string; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'zero'; readonly text: string; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'notRecorded'; readonly label: 'Not recorded'; readonly reason: string; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'notApplicable'; readonly label: '\u2014'; readonly reason: string; readonly spoken: string; readonly audit: string }
  | { readonly kind: 'partial'; readonly text: string; readonly reason: string; readonly atLeast?: boolean; readonly spoken: string; readonly audit: string };

export const NOT_RECORDED_LABEL = 'Not recorded';
export const NOT_APPLICABLE_LABEL = '\u2014';
export const PARTIAL_LABEL = 'partial';

export function formatClockSeconds(seconds: number | bigint): string {
  const total = Math.max(0, Math.floor(Number(seconds)));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  return `${String(hours).padStart(2, '0')}:${String(minutes).padStart(2, '0')}:${String(total % 60).padStart(2, '0')}`;
}

export function formatFixed5Amount(value: number | bigint): string {
  const big = typeof value === 'bigint' ? value : BigInt(Math.trunc(value));
  if (big === 0n) return '0';
  if (big < 0n) return '!';
  return formatNativeFixed5(big, 2);
}

export function formatScalar(raw: Scalar, format?: ValueFormat): string {
  if (typeof raw === 'string') return raw;
  if (!format || format === 'integer') {
    if (typeof raw === 'bigint') {
      const negative = raw < 0n;
      const digits = String(negative ? -raw : raw).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
      return `${negative ? '\u2212' : ''}${digits}`;
    }
    const whole = Math.trunc(raw);
    if (whole === 0) return '0';
    const negative = whole < 0;
    const digits = String(Math.abs(whole)).replace(/\B(?=(\d{3})+(?!\d))/g, ',');
    return `${negative ? '\u2212' : ''}${digits}`;
  }
  if (format === 'clockSeconds') return formatClockSeconds(raw);
  if (format === 'decimal1') {
    const num = Number(raw);
    if (num === 0) return '0';
    const negative = num < 0;
    const text = Math.abs(num).toLocaleString('en-US', { maximumFractionDigits: 1 });
    return `${negative ? '\u2212' : ''}${text}`;
  }
  if (format === 'percent2') {
    const num = Number(raw);
    if (num === 0) return '0%';
    return `${num.toFixed(2)}%`;
  }
  if (format === 'fixed5Percent') {
    if (raw === 0 || raw === 0n) return '0%';
    if (typeof raw === 'bigint' || Math.abs(raw) >= 2 ** 40) {
      return percentCell(typeof raw === 'bigint' ? raw : BigInt(Math.trunc(raw)));
    }
    const int = Math.trunc(raw);
    const pct = Math.min(100, Math.max(0, Math.trunc((int + (int < 0 ? -50000 : 50000)) / 100000)));
    return `${pct}%`;
  }
  if (format === 'fixed5Amount') return formatFixed5Amount(raw);
  if (format === 'fixed5Width') {
    const big = typeof raw === 'bigint' ? raw : BigInt(Math.trunc(raw));
    if (big === 0n) return '0';
    return formatNativeFixed5(big, 1);
  }
  const suffix = format.unit ? ` ${format.unit}` : '';
  const sign = format.signed && raw > 0 ? '+' : '';
  if (raw === 0 || raw === 0n) {
    return `${format.percent ? '0%' : '0'}${suffix}`;
  }
  if (format.maxDecimals !== undefined && format.decimals === 0) {
    const num = Number(raw);
    const negative = num < 0;
    const text = Math.abs(num).toLocaleString('en-US', { maximumFractionDigits: format.maxDecimals });
    return `${negative ? '\u2212' : sign}${text}${format.percent ? '%' : ''}${suffix}`;
  }
  return `${sign}${amountText(raw, format.decimals, format.percent)}${suffix}`;
}

export function formatValueState(value: Value, format?: ValueFormat): FormattedValue {
  assertValue(value, 'formatValueState');
  switch (value.kind) {
    case 'known': {
      const text = formatScalar(value.value, format);
      return { kind: 'known', text, spoken: text, audit: text };
    }
    case 'zero': {
      const text = formatScalar(value.value, format);
      return { kind: 'zero', text, spoken: text, audit: text };
    }
    case 'notRecorded': {
      const spoken = `Not recorded: ${value.reason}`;
      return { kind: 'notRecorded', label: NOT_RECORDED_LABEL, reason: value.reason, spoken, audit: spoken };
    }
    case 'notApplicable': {
      const spoken = `Not applicable: ${value.reason}`;
      return { kind: 'notApplicable', label: NOT_APPLICABLE_LABEL, reason: value.reason, spoken, audit: spoken };
    }
    case 'partial': {
      const text = formatScalar(value.value, format);
      const lead = value.atLeast ? `at least ${text}` : text;
      const spoken = `${lead}, partial: ${value.reason}`;
      return { kind: 'partial', text, reason: value.reason, ...(value.atLeast ? { atLeast: true } : {}), spoken, audit: spoken };
    }
  }
}

export function spokenValue(value: Value, format?: ValueFormat): string {
  return formatValueState(value, format).spoken;
}

export function auditValue(value: Value, format?: ValueFormat): string {
  return formatValueState(value, format).audit;
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
