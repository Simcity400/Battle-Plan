import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const TRANSFER_BREAKDOWN = { id: 'parts', label: 'Transfer breakdown' };

export const OPTIONS: readonly OptionField[] = [
  breakdownOption([TRANSFER_BREAKDOWN], 'Transfer diagram'),
  { key: 'selection', section: 'options', label: 'Transfer selection', kind: 'content-choice', automatic: 'All transfers', default: '', visibleWhen: () => false },
  DRILL_OPTION,
  { key: 'measure', section: 'options', label: 'Size', kind: 'choice', default: 'states',
    choices: [{ value: 'states', label: 'State count' }, { value: 'victoryPoints', label: 'Victory points' }] },
  { key: 'scope', section: 'options', label: 'Scope', kind: 'choice', default: 'campaign',
    choices: [{ value: 'campaign', label: 'Whole campaign' }, { value: 'conference', label: 'One conference' }] },
  { key: 'conference', section: 'options', label: 'Conference', kind: 'content-choice', automatic: 'Newest conference shown', default: '',
    visibleWhen: options => options.scope === 'conference' },
  { key: 'colour', section: 'options', label: 'Ribbon colour', kind: 'choice', default: 'giver',
    choices: [{ value: 'giver', label: 'By giver country' }, { value: 'taker', label: 'By taker country' }] },
];

export const definition = {
  kind: 'territory-transfers', version: 1, title: 'Territory transfers', category: 'Diplomacy', icon: 'flows',
  description: 'States transferred in peace settlements.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  countrySelection: false,
  countries: { max: Number.MAX_SAFE_INTEGER },
  defaultCountries: { kind: 'all' }, minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 12 }, options: OPTIONS,
} as const satisfies WidgetMeta;
