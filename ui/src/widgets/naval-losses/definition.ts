import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { TOP_COUNTRIES, raceViewVariants } from '../../charts/race/options.ts';
import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';

export const OPTIONS: readonly OptionField[] = [
  { key: 'kind', section: 'options', label: 'Count', kind: 'choice', default: 'both',
    choices: [{ value: 'ships', label: 'Ships' }, { value: 'convoys', label: 'Convoys' }, { value: 'both', label: 'Ships and convoys' }] },
  { key: 'attribution', section: 'options', label: 'Count for', kind: 'choice', default: 'lost',
    choices: [{ value: 'lost', label: 'Lost' }, { value: 'sunkBy', label: 'Sunk by' }] },
  { key: 'period', section: 'options', label: 'Period', kind: 'choice', default: 'running',
    choices: [{ value: 'running', label: 'Running total' }, { value: 'month', label: 'Per month' }] },
  ...partsRaceOptions('kind'),
];

export const definition = {
  kind: 'naval-losses', version: 1, title: 'Naval losses', icon: 'shield',
  description: 'Ships and convoys lost or sunk by each country, as a race or lines over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES, options: OPTIONS,
  variants: [...raceViewVariants('Naval losses', OPTIONS)],
} as const satisfies WidgetMeta;
