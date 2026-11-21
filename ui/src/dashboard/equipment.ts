import { entryAt } from './reads.ts';
import type { OptionField } from './options';
import type { Domain, StockVariantName } from '../generated/dashboard';

export const REFITS_OPTION: OptionField = {
  key: 'refits', section: 'options', label: 'Count refitted ships',
  hint: 'A refit rebuilds a ship the country already had as another design', kind: 'toggle', default: false,
};

export const DOMAINS: readonly Domain[] = ['land', 'air', 'naval'];

export const DOMAIN_NAMES: Record<Domain, string> = { land: 'Land', air: 'Air', naval: 'Naval' };

export const UNRESOLVED_MARK = ':';
export const UNRESOLVED_ARM = 'unresolved';

export const UNRESOLVED_DOMAIN = 'Type not resolved';

export const domainName = (key: string) => (DOMAINS as readonly string[]).includes(key) ? DOMAIN_NAMES[key as Domain] : UNRESOLVED_DOMAIN;

export function variantNameAt(names: readonly StockVariantName[], hour: number, fallback: string): string {
  const entry = entryAt(names, hour);
  return (entry?.known ? entry.name : null) ?? fallback;
}

export const unresolvedTypeName = (_key: string) => 'Equipment type name unavailable';
