import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { Run } from '../../charts/series';
import type { FactionManpowerMember, FactionManpowerRun } from '../../generated/dashboard';

/** Known values only; a stretch not recorded is a gap, never zero. */
export const manpowerLine = (runs: readonly FactionManpowerRun[]): Run[] => runs.map((run, index) => ({ firstHour: run.firstHour, lastHour: run.lastHour,
  value: run.value, ...(run.startsSegment || index === 0 ? { breakBefore: true } : {}) }));

/** The holder a tag named at `hour`, the hour it was picked: the occurrence of that tag with a known contribution
 * then, and every occurrence of that holder (matched by its slot and country id). It depends only on the hour picked,
 * so playback and a later holder of the tag never change it; an occurrence whose slot is not known never joins. */
export function chosenMembers(members: readonly FactionManpowerMember[], tag: string, hour: number): FactionManpowerMember[] {
  const identity = (member: FactionManpowerMember) => member.slot === null ? null : `${member.slot}:${member.countryId}`;
  const slot = countryFromKey(tag);
  const holders = new Set(members.filter(member => (typeof slot === 'number' ? member.slot === slot : member.tag === tag) && identity(member) !== null
    && member.runs.some(run => run.firstHour <= hour && hour <= run.lastHour)).map(identity));
  return members.filter(member => identity(member) !== null && holders.has(identity(member)));
}
