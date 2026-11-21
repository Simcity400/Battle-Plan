import { nameReadMetadata, type NameRead } from '../../dashboard/nameReads.ts';
import { nameKey, type FAILED_FACT } from '../../dashboard/countryFacts.ts';
import { flagReadMetadata } from '../../dashboard/countryFlagsAt.ts';
import type { FlagRead } from '../../dashboard/countryFlags';
import type { LossRow } from './casualtiesBreakdownData';

export function casualtyCountryCell(row: Pick<LossRow, 'enemyTag' | 'enemyHour'>,
  names: ReadonlyMap<string, NameRead>, flags: ReadonlyMap<string, FlagRead | null | typeof FAILED_FACT | Error>) {
  const key = nameKey(row.enemyHour, row.enemyTag);
  return { ...nameReadMetadata(names.get(key)), ...flagReadMetadata(flags.get(key)) };
}

export function casualtyEnemyName(row: Pick<LossRow, 'enemyTag' | 'enemyHour'>, names: ReadonlyMap<string, NameRead>) {
  return nameReadMetadata(names.get(nameKey(row.enemyHour, row.enemyTag))).name;
}
