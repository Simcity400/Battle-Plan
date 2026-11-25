
export function niceStep(span: number, count = 5, whole = false): number {
  if (!(span > 0) || !Number.isFinite(span)) return 1;
  const raw = span / Math.max(1, count);
  const power = 10 ** Math.floor(Math.log10(raw));
  const fraction = raw / power;
  const step = (fraction <= 1 ? 1 : fraction <= 2 ? 2 : fraction <= 2.5 ? 2.5 : fraction <= 5 ? 5 : 10) * power;
  if (!whole) return step;
  if (step < 1) return 1;
  return Number.isInteger(step) ? step : 5 * power;
}

export function linearTicks(min: number, max: number, count = 5, whole = false): number[] {
  if (!(max > min)) return Number.isFinite(min) ? (whole ? [Math.round(min)] : [min]) : [];
  const step = niceStep(max - min, count, whole);
  const start = Math.ceil(min / step - 1e-9);
  if (!Number.isSafeInteger(start) || !Number.isSafeInteger(Math.floor(max / step) + 1)) return [min, max];
  const decimals = whole ? 0 : Math.min(100, exactDecimal(step).scale);
  const ticks: number[] = [];
  for (let index = start; index * step <= max + step * 1e-9 && ticks.length < 1000; index++) ticks.push(Number((index * step).toFixed(decimals)));
  return ticks;
}

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

export function formatNativeFixed5(raw: string | bigint, digits = 2): string {
  if (!Number.isInteger(digits) || digits < 0 || digits > 5) throw new RangeError('Native fixed5 precision must be 0–5.');
  const value = typeof raw === 'bigint' ? raw : BigInt(raw);
  const magnitude = value < 0n ? -value : value;
  const whole = BigInt.asIntN(32, magnitude / 100000n).toString();
  const fraction = digits ? `.${(magnitude % 100000n).toString().padStart(5, '0').slice(0, digits)}` : '';
  return `${value < 0n ? '-' : ''}${whole}${fraction}`;
}

export function fixedNumber(raw: string | bigint, scale: number): number {
  return Number(typeof raw === 'bigint' ? raw : BigInt(raw)) / 10 ** scale;
}

export function rescale(raw: string | bigint, denominator: bigint | number, decimals: number): bigint {
  const value = (typeof raw === 'bigint' ? raw : BigInt(raw)) * 10n ** BigInt(decimals);
  const whole = BigInt(denominator);
  const half = whole / 2n;
  return value < 0n ? -((-value + half) / whole) : (value + half) / whole;
}

export function fractionSum(values: Iterable<readonly [bigint, bigint | number]>): readonly [bigint, bigint] {
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
    const divisor = gcd(sum, denominator);
    sum /= divisor;
    denominator /= divisor;
  }
  return [sum, denominator];
}

export function formatAmount(raw: string | bigint, scale: number, digits = 2): string {
  const text = formatFixed(raw, scale, digits);
  return text.includes('.') ? text.replace(/\.?0+$/, '') : text;
}

export function formatRate(raw: string | bigint, scale: number): string {
  const value = typeof raw === 'bigint' ? raw : BigInt(raw);
  const magnitude = value < 0n ? -value : value;
  const unit = 10n ** BigInt(scale);
  if (magnitude === 0n || magnitude >= 10n * unit) return formatAmount(value, scale, 0);
  let digits = 1;
  for (let bound = unit; magnitude < bound && digits < scale; bound /= 10n) digits++;
  return formatAmount(value, scale, digits);
}

export function formatSigned(raw: string | bigint, scale: number, digits = 2): string {
  const value = typeof raw === 'bigint' ? raw : BigInt(raw);
  const text = formatAmount(value, scale, digits);
  return value > 0n && text !== '0' ? `+${text}` : text;
}

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

export function formatAxisValue(value: number, ticks: readonly number[] = [value]): string {
  const all = ticks.length ? ticks : [value];
  let digits = 15;
  let decimals = all.map(tick => exactDecimal(tick, digits));
  const key = (decimal: { units: bigint; scale: number }) => `${decimal.units}e-${decimal.scale}`;
  if (new Set(decimals.map(key)).size < new Set(all).size) { digits = 17; decimals = all.map(tick => exactDecimal(tick, digits)); }
  const own = exactDecimal(value, digits);
  const places = Math.min(20, Math.max(0, ...decimals.map(decimal => decimal.scale)));
  const kPlaces = Math.max(0, ...decimals.map(decimal => trimDecimal(decimal.units, decimal.scale + 3).scale));
  if (Math.abs(value) >= 1000 && kPlaces <= 1) return `${formatFixed(own.units, own.scale + 3, kPlaces)}k`;
  return formatFixed(own.units, own.scale, places);
}

export function formatAxisPercent(value: number, ticks: readonly number[] = [value]): string {
  return `${formatAxisValue(value, ticks)}%`;
}
