import type { CountryGroupReply } from '../generated/dashboard';
import type { EventBadge } from '../charts/badges';
import { badgeRows } from '../charts/badges.ts';
import { runAt } from '../charts/series.ts';
import type { Span } from '../charts/series';
import type { CountryLabel } from './countryFlagsAt';

/** Callers supply the drawn native-holder histories, including selected aliases.
 * An absent legacy identity field uses proven coverage; an empty field proves none. */
export function countryCapitulationBadges(groups: Pick<CountryGroupReply, 'capitulations'> | undefined,
  countries: readonly { tag: string; identities?: readonly Span[]; segments: readonly Span[]; membership?: readonly Span[] | null }[],
  extent: readonly [number, number]): ReadonlyMap<string, readonly EventBadge[]> {
  const byTag = new Map(countries.map(country => [country.tag, country]));
  const badges = new Map<string, EventBadge[]>();
  for (const event of groups?.capitulations ?? []) {
    if (event.hour < extent[0] || event.hour > extent[1]) continue;
    const country = byTag.get(event.tag);
    if (!country || !runAt(country.identities ?? country.segments, event.hour)
      || country.membership && !runAt(country.membership, event.hour)) continue;
    const rows = badges.get(event.tag) ?? [];
    if (!badges.has(event.tag)) badges.set(event.tag, rows);
    rows.push({ hour: event.hour, glyph: '\u00d7', label: `${event.tag} capitulated`, capitulationTag: event.tag });
  }
  return badges;
}

/** Only reached events use dated names/flags; generic badges retain their labels. */
export function countryCapitulationRows(badges: readonly EventBadge[], labelAt: (hour: number) => CountryLabel) {
  const rows = badgeRows(badges);
  badges.forEach((badge, index) => {
    if (badge.capitulationTag === undefined) return;
    const label = labelAt(badge.hour)(badge.capitulationTag);
    rows[index] = { ...rows[index], label: `${badge.glyph} ${label.name} capitulated`, countryFlag: label };
  });
  return rows;
}
