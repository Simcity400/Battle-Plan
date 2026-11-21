import { countryFromKey } from '../../dashboard/countryKey.ts';
import { validateForceReply } from '../../charts/race/forceRace.ts';
import type { ForceParams } from '../../charts/race/forceRace';
import type { OptionValue } from '../../dashboard/options';
import { formatAxisValue, formatFixed } from '../../charts/valueAxis.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { ForceSizeReply } from '../../generated/dashboard';

export function validateNavyReply(data: unknown): ForceSizeReply {
  return validateForceReply(data, true);
}

export const formatNavyValue = (value: number, options: Readonly<Record<string, OptionValue>>) =>
  options.measure === 'strength' ? formatFixed(BigInt(Math.round(value)), 5, 5).replace(/\.?0+$/, '') : usNumber.format(value);

export function navyChartOptions(options: Readonly<Record<string, OptionValue>>): Readonly<Record<string, OptionValue>> {
  if (options.measure !== 'strength') return options;
  return { ...options,
    yMin: typeof options.yMin === 'number' ? options.yMin * 100_000 : options.yMin,
    yMax: typeof options.yMax === 'number' ? options.yMax * 100_000 : options.yMax };
}

export const formatHullTick = (value: number, ticks: readonly number[]) =>
  formatAxisValue(value / 100_000, ticks.map(tick => tick / 100_000));

export function navyRequest(branchId: string, resolved: { tags: readonly string[]; top?: number },
  options: Readonly<Record<string, OptionValue>>): ForceParams {
  return { branchId, countries: resolved.top == null ? resolved.tags.map(countryFromKey) : [], top: resolved.top ?? null,
    ownership: options.ownership === 'logical' ? 'logical' : 'roster', grouping: options.grouping === 'type' ? 'type' : 'class',
    measure: options.measure === 'strength' ? 'strength' : 'ships' };
}

export function navyTotals(options: Readonly<Record<string, OptionValue>>) {
  const owner = options.ownership === 'logical' ? 'logical' : 'roster';
  return options.measure === 'strength' ? { kind: 'navy-totals', measure: `${owner}Strength`, decimals: 5 }
    : { kind: 'navy-totals', measure: `${owner}Ships`, decimals: 0 };
}
