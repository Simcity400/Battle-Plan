import type { CountrySelection } from '../dashboard/model';
import type { GroupSelection } from '../generated/dashboard';

const END_MEMBERS = 'Members at the end of the recording are not known at this date. Choose the members at this date, or every member so far.';

export function groupSelection(countries: CountrySelection): GroupSelection | null {
  if (countries.kind !== 'group') return null;
  const membership = countries.membership === 'ever' ? 'ever' : 'hour';
  const group = countries.group;
  if (group.type === 'humans') return { type: 'humans', membership };
  if (group.type === 'faction') return { type: 'faction', id: group.id, membership };
  if (group.type === 'war') return { type: 'war', tag: group.tag, membership };
  return null;
}

export function groupUnavailable(countries: CountrySelection): string | null {
  return countries.kind === 'group' && countries.group.type !== 'top' && countries.membership === 'end' ? END_MEMBERS : null;
}
