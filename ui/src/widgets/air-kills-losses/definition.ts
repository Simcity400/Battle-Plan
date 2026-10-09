import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { END_LABELS_OPTION } from '../../dashboard/lineChartOptions.ts';
import { DATE_OVERLAY_OPTION, RACE_AXIS_OPTIONS, RACE_TIME_OPTIONS, TOP_COUNTRIES } from '../../charts/race/options.ts';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';

export const OPTIONS: readonly OptionField[] = [
  breakdownOption([{ id: 'breakdown', label: 'Loss causes' }], 'Chart'),
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'race',
    choices: [{ value: 'race', label: 'Race' }, { value: 'lines', label: 'Lines over time' }] },
  { key: 'measure', section: 'options', label: 'Rank by', kind: 'choice', default: 'taken',
    choices: [{ value: 'taken', label: 'Own losses booked' }, { value: 'dealt', label: 'Enemy losses credited' }] },
  { key: 'regions', section: 'options', label: 'Air regions', kind: 'content-set', default: '', automatic: 'Every recorded region' },
  { key: 'causes', section: 'options', label: 'Loss causes', kind: 'content-set', default: '', automatic: 'Every recorded cause' },
  { key: 'ratio', section: 'options', label: 'Ratio beside race values', kind: 'toggle', default: true, visibleWhen: options => options.view === 'race' },
  DATE_OVERLAY_OPTION, { ...END_LABELS_OPTION, visibleWhen: options => options.view === 'lines' }, ...RACE_TIME_OPTIONS, ...RACE_AXIS_OPTIONS, DRILL_OPTION,
];

export const definition = {
  kind: 'air-kills-losses', version: 1, title: 'Air kills & losses', icon: 'target',
  description: 'Native aircraft loss bookkeeping and enemy credit, with operational losses and exact signed corrections.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER, ranked: true }, defaultCountries: TOP_COUNTRIES,
  availability: { product: 'military-air-ledger', params: { family: 'airLosses' } }, options: OPTIONS,
} as const satisfies WidgetMeta;
