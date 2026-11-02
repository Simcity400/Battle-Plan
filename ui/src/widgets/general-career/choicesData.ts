import { leaderName } from '../generals/generalsData.ts';
import type { UnitLeadersData } from '../generals/generalsData';
import { subjectToken } from '../../dashboard/chosen.ts';

export function leaderChoices(data: UnitLeadersData, campaignKey: string, countryName: (hour: number, tag: string) => { name: string; nameDetail?: string }) {
  return data.leaders.map(leader => {
    const run = leader.runs.at(-1); const person = run && data.people[run.value.person];
    const name = person ? leaderName({ person }) : `Leader ${leader.reference}`;
    const owner = run && data.roles[run.value.role].owner;
    const country = owner && run ? countryName(run.lastHour, owner) : undefined;
    return { ...(country?.nameDetail ? { detail: country.nameDetail } : {}), value: subjectToken(campaignKey, leader.reference), name, label: `${name} · ${country?.name ?? 'Owner not recorded'}` };
  }).sort((a, b) => a.name.localeCompare(b.name) || a.value.localeCompare(b.value)).map(({ name: _name, ...choice }) => choice);
}
