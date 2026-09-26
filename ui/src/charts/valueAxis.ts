/** Value-axis ticks and exact number formatting. */

/** A 1, 2 or 5 × 10ⁿ step giving about `count` ticks over `span`. */
export function niceStep(span: number, count = 5): number {
  if (!(span > 0) || !Number.isFinite(span)) return 1;
  const raw = span / Math.max(1, count);
  const power = 10 ** Math.floor(Math.log10(raw));
  const fraction = raw / power;
  return (fraction <= 1 ? 1 : fraction <= 2 ? 2 : fraction <= 5 ? 5 : 10) * power;
}

/** Ticks on multiples of a nice step inside [min, max]. */
export function linearTicks(min: number, max: number, count = 5): number[] {
  if (!(max > min)) return Number.isFinite(min) ? [min] : [];
  const step = niceStep(max - min, count);
  const start = Math.ceil(min / step - 1e-9);
  // Past 2^53 an index no longer advances by one, so such a narrow range just shows its ends.
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(Math.floor(max / step) + 1)) return [min, max];
  // Rounding to the step's decimals removes float noise (0.6000000000000001)
  // without merging ticks on large values, as significant-digit rounding would.
  const decimals = Math.min(100, Math.max(0, -Math.floor(Math.log10(step))));
  const ticks: number[] = [];
  for (let index = start; index * step <= max + step * 1e-9 && ticks.length < 1000; index++) ticks.push(Number((index * step).toFixed(decimals)));
  return ticks;
}

/** 1, 2 and 5 × 10ⁿ inside a positive range; empty when the range isn't positive. */
export function logTicks(min: number, max: number): number[] {
  if (!(min > 0) || !(max > min)) return [];
  const ticks: number[] = [];
  for (let exponent = Math.floor(Math.log10(min)); exponent <= Math.ceil(Math.log10(max)); exponent++) {
    for (const mantissa of [1, 2, 5]) {
      const tick = Number((mantissa * 10 ** exponent).toPrecision(12));
      if (tick >= min && tick <= max) ticks.push(tick);
    }
  }
  return ticks;
}

/** Formats a recorded fixed-point integer exactly: `raw` is the stored integer and
 * `scale` its number of decimal places. Rounds half away from zero to `digits`. */
export function formatFixed(raw: string | bigint, scale: number, digits: number): string {
  let value = typeof raw === 'bigint' ? raw : BigInt(raw);
  const negative = value < 0n;
  if (negative) value = -value;
  if (digits < scale) {
    const divisor = 10n ** BigInt(scale - digits);
    value = (value + divisor / 2n) / divisor;
  } else value *= 10n ** BigInt(digits - scale);
  const text = value.toString().padStart(digits + 1, '0');
  const whole = text.slice(0, text.length - digits);
  const fraction = digits ? `.${text.slice(text.length - digits)}` : '';
  const grouped = whole.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${negative && /[1-9]/.test(text) ? '−' : ''}${grouped}${fraction}`;
}

/** A plotting value for a fixed-point integer; labels should use `formatFixed`. */
export function fixedNumber(raw: string | bigint, scale: number): number {
  return Number(typeof raw === 'bigint' ? raw : BigInt(raw)) / 10 ** scale;
}
