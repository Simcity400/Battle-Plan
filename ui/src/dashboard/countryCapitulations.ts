import type { CountryGroupReply, CountryIdentityHistory } from '../generated/dashboard';
import { countryFromKey } from './countryKey.ts';
import type { EventBadge } from '../charts/badges';
import { badgeRows } from '../charts/badges.ts';
import { runAt } from '../charts/series.ts';
import type { Span } from '../charts/series';
import type { CountryLabel } from './countryFlagsAt';

export function provisionalGroupEvent(groups: unknown, hour: number): boolean {
  const spans = (groups as { eventsApproximate?: readonly Span[] } | undefined)?.eventsApproximate;
  return !!spans?.some(span => span.firstHour <= hour && hour <= span.lastHour);
}

export function countryCapitulationBadges(groups: Pick<CountryGroupReply, 'capitulations'> | undefined,
  countries: readonly { tag: string; identities?: readonly Span[]; segments: readonly Span[]; membership?: readonly Span[] | null }[],
  extent: readonly [number, number], histories: readonly CountryIdentityHistory[] = []): ReadonlyMap<string, readonly EventBadge[]> {
  const byTag = new Map(countries.map(country => [country.tag, country]));
  const bySlot = new Map(countries.flatMap(country => { const slot = countryFromKey(country.tag); return typeof slot === 'number' ? [[slot, country] as const] : []; }));
  const runs = bySlot.size ? new Map(histories.map(history => [history.tag, history.identities])) : null;
  const badges = new Map<string, EventBadge[]>();
  for (const event of groups?.capitulations ?? []) {
    if (event.hour < extent[0] || event.hour > extent[1]) continue;
    const held = runs && runAt(runs.get(event.tag) ?? [], event.hour)?.slot;
    const country = byTag.get(event.tag) ?? (held == null ? undefined : bySlot.get(held));
    if (!country || !runAt(country.identities ?? country.segments, event.hour)
      || country.membership && !runAt(country.membership, event.hour)) continue;
    const rows = badges.get(country.tag) ?? [];
    if (!badges.has(country.tag)) badges.set(country.tag, rows);
    rows.push({ hour: event.hour, glyph: '\u00d7', label: 'Country capitulated', capitulationTag: event.tag,
      ...(provisionalGroupEvent(groups, event.hour) ? { approximate: true } : {}) });
  }
  return badges;
}

export function countryCapitulationRows(badges: readonly EventBadge[], labelAt: (hour: number) => CountryLabel) {
  const rows = badgeRows(badges);
  badges.forEach((badge, index) => {
    if (badge.capitulationTag === undefined) return;
    const label = labelAt(badge.hour)(badge.capitulationTag);
    rows[index] = { ...rows[index], label: `${badge.glyph} ${label.name} capitulated`, countryFlag: label };
  });
  return rows;
}
