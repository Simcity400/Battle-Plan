import { END_LABELS_OPTION, LINE_AXIS_OPTIONS, LINE_TIME_OPTIONS } from '../../dashboard/lineChartOptions.ts';
import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import type { Breakdown } from '../../charts/drill/drillState';
import type { OptionField } from '../../dashboard/options';

export const POWER_BREAKDOWNS: readonly Breakdown[] = [
  { id: 'income', label: 'Daily income and costs' },
  { id: 'spending', label: 'Spending that day' },
];

/** Settings every balance widget (political power, command power) offers. */
export const MEASURE_OPTIONS: readonly OptionField[] = [
  ...LINE_TIME_OPTIONS,
  { key: 'spendTicks', section: 'options', label: 'Spend ticks', hint: 'Mark recorded payments; net balance falls when capture is unavailable', kind: 'toggle', default: true },
  { key: 'areaFill', section: 'options', label: 'Area fill', hint: "Shade under a single country's line", kind: 'toggle', default: true },
  END_LABELS_OPTION,
  ...LINE_AXIS_OPTIONS,
  breakdownOption(POWER_BREAKDOWNS, 'Balance'),
  DRILL_OPTION,
];
