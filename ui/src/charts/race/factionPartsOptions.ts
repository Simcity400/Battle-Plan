import { breakdownOption, DRILL_OPTION } from '../drill/drillState.ts';
import type { CountrySelection } from '../../dashboard/model';
import type { OptionField } from '../../dashboard/options';

export const factionMemberOptions: readonly OptionField[] = [
  breakdownOption([{ id: 'members', label: 'Members' }], 'Faction totals'),
  DRILL_OPTION,
  { key: 'memberFaction', section: 'options', label: 'Saved faction', kind: 'content-choice', default: '',
    automatic: 'Chosen faction', visibleWhen: () => false },
];

export function factionMemberChoice(countries?: CountrySelection): boolean {
  return countries?.kind === 'factions' || (countries?.kind === 'group' && countries.aggregate && countries.group.type === 'faction');
}
