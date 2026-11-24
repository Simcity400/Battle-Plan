import type { OptionValue } from '../../dashboard/options';

export function airForceTotals(options: Readonly<Record<string, OptionValue>>) {
  const owner = options.ownership === 'effective' ? 'effective' : 'roster';
  return { kind: 'air-force-totals', measure: `${owner}${options.measure === 'wings' ? 'Wings' : 'Planes'}`, decimals: 0 };
}
