import { partsRaceOptions } from '../../charts/race/partsRaceOptions.ts';
import { TOP_COUNTRIES, raceViewVariants } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'planes',
    choices: [{ value: 'planes', label: 'Number of planes' }, { value: 'wings', label: 'Number of wings' }] },
  { key: 'ownership', section: 'options', label: 'Count for', hint: 'A wing can fight for another country while remaining on its owner’s roster',
    kind: 'choice', default: 'roster', choices: [{ value: 'roster', label: 'Roster owner' }, { value: 'effective', label: 'Country it fights for' }] },
  ...partsRaceOptions('type'),
];

export const definition = {
  kind: 'air-force-size', version: 1, title: 'Air force size', icon: 'layers',
  description: 'Compare aircraft in countries’ air wings over time and by type.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true },
  defaultCountries: TOP_COUNTRIES, options: OPTIONS,
  variants: [...raceViewVariants('Air force size', OPTIONS)],
} as const satisfies WidgetMeta;
