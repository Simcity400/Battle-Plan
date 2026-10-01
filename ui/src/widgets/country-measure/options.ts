import { END_LABELS_OPTION, LINE_AXIS_OPTIONS, LINE_TIME_OPTIONS } from '../../dashboard/lineChartOptions.ts';
import type { OptionField } from '../../dashboard/options';

/** Settings every balance widget (political power, command power) offers. */
export const MEASURE_OPTIONS: readonly OptionField[] = [
  ...LINE_TIME_OPTIONS,
  { key: 'spendTicks', section: 'options', label: 'Spend ticks', hint: 'Mark each hour the balance fell', kind: 'toggle', default: true },
  { key: 'areaFill', section: 'options', label: 'Area fill', hint: "Shade under a single country's line", kind: 'toggle', default: true },
  END_LABELS_OPTION,
  ...LINE_AXIS_OPTIONS,
];
