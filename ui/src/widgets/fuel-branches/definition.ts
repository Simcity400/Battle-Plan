import type { WidgetMeta } from '../../dashboard/definitions';
import { END_LABELS_OPTION, LINE_AXIS_OPTIONS, TIME_CHART_TIME_OPTIONS } from '../../dashboard/lineChartOptions.ts';
import { choiceVariants } from '../../dashboard/options.ts';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { FUEL_DECIMALS_OPTION } from './fuelBranchesData.ts';

const overTime = (options: Readonly<Record<string, OptionValue>>) => options.view === 'time';

export const OPTIONS: readonly OptionField[] = [
  { key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'bars',
    choices: [{ value: 'bars', label: 'Bars' }, { value: 'time', label: 'Over time' }] },
  FUEL_DECIMALS_OPTION,
  { ...END_LABELS_OPTION, visibleWhen: overTime },
  ...TIME_CHART_TIME_OPTIONS.map(field => ({ ...field, visibleWhen: (options: Readonly<Record<string, OptionValue>>) => overTime(options) && (!field.visibleWhen || field.visibleWhen(options)) })),
  ...LINE_AXIS_OPTIONS.map(field => ({ ...field, visibleWhen: overTime })),
];

export const definition = {
  kind: 'fuel-branches', version: 1, title: 'Fuel by branch', icon: 'droplet', category: 'Economy',
  description: 'Fuel each military branch requested and received, and when shortages began.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 5 }, defaultSize: { w: 6, h: 8 },
  countries: { max: Number.MAX_SAFE_INTEGER }, options: OPTIONS,
  variants: choiceVariants(OPTIONS.find(field => field.key === 'view'), () => 'Fuel by branch over time'),
} as const satisfies WidgetMeta;
