import { formatAmount, formatFixed } from '../charts/valueAxis.ts';
import type { OptionField, OptionValue } from './options';

export const POWER_DECIMALS_OPTION: OptionField = { key: 'decimals', section: 'options', label: 'Decimals',
  hint: 'Decimal places in political and command power amounts.', kind: 'number', min: 0, max: 5, step: 1, default: 0 };

export const RATE_DECIMALS = 2;

export function powerDecimals(options: Readonly<Record<string, OptionValue>>, scale = 5): number {
  const chosen = options[POWER_DECIMALS_OPTION.key];
  return typeof chosen === 'number' && Number.isInteger(chosen) ? Math.max(0, Math.min(scale, chosen)) : 0;
}

export const formatPower = (raw: bigint, digits: number, scale = 5) => formatAmount(raw, scale, digits);

export const formatPowerChange = (raw: bigint, digits: number, scale = 5) => {
  const text = formatAmount(raw, scale, digits);
  return raw > 0n && text !== '0' ? `+${text}` : text;
};

export function formatRate(raw: bigint, digits: number, scale = 5): string {
  const text = formatFixed(raw, scale, Math.min(scale, Math.max(RATE_DECIMALS, digits)));
  return raw > 0n && /[1-9]/.test(text) ? `+${text}` : text;
}
