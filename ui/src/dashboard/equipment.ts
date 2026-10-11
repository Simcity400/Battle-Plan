/** Names every equipment widget shows the same way (Equipment stockpile, Equipment
 * produced): a variant as recorded at an hour, an equipment type content does not
 * resolve, and the arms. The products resolve types through the recorded content
 * stack (`crates/dashboard/src/products/equipment.rs`); nothing here is vanilla. */
import { entryAt } from './reads.ts';
import type { Domain, StockVariantName } from '../generated/dashboard';
import type { OptionField } from './options';

/** Every production widget uses the same refit selection and default. */
export const REFITS_OPTION: OptionField = {
  key: 'refits', section: 'options', label: 'Count refitted ships',
  hint: 'A refit rebuilds a ship the country already had as another design', kind: 'toggle', default: false,
};

/** The arms, in the game's order. */
export const DOMAINS: readonly Domain[] = ['land', 'air', 'naval'];

export const DOMAIN_NAMES: Record<Domain, string> = { land: 'Land', air: 'Air', naval: 'Naval' };

/** Starts the key of a type content does not resolve, before its recorded token (the
 * products' `UNRESOLVED_MARK`): a mark a setting can store (`storableToken`), so such a
 * type can be included and chosen like any other. */
export const UNRESOLVED_MARK = ':';
/** The arm of the types content does not resolve, whose arm cannot be told. */
export const UNRESOLVED_ARM = 'unresolved';

/** The group of types whose arm content does not resolve. */
export const UNRESOLVED_DOMAIN = 'Type not resolved';

/** An arm's name; `UNRESOLVED_ARM` (or any other key) is the group of unresolved types. */
export const domainName = (key: string) => (DOMAINS as readonly string[]).includes(key) ? DOMAIN_NAMES[key as Domain] : UNRESOLVED_DOMAIN;

/** A variant's name as recorded at an hour: the latest read at or before it (the kit's
 * `entryAt`); before the first there is none, so no later name is backfilled, and
 * where its names stopped being known (a gap, a due read that did not land) none
 * holds either. Its name, else `fallback` (not named yet, not known then, not named
 * by content, or no definition recorded), such as a descriptive variant handle. */
export function variantNameAt(names: readonly StockVariantName[], hour: number, fallback: string): string {
  const entry = entryAt(names, hour);
  return (entry?.known ? entry.name : null) ?? fallback;
}

/** An unresolved type keeps its key for identity and diagnostics, with an
 * explicit unavailable title in the interface. */
export const unresolvedTypeName = (_key: string) => 'Equipment type name unavailable';
