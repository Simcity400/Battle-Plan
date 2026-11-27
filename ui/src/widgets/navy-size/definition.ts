import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';
import { TOP_COUNTRIES, raceViewVariants } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'ships',
    choices: [{ value: 'ships', label: 'Number of ships' }, { value: 'strength', label: 'Current hull strength (HP)' }] },
  { key: 'ownership', section: 'options', label: 'Count fleets for',
    kind: 'choice', default: 'roster', choices: [{ value: 'roster', label: 'Country holding the fleet' }, { value: 'logical', label: 'Assigned country' }] },
  { key: 'grouping', section: 'options', label: 'Group by',
    kind: 'choice', default: 'class', choices: [{ value: 'class', label: 'Ship class' }, { value: 'type', label: 'Role' }] },
  ...partsRaceOptions('class'),
];

export const definition = {
  kind: 'navy-size', version: 1, title: 'Navy size', icon: 'flows',
  description: 'Ships by country and class over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES, options: OPTIONS,
  variants: [...raceViewVariants('Navy size', OPTIONS)],
} as const satisfies WidgetMeta;
