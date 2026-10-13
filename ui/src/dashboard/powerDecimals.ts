import { formatAmount, formatFixed } from '../charts/valueAxis.ts';
import type { OptionField, OptionValue } from './options';

/** Political and command power amounts (balances, maxima, payments, totals) are
 * written in whole numbers unless a widget's settings ask for decimals, up to
 * every recorded digit. Daily rates keep the two decimals the game shows. Only the
 * writing rounds: totals, shares, order and geometry use the exact amounts. */
export const POWER_DECIMALS_OPTION: OptionField = { key: 'decimals', section: 'options', label: 'Decimals',
  hint: 'Decimal places in political and command power amounts.', kind: 'number', min: 0, max: 5, step: 1, default: 0 };

/** Rates show at least this many decimals, as the game's daily figures do. */
export const RATE_DECIMALS = 2;

/** The decimals chosen (`resolveOptions`), at most the `scale` amounts are recorded with. */
export function powerDecimals(options: Readonly<Record<string, OptionValue>>, scale = 5): number {
  const chosen = options[POWER_DECIMALS_OPTION.key];
  return typeof chosen === 'number' && Number.isInteger(chosen) ? Math.max(0, Math.min(scale, chosen)) : 0;
}

/** An amount of `scale` decimals written to `digits`. */
export const formatPower = (raw: bigint, digits: number, scale = 5) => formatAmount(raw, scale, digits);

/** A spending amount, written as the decrease it was (`−50`). */
export const formatSpent = (raw: bigint, digits: number, scale = 5) => {
  const text = formatAmount(raw, scale, digits);
  return text === '0' ? text : `−${text}`;
};

/** A signed change in whole amounts (`+20`, `−20`). */
export const formatPowerChange = (raw: bigint, digits: number, scale = 5) => {
  const text = formatAmount(raw, scale, digits);
  return raw > 0n && text !== '0' ? `+${text}` : text;
};

/** A signed daily rate with the game's two decimals, or more when chosen (`+0.83`, `−1.17`). */
export function formatRate(raw: bigint, digits: number, scale = 5): string {
  const text = formatFixed(raw, scale, Math.min(scale, Math.max(RATE_DECIMALS, digits)));
  return raw > 0n && /[1-9]/.test(text) ? `+${text}` : text;
}
