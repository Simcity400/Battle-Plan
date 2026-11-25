import type { OptionField, OptionValue } from './options';

export const IC_DECIMALS_OPTION: OptionField = { key: 'icDecimals', section: 'options', label: 'IC decimals',
  hint: 'Decimal places in IC amounts.', kind: 'number', min: 0, max: 5, step: 1, default: 0 };

export function icDecimals(options: Readonly<Record<string, OptionValue>>, scale: number): number {
  const chosen = options[IC_DECIMALS_OPTION.key];
  return typeof chosen === 'number' && Number.isInteger(chosen) ? Math.max(0, Math.min(scale, chosen)) : 0;
}
