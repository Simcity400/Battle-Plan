import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, raceViewOption, TOP_COUNTRIES } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const OPTIONS: readonly OptionField[] = [
  { ...raceViewOption('nuke type'), visibleWhen: options => options.show === 'main' },
  breakdownOption([{ id: 'breakdown', label: 'Nuke types' }], 'Country totals'),
  { key: 'drillOriginHour', section: 'options', label: 'Country selected at hour', kind: 'number',
    min: -2147483648, max: 2147483647, step: 1, default: null, visibleWhen: () => false },
  { ...DATE_OVERLAY_OPTION, visibleWhen: options => options.show === 'main' && options.view === 'race' },
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true,
    visibleWhen: options => options.show === 'main' && options.view === 'lines' },
  ...[...RACE_TIME_OPTIONS, ...RACE_AXIS_OPTIONS].map(field => ({ ...field,
    visibleWhen: (options, countries) => field.visibleWhen?.({ ...options, view: options.show === 'breakdown' ? 'mix' : options.view }, countries) ?? true } satisfies OptionField)),
  { ...DRILL_OPTION, visibleWhen: options => options.show === 'main' },
];

export const definition = {
  kind: 'nuclear-arsenal', version: 1, title: 'Nuclear arsenal', icon: 'target',
  description: 'The game’s nuclear stockpile counter for every country, split by its content’s nuke types.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true }, defaultCountries: TOP_COUNTRIES, options: OPTIONS,
} as const satisfies WidgetMeta;
