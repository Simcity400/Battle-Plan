import { entryAt } from './reads.ts';
import type { OptionField } from './options';
import type { StockVariantName } from '../generated/dashboard';

export const REFITS_OPTION: OptionField = {
  key: 'refits', section: 'options', label: 'Count refitted ships',
  hint: 'A refit rebuilds a ship the country already had as another design', kind: 'toggle', default: false,
};

export function variantNameAt(names: readonly StockVariantName[], hour: number, fallback: string): string {
  const entry = entryAt(names, hour);
  return (entry?.known ? entry.name : null) ?? fallback;
}

export const unresolvedTypeName = (_key: string) => 'Equipment type name unavailable';
