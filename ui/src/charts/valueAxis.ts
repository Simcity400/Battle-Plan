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

/** Log ticks at least `minGap` pixels apart: all of them when they fit, else only
 * powers of ten, then every second, third… power of ten. */
export function thinLogTicks(ticks: readonly number[], toPixel: (value: number) => number, minGap: number): number[] {
  const spaced = (list: readonly number[]) => list.every((value, index) => index === 0 || Math.abs(toPixel(value) - toPixel(list[index - 1])) >= minGap);
  if (spaced(ticks)) return [...ticks];
  const decades = ticks.filter(value => value.toExponential(0).startsWith('1e'));
  for (let every = 1; every < decades.length; every++) {
    const kept = decades.filter((_, index) => index % every === 0);
    if (spaced(kept)) return kept;
  }
  return decades.slice(0, 1);
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

/** Fixed5 formatting with a fixed number of digits.
 * The whole part narrows to i32; fraction digits truncate,
 * keep trailing zeros and have no grouping. Sort/tooltips retain the exact raw. */
export function formatNativeFixed5(raw: string | bigint, digits = 2): string {
  if (!Number.isInteger(digits) || digits < 0 || digits > 5) throw new RangeError('Native fixed5 precision must be 0–5.');
  const value = typeof raw === 'bigint' ? raw : BigInt(raw);
  const magnitude = value < 0n ? -value : value;
  const whole = BigInt.asIntN(32, magnitude / 100000n).toString();
  const fraction = digits ? `.${(magnitude % 100000n).toString().padStart(5, '0').slice(0, digits)}` : '';
  return `${value < 0n ? '-' : ''}${whole}${fraction}`;
}

/** A plotting value for a fixed-point integer; labels should use `formatFixed`. */
export function fixedNumber(raw: string | bigint, scale: number): number {
  return Number(typeof raw === 'bigint' ? raw : BigInt(raw)) / 10 ** scale;
}

/** A recorded amount of `denominator` units to the whole (such as the engine's
 * 32768ths) as an integer of 10^-`decimals` units, rounded half away from zero, so
 * `formatAmount` and `formatSigned` can write it exactly. `raw` may be a sum or a
 * difference over any denominator, such as a net of two scales. */
export function rescale(raw: string | bigint, denominator: bigint | number, decimals: number): bigint {
  const value = (typeof raw === 'bigint' ? raw : BigInt(raw)) * 10n ** BigInt(decimals);
  const whole = BigInt(denominator);
  const half = whole / 2n;
  return value < 0n ? -((-value + half) / whole) : (value + half) / whole;
}

/** Sums exact fractions with positive denominators, then rounds once to decimal
 * units (`rescale`), such as averages over different recorded durations. */
export function rescaleSum(values: Iterable<readonly [bigint, bigint | number]>, decimals: number): bigint {
  const [raw, denominator] = fractionSum(values);
  return rescale(raw, denominator, decimals);
}

/** Exact reduced sum; presentation can round later without changing its amount. */
export function fractionSum(values: Iterable<readonly [bigint, bigint | number]>): readonly [bigint, bigint] {
  // Common durations need only one fraction, however many countries contributed.
  const totals = new Map<bigint, bigint>();
  for (const [raw, denominator] of values) {
    const units = BigInt(denominator);
    if (units <= 0n) throw new RangeError('A fraction’s denominator must be positive.');
    totals.set(units, (totals.get(units) ?? 0n) + raw);
  }
  const gcd = (first: bigint, second: bigint) => {
    let a = first < 0n ? -first : first;
    let b = second;
    while (b) [a, b] = [b, a % b];
    return a;
  };
  let sum = 0n;
  let denominator = 1n;
  for (const [units, raw] of totals) {
    if (!raw) continue;
    const common = gcd(denominator, units);
    sum = sum * (units / common) + raw * (denominator / common);
    denominator *= units / common;
    // Reduce after adding so the next sum keeps only the precision it needs.
    const divisor = gcd(sum, denominator);
    sum /= divisor;
    denominator /= divisor;
  }
  return [sum, denominator];
}

/** An amount as the game shows it, from a recorded fixed-point integer: at most
 * `digits` decimals (two by default), none when whole. */
export function formatAmount(raw: string | bigint, scale: number, digits = 2): string {
  const text = formatFixed(raw, scale, digits);
  return text.includes('.') ? text.replace(/\.?0+$/, '') : text;
}

/** An amount with its sign, for surpluses, deficits and changes: `+` when above
 * zero at the digits shown, `−` below. */
export function formatSigned(raw: string | bigint, scale: number, digits = 2): string {
  const value = typeof raw === 'bigint' ? raw : BigInt(raw);
  const text = formatAmount(value, scale, digits);
  return value > 0n && text !== '0' ? `+${text}` : text;
}

/** A number as an exact decimal, `units` / 10^`scale` with no trailing zeros. A
 * double is read at 15 significant digits, all it is faithful to, which drops only
 * binary noise (0.1 + 0.2 reads 0.3); `digits` 17 reads it exactly. */
export function exactDecimal(value: number, digits = 15): { units: bigint; scale: number } {
  if (!Number.isFinite(value) || value === 0) return { units: 0n, scale: 0 };
  const [mantissa, exponent = '0'] = value.toPrecision(digits).split('e');
  const [whole, fraction = ''] = mantissa.split('.');
  return trimDecimal(BigInt(whole + fraction), fraction.length - Number(exponent));
}

function trimDecimal(units: bigint, scale: number): { units: bigint; scale: number } {
  if (scale < 0) return { units: units * 10n ** BigInt(-scale), scale: 0 };
  while (scale > 0 && units % 10n === 0n) { units /= 10n; scale--; }
  return { units, scale };
}

/** A value-axis label, written exactly with the decimals its ticks need, so no two
 * ticks ever read the same. `ticks` is every tick drawn with it. Ticks sit on
 * multiples of their step, so the finest tick has the step's decimals; they are
 * counted on exact decimals, never on a rounded float, so 10,000,000.00002 and
 * 10,000,000.00004 stay apart. Thousands are shortened to k when every tick then
 * needs at most one decimal (1.5k); otherwise, as for 10,000.01 beside 10,000.02,
 * the whole number is written. It is the time chart's default. */
export function formatAxisValue(value: number, ticks: readonly number[] = [value]): string {
  const all = ticks.length ? ticks : [value];
  let digits = 15;
  let decimals = all.map(tick => exactDecimal(tick, digits));
  // Ticks this close together differ only past 15 digits: read them exactly.
  const key = (decimal: { units: bigint; scale: number }) => `${decimal.units}e-${decimal.scale}`;
  if (new Set(decimals.map(key)).size < new Set(all).size) { digits = 17; decimals = all.map(tick => exactDecimal(tick, digits)); }
  const own = exactDecimal(value, digits);
  const places = Math.min(20, Math.max(0, ...decimals.map(decimal => decimal.scale)));
  const kPlaces = Math.max(0, ...decimals.map(decimal => trimDecimal(decimal.units, decimal.scale + 3).scale));
  if (Math.abs(value) >= 1000 && kPlaces <= 1) return `${formatFixed(own.units, own.scale + 3, kPlaces)}k`;
  return formatFixed(own.units, own.scale, places);
}

/** A percentage value-axis label: `formatAxisValue` with a percent sign, so ticks
 * of a small range stay apart (0.1% beside 0.2%, never every tick "0%"). Pass
 * it as a time chart's `formatValue` for values already in per cent. */
export function formatAxisPercent(value: number, ticks: readonly number[] = [value]): string {
  return `${formatAxisValue(value, ticks)}%`;
}
