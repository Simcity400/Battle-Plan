import { leaderName } from '../generals/generalsData.ts';
import type { LeaderChoice } from '../generals/types';
import { subjectToken } from '../../dashboard/chosen.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { LeaderChoicesReply, LeaderSelection } from '../../generated/dashboard';
import { leaderSelectionOf } from './dossierProjection.ts';

export function validateLeaderChoicesReply(input: unknown): LeaderChoicesReply {
  const bad = (): never => { throw new Error('The leader choices reply is malformed.'); };
  if (typeof input !== 'object' || input === null) return bad();
  const choices = (input as LeaderChoicesReply).choices;
  if (!Array.isArray(choices) || !choices.every(choice => Array.isArray(choice) && choice.length === 5 && typeof choice[0] === 'string' && typeof choice[3] === 'number')) return bad();
  return input as LeaderChoicesReply;
}

export function leaderChoices(choices: readonly LeaderChoice[], campaignKey: string, countryName: (hour: number, tag: string) => { name: string; nameDetail?: string }) {
  return choices.map(([reference, recorded, owner, lastHour, membership]) => {
    const name = leaderName({ name: recorded });
    const country = owner === null ? undefined : countryName(lastHour, owner);
    const detail = [country?.nameDetail, membership].filter(Boolean).join('\n');
    const label = `${name} · ${country?.name ?? 'Owner not recorded'}${membership === null ? '' : ' · Membership not recorded'}`;
    return { ...(detail ? { detail } : {}), value: subjectToken(campaignKey, reference), name, label };
  }).sort((a, b) => a.name.localeCompare(b.name) || a.value.localeCompare(b.value)).map(({ name: _name, ...choice }) => choice);
}

export function choicesSelection(countries: CountrySelection, selected: readonly string[]): LeaderSelection | null {
  return leaderSelectionOf(countries, selected) ?? (countries.kind === 'group' ? { kind: 'all' } : null);
}
