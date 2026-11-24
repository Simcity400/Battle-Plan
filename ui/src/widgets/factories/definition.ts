import { TOP_COUNTRIES, raceViewVariants } from '../../charts/race/options.ts';
import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';
import { breakdownOption, breakdownVariants } from '../../charts/drill/drillState.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { COUNT_MODES } from './factoriesData.ts';

export const OPTIONS: readonly OptionField[] = [
  ...partsRaceOptions('type').map(field => field.key === 'show'
    ? breakdownOption([{ id: 'breakdown', label: 'Factory types' }], 'Factories') : field),
  { key: 'count', section: 'options', label: 'Count', hint: 'Available after damage and other commitments.', kind: 'choice', default: 'free',
    choices: COUNT_MODES.map(mode => ({ value: mode.value, label: mode.label })) },
  { key: 'types', section: 'options', label: 'Factory types', kind: 'content-set', automatic: 'All types', default: '' },
];

export const definition = {
  kind: 'factories', version: 3, title: 'Factories', icon: 'factory',
  description: 'Factories by country and type, counted as the game counts them.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES,
  options: OPTIONS,
  migrate: (_from, { options = {}, ...widget }) => ({ ...widget,
    options: { ...options, view: ['race', 'lines', 'mix'].includes(String(options.view)) ? options.view : 'race' } }),
  variants: [...raceViewVariants('Factories', OPTIONS), ...breakdownVariants('Factories', OPTIONS)],
  playsBackDated: true,
} as const satisfies WidgetMeta;
