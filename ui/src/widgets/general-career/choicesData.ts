import { leaderName } from '../generals/generalsData.ts';
import type { LeaderChoice } from '../generals/types';
import { subjectToken } from '../../dashboard/chosen.ts';
import type { LeaderChoicesReply } from '../../generated/dashboard';

export function validateLeaderChoicesReply(input: unknown): LeaderChoicesReply {
  const bad = (): never => { throw new Error('The leader choices reply is malformed.'); };
  if (typeof input !== 'object' || input === null) return bad();
  const choices = (input as LeaderChoicesReply).choices;
  if (!Array.isArray(choices) || !choices.every(choice => Array.isArray(choice) && choice.length === 4 && typeof choice[0] === 'string' && typeof choice[3] === 'number')) return bad();
  return input as LeaderChoicesReply;
}

export function leaderChoices(choices: readonly LeaderChoice[], campaignKey: string, countryName: (hour: number, tag: string) => { name: string; nameDetail?: string }) {
  return choices.map(([reference, recorded, owner, lastHour]) => {
    const name = leaderName({ name: recorded });
    const country = owner === null ? undefined : countryName(lastHour, owner);
    return { ...(country?.nameDetail ? { detail: country.nameDetail } : {}), value: subjectToken(campaignKey, reference), name, label: `${name} · ${country?.name ?? 'Owner not recorded'}` };
  }).sort((a, b) => a.name.localeCompare(b.name) || a.value.localeCompare(b.value)).map(({ name: _name, ...choice }) => choice);
}
