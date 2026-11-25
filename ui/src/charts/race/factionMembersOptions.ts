import { breakdownOption, DRILL_OPTION } from '../drill/drillState.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { OptionField } from '../../dashboard/options';

const FACTION_MEMBERS = { id: 'members', label: 'Members' };

export const factionMemberOptions: readonly OptionField[] = [
  breakdownOption([FACTION_MEMBERS], 'Faction totals'),
  DRILL_OPTION,
];

export function factionMemberChoice(countries?: CountrySelection): boolean {
  return countries?.kind === 'factions' || (countries?.kind === 'group' && countries.aggregate && countries.group.type === 'faction');
}
