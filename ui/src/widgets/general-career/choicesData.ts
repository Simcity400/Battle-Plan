import { leaderName } from '../generals/generalsData.ts';
import type { UnitLeadersData } from '../generals/generalsData';

export function leaderChoices(data: UnitLeadersData, countryName: (hour: number, tag: string) => string) {
  return data.leaders.map(leader => {
    const run = leader.runs.at(-1); const person = run && data.people[run.value.person];
    const name = person ? leaderName({ person }) : `Leader ${leader.reference}`;
    const owner = run && data.roles[run.value.role].owner;
    return { value: leader.reference, name, label: `${name} · ${owner && run ? countryName(run.lastHour, owner) : 'Owner not recorded'}` };
  }).sort((a, b) => a.name.localeCompare(b.name) || a.value.localeCompare(b.value)).map(({ value, label }) => ({ value, label }));
}
