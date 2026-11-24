import type { CountrySelection } from '../dashboard/model';
import type { GroupSelection } from '../generated/dashboard';

export function groupSelection(countries: CountrySelection): GroupSelection | null {
  if (countries.kind !== 'group') return null;
  const { group, membership } = countries;
  if (group.type === 'humans') return { type: 'humans', membership };
  if (group.type === 'faction') return { type: 'faction', id: group.id, membership };
  if (group.type === 'war') return { type: 'war', tag: group.tag, membership };
  return null;
}
