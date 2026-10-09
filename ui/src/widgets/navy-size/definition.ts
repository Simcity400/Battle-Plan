import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';
import { TOP_COUNTRIES } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'ships',
    choices: [{ value: 'ships', label: 'Number of ships' }, { value: 'strength', label: 'Current hull strength (HP)' }] },
  { key: 'ownership', section: 'options', label: 'Count for', hint: 'Use the task force’s recorded roster owner or logical country; expeditionary ownership identifies the sender',
    kind: 'choice', default: 'roster', choices: [{ value: 'roster', label: 'Roster owner' }, { value: 'logical', label: 'Logical country' }] },
  { key: 'grouping', section: 'options', label: 'Group by', hint: 'Classes and broad roles come from the campaign’s own sub-unit definitions',
    kind: 'choice', default: 'class', choices: [{ value: 'class', label: 'Authored ship class' }, { value: 'type', label: 'Broad role' }] },
  ...partsRaceOptions('class'),
];

export const definition = {
  kind: 'navy-size', version: 1, title: 'Navy size', icon: 'flows',
  description: 'Ships per country: a race, lines over time or one country by content ship class.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES, options: OPTIONS,
} as const satisfies WidgetMeta;
