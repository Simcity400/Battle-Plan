import type { EventBadge } from '../charts/badges';
import { badgeRows } from '../charts/badges.ts';
import type { CountryLabel } from './countryFlagsAt';

export function countryCapitulationRows(badges: readonly EventBadge[], labelAt: (hour: number) => CountryLabel) {
  const rows = badgeRows(badges);
  badges.forEach((badge, index) => {
    if (badge.capitulationTag === undefined) return;
    const label = labelAt(badge.hour)(badge.capitulationTag);
    rows[index] = { ...rows[index], label: `${badge.glyph} ${label.name} capitulated`, countryFlag: label };
  });
  return rows;
}
