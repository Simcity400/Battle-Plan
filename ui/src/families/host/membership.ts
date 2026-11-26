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
  const joined = new Map<string, number>();
  for (const run of runs) joined.set(run.tag, Math.min(joined.get(run.tag) ?? Infinity, run.firstHour));
  const membership = new Map<string, MembershipRun[]>();
  for (const tag of resolved.tags) {
    const firstHour = joined.get(tag);
    if (firstHour !== undefined) membership.set(tag, [{ tag, firstHour, lastHour }]);
  }
  return { ...resolved, membership };
}
