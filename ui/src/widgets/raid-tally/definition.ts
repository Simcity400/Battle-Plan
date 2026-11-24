import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { RAIDS_AVAILABILITY } from '../../dashboard/raids.ts';
import { RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, TOP_COUNTRIES, raceViewVariants } from '../../charts/race/options.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'race',
    choices: [{ value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines' }] },
  { key: 'metric', section: 'options', label: 'Count', kind: 'choice', default: 'both',
    choices: [{ value: 'both', label: 'Both' }, { value: 'launched', label: 'Launched' }, { value: 'succeeded', label: 'Succeeded' }] },
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true, visibleWhen: options => options.view === 'lines' },
  ...RACE_TIME_OPTIONS, ...RACE_AXIS_OPTIONS,
];
export const definition = {
  kind: 'raid-tally', version: 1, title: 'Raid tally', icon: 'target',
  description: 'Raid launches and successes by country.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES, availability: RAIDS_AVAILABILITY, options: OPTIONS,
  variants: raceViewVariants('Raid tally', OPTIONS),
} as const satisfies WidgetMeta;
