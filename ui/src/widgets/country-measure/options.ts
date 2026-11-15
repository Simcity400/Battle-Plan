import { END_LABELS_OPTION, LINE_AXIS_OPTIONS, LINE_TIME_OPTIONS } from '../../dashboard/lineChartOptions.ts';
import type { OptionField } from '../../dashboard/options';
import { POWER_DECIMALS_OPTION } from '../../dashboard/powerDecimals.ts';

/** Settings every balance widget (political power, command power) offers. */
export const MEASURE_OPTIONS: readonly OptionField[] = [
  ...LINE_TIME_OPTIONS,
  { key: 'spendTicks', section: 'options', label: 'Spending markers', kind: 'toggle', default: true },
  { key: 'areaFill', section: 'options', label: 'Area fill', hint: "Shade under a single country's line", kind: 'toggle', default: true },
  END_LABELS_OPTION,
  POWER_DECIMALS_OPTION,
  ...LINE_AXIS_OPTIONS,
];
