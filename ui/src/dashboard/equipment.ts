/** Names every equipment widget shows the same way (Equipment stockpile, Equipment
 * produced): a variant as recorded at an hour, an equipment type content does not
 * resolve, and the arms. The products resolve types through the recorded content
 * stack (`crates/dashboard/src/products/equipment.rs`); nothing here is vanilla. */
import { entryAt } from './reads.ts';
import type { Domain, StockVariantName } from '../generated/dashboard';

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
 * by content, or no definition recorded), such as its equipment token or handle. */
export function variantNameAt(names: readonly StockVariantName[], hour: number, fallback: string): string {
  const entry = entryAt(names, hour);
  return (entry?.known ? entry.name : null) || fallback;
}

/** The name of a type key content does not resolve: its recorded token (keys of
 * unresolved types start with `UNRESOLVED_MARK`, so they never collide with an
 * archetype; the products warn about a content key that does). */
export const unresolvedTypeName = (key: string) => key.startsWith(UNRESOLVED_MARK) ? key.slice(UNRESOLVED_MARK.length) : key;
