import type { Run } from '../../charts/series';
import type { FactionManpowerMember, FactionManpowerRun } from '../../generated/dashboard';

/** Known values only; an unknown stretch is a gap, never zero. */
export const manpowerLine = (runs: readonly FactionManpowerRun[]): Run[] => runs.flatMap((run, index) => run.value === null ? []
  : [{ firstHour: run.firstHour, lastHour: run.lastHour, value: run.value, ...(run.startsSegment || index === 0 || runs[index - 1].value === null ? { breakBefore: true } : {}) }]);

/** The native holder a tag named at `hour`, the hour it was picked: the member of that tag with a
 * known contribution then (an earlier holder's unknown tail is not proof it still held the tag),
 * and every occurrence of that holder, matched by its slot and country id. The result depends
 * only on the hour picked, so playback and a later holder of the tag never change it; a member
 * whose holder is not known never joins. */
export function chosenMembers(members: readonly FactionManpowerMember[], tag: string, hour: number): FactionManpowerMember[] {
  const identity = (member: FactionManpowerMember) => member.slot === null || member.countryId === null ? null : `${member.slot}:${member.countryId}`;
  const holders = new Set(members.filter(member => member.tag === tag && identity(member) !== null
    && member.runs.some(run => run.value !== null && run.firstHour <= hour && hour <= run.lastHour)).map(identity));
  return members.filter(member => identity(member) !== null && holders.has(identity(member)));
}
