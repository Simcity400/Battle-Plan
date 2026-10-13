import { formatAmount } from '../charts/valueAxis.ts';
import type { OptionField, OptionValue } from './options';

/** Spent factory IC is written in whole numbers unless a widget's settings ask for
 * decimals, up to every recorded digit. Only the writing rounds: totals, shares,
 * slices and order use the exact recorded amounts. */
export const IC_DECIMALS_OPTION: OptionField = { key: 'icDecimals', section: 'options', label: 'IC decimals',
  hint: 'Decimal places in IC amounts.', kind: 'number', min: 0, max: 5, step: 1, default: 0 };

/** The decimals chosen (`resolveOptions`), at most the `scale` the amounts are recorded with. */
export function icDecimals(options: Readonly<Record<string, OptionValue>>, scale: number): number {
  const chosen = options[IC_DECIMALS_OPTION.key];
  return typeof chosen === 'number' && Number.isInteger(chosen) ? Math.max(0, Math.min(scale, chosen)) : 0;
}

/** A recorded IC amount of `scale` decimals written to `digits`, from the exact
 * integer when there is one (a number loses digits past 2^53), else the number. */
export const formatIc = (value: number, exact: bigint | null, scale: number, digits: number) =>
  formatAmount(exact ?? BigInt(Math.round(value)), scale, digits);
