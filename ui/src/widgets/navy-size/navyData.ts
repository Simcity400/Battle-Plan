import type { OptionValue } from '../../dashboard/options';

export function navyTotals(options: Readonly<Record<string, OptionValue>>) {
  const owner = options.ownership === 'logical' ? 'logical' : 'roster';
  return options.measure === 'strength' ? { kind: 'navy-totals', measure: `${owner}Strength`, decimals: 5 }
    : { kind: 'navy-totals', measure: `${owner}Ships`, decimals: 0 };
}
