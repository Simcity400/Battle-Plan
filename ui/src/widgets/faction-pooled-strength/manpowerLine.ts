import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { Run } from '../../charts/series';
import type { FactionManpowerMember, FactionManpowerRun } from '../../generated/dashboard';

export const manpowerLine = (runs: readonly FactionManpowerRun[]): Run[] => runs.map((run, index) => ({ firstHour: run.firstHour, lastHour: run.lastHour,
  value: run.value, ...(run.startsSegment || index === 0 ? { breakBefore: true } : {}) }));

export function chosenMembers(members: readonly FactionManpowerMember[], tag: string, hour: number): FactionManpowerMember[] {
  const identity = (member: FactionManpowerMember) => member.slot === null ? null : `${member.slot}:${member.countryId}`;
  const slot = countryFromKey(tag);
  const holders = new Set(members.filter(member => (typeof slot === 'number' ? member.slot === slot : member.tag === tag) && identity(member) !== null
    && member.runs.some(run => run.firstHour <= hour && hour <= run.lastHour)).map(identity));
  return members.filter(member => identity(member) !== null && holders.has(identity(member)));
}
