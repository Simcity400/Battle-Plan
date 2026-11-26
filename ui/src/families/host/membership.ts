import { countryIdentityIndex, validCountryIdentityHistories } from '../../dashboard/countryIdentity.ts';
import { groupRuns, type ResolvedCountries } from '../../dashboard/groups.ts';
import type { CountrySelection } from '../../dashboard/model.ts';
import type { CountryGroupReply, MembershipRun } from '../../generated/dashboard';

export function withSoFarMembership(
  resolved: ResolvedCountries,
  selection: CountrySelection,
  groups: CountryGroupReply | undefined,
  lastHour: number,
): ResolvedCountries {
  if (resolved.membership || selection.kind !== 'group' || selection.membership !== 'ever' || !groups) return resolved;
  const runs = groupRuns(groups, selection.group);
  if (!runs) return resolved;
  const spans: MembershipRun[] = [...runs];
  if (validCountryIdentityHistories(groups.countryIdentities)) {
    const index = countryIdentityIndex(groups.countryIdentities);
    for (const [key, from] of index.firstHoursDuring(runs)) spans.push(...index.aliasRuns(new Set([key]), from, lastHour));
  }
  spans.sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag));
  const membership = new Map<string, MembershipRun[]>();
  for (const span of spans) {
    const own = membership.get(span.tag);
    const previous = own?.at(-1);
    if (!own) membership.set(span.tag, [{ ...span }]);
    else if (previous && span.firstHour <= previous.lastHour + 1) own[own.length - 1] = { ...previous, lastHour: Math.max(previous.lastHour, span.lastHour) };
    else own.push({ ...span });
  }
  return { ...resolved, tags: [...membership.keys()], membership };
}

