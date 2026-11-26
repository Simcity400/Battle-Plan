import { groupRuns, type ResolvedCountries } from '../../dashboard/groups.ts';
import type { CountrySelection } from '../../dashboard/model.ts';
import type { CountryGroupReply, MembershipRun } from '../../generated/dashboard';
import type { HolderRun } from '../holders.ts';

export interface SlotRuns {
  tagRuns(slot: number): readonly HolderRun[];
  runsForKey(key: string): readonly HolderRun[];
}

export function withSoFarMembership(
  resolved: ResolvedCountries,
  selection: CountrySelection,
  groups: CountryGroupReply | undefined,
  lastHour: number,
  slots: SlotRuns | undefined,
): ResolvedCountries {
  if (resolved.membership || selection.kind !== 'group' || selection.membership !== 'ever' || !groups || !slots) return resolved;
  const runs = groupRuns(groups, selection.group);
  if (!runs) return resolved;
  const joined = new Map<number, number>();
  const spans: MembershipRun[] = [];
  for (const run of runs) {
    const holder = slots.runsForKey(run.tag).find(held => held.firstHour <= run.firstHour && run.firstHour <= held.lastHour);
    if (holder?.countryId === undefined || holder.unresolved) spans.push(run);
    else joined.set(holder.countryId, Math.min(joined.get(holder.countryId) ?? Infinity, run.firstHour));
  }
  for (const [slot, from] of joined) {
    for (const held of slots.tagRuns(slot)) {
      const firstHour = Math.max(from, held.firstHour), last = Math.min(lastHour, held.lastHour);
      if (firstHour <= last) spans.push({ tag: held.tag, firstHour, lastHour: last });
    }
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
