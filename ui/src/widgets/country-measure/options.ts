import { END_LABELS_OPTION, LINE_AXIS_OPTIONS, TIME_CHART_TIME_OPTIONS } from '../../dashboard/lineChartOptions.ts';
import type { OptionField } from '../../dashboard/options';
import { POWER_DECIMALS_OPTION } from '../../dashboard/powerDecimals.ts';

export const MEASURE_OPTIONS: readonly OptionField[] = [
  ...TIME_CHART_TIME_OPTIONS,
  { key: 'spendTicks', section: 'options', label: 'Spending markers', kind: 'toggle', default: true },
  { key: 'areaFill', section: 'options', label: 'Area fill', hint: "Shade under a single country's line", kind: 'toggle', default: true },
  END_LABELS_OPTION,
  POWER_DECIMALS_OPTION,
  ...LINE_AXIS_OPTIONS,
];
