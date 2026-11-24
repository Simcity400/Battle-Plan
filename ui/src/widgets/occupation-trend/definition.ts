import { RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, raceViewOption, raceViewVariants } from '../../charts/race/options.ts';
import { END_LABELS_OPTION } from '../../dashboard/lineChartOptions.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';

export const OPTIONS: readonly OptionField[] = [
  raceViewOption('measure', 'lines'),
  { key: 'measure', section: 'options', label: 'Measure', kind: 'choice', default: 'resistance',
    choices: [{ value: 'resistance', label: 'Resistance' }, { value: 'compliance', label: 'Compliance' }, { value: 'both', label: 'Both' }] },
  { key: 'rankingMeasure', section: 'options', label: 'Rank both measures by', kind: 'choice', default: 'resistance',
    choices: [{ value: 'resistance', label: 'Resistance' }, { value: 'compliance', label: 'Compliance' }],
    visibleWhen: options => options.measure === 'both' },
  { key: 'collaboration', section: 'options', label: 'Show collaboration', kind: 'toggle', default: false },
  { key: 'digits', section: 'options', label: 'Decimals in chart labels', kind: 'number', min: 0, max: 5, step: 1, default: 2 },
  { ...END_LABELS_OPTION, visibleWhen: options => options.view !== 'race' },
  ...RACE_TIME_OPTIONS, ...RACE_AXIS_OPTIONS,
];

export const definition = {
  kind: 'occupation-trend', version: 1, title: 'Occupation trend', icon: 'globe',
  description: 'Average resistance and compliance in every occupier’s held states over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  countries: { max: Number.MAX_SAFE_INTEGER, ranked: true, all: true },
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 }, options: OPTIONS,
  variants: raceViewVariants('Occupation trend', OPTIONS, { mix: 'Occupation trend of one country' }),
} as const satisfies WidgetMeta;
