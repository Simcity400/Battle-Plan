import type { ChartAxisFormatterId, ChartFitId } from '../../charts/renderTimeChart.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { MEASURE_OPTIONS } from '../../widgets/country-measure/options.ts';

export interface TimeChartDeclaration {
  readonly kind: string;
  readonly family: string;
  readonly measure: string;
  readonly title: string;
  readonly noun: string;
  readonly unit: string;
  readonly icon: WidgetMeta['icon'];
  readonly options: readonly OptionField[];
  readonly formatValue: ChartAxisFormatterId;
  readonly fitValues?: ChartFitId;
  readonly spendTicks?: boolean;
  readonly held?: boolean;
  readonly balanceChanges?: boolean;
}

export const POLITICAL_POWER_DECLARATION: TimeChartDeclaration = {
  kind: 'political-power',
  family: 'country-measure',
  measure: 'politicalPower',
  title: 'Political power',
  noun: 'political power',
  unit: 'PP',
  icon: 'sparkle',
  options: MEASURE_OPTIONS,
  formatValue: 'powerAxis',
  spendTicks: true,
  held: false,
  balanceChanges: true,
};
