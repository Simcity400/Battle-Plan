import { breakdownOption, DRILL_OPTION } from '../drill/drillState.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { OptionField } from '../../dashboard/options';

/** A faction's members, the breakdown a faction's bar or line opens, and a view a widget can be pinned to. */
export const FACTION_MEMBERS = { id: 'members', label: 'Members' };

export const factionMemberOptions: readonly OptionField[] = [
  breakdownOption([FACTION_MEMBERS], 'Faction totals'),
  DRILL_OPTION,
];

/** Whether the countries are factions: each faction's total, or one faction's as one total. */
export function factionMemberChoice(countries?: CountrySelection): boolean {
  return countries?.kind === 'factions' || (countries?.kind === 'group' && countries.aggregate && countries.group.type === 'faction');
}
