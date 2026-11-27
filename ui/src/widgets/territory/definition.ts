import { RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, raceViewOption, TOP_COUNTRIES, raceViewVariants } from '../../charts/race/options.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { MEASURES, RULES } from '../../dashboard/territoryMeasure.ts';

const linesOnly = (option: OptionField): OptionField => ({ ...option,
  visibleWhen: options => options.view === 'lines' && (option.visibleWhen?.(options) ?? true) });
export const OPTIONS: readonly OptionField[] = [
  raceViewOption(undefined, 'race', [{ value: 'share', label: 'Share' }]),
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'states', choices: MEASURES },
  { key: 'rule', section: 'options', label: 'Territory rule', kind: 'choice', default: 'controlled',
    hint: 'Occupied counts what a country controls that another country owns, allies’ and subjects’ land included.', choices: RULES },
  { key: 'endLabels', section: 'options', label: 'Names at line ends', kind: 'toggle', default: true, visibleWhen: options => options.view === 'lines' },
  ...RACE_TIME_OPTIONS.map(linesOnly),
  ...RACE_AXIS_OPTIONS.map(linesOnly),
];

export const definition = {
  kind: 'territory', version: 1, title: 'Territory', icon: 'map',
  description: 'How much of the world each country holds: a race, lines over time or a world share.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 12 },
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, aggregate: true, factions: true }, defaultCountries: TOP_COUNTRIES,
  factionColours: true,
  options: OPTIONS,
  variants: [...raceViewVariants('Territory', OPTIONS),
    { key: 'rule=occupied', initial: { title: 'Occupied territory', options: { rule: 'occupied' } } },
    { key: 'rule=occupied,view=lines', initial: { title: 'Occupied territory over time', options: { rule: 'occupied', view: 'lines' } } }],
} as const satisfies WidgetMeta;
