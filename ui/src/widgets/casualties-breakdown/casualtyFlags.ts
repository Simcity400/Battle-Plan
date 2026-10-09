import { nameKey, type FAILED_FACT } from '../../dashboard/countryFacts.ts';
import { flagReadMetadata, type FlagRead } from '../../dashboard/countryFlagsAt.ts';
import type { LossRow } from './casualtiesBreakdownData';

/** An ended enemy keeps its supplying holder/hour for names and structured flags. */
export function casualtyCountryCell(row: Pick<LossRow, 'enemyTag' | 'enemyHour'>,
  names: ReadonlyMap<string, string>, flags: ReadonlyMap<string, FlagRead | null | typeof FAILED_FACT>) {
  const key = nameKey(row.enemyHour, row.enemyTag);
  return { name: names.get(key) ?? row.enemyTag, ...flagReadMetadata(flags.get(key)) };
}
