import { TOP_COUNTRIES, raceViewVariants } from '../../charts/race/options.ts';
import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';
import { breakdownOption, breakdownVariants } from '../../charts/drill/drillState.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = partsRaceOptions('nuke type').map(field => field.key === 'show'
  ? breakdownOption([{ id: 'breakdown', label: 'Weapon types' }], 'Country totals')
  : field.key === 'split' ? { ...field, label: 'Show weapon types', hint: 'Split each stockpile into its weapon types.', default: true } as OptionField : field);

export const definition = {
  kind: 'nuclear-arsenal', version: 3, title: 'Nuclear arsenal', icon: 'target',
  description: 'Nuclear stockpiles by country and weapon type, as the top bar shows them.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true }, defaultCountries: TOP_COUNTRIES, options: OPTIONS,
  variants: [...raceViewVariants('Nuclear arsenal', OPTIONS), ...breakdownVariants('Nuclear arsenal', OPTIONS)],
} as const satisfies WidgetMeta;
