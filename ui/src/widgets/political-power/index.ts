import { createElement } from 'react';
import type { WidgetProps, WidgetRenderer } from '../../dashboard/definitions';
import CountryChip from '../../dashboard/CountryChip';
import { POLITICAL_POWER_DECLARATION } from '../../families/time-chart/declaration.ts';
import TimeChartHost from '../../families/time-chart/TimeChartHost.tsx';

export default {
  countryChip: CountryChip,
  render: (props: WidgetProps) => createElement(TimeChartHost, { ...props, declaration: POLITICAL_POWER_DECLARATION }),
} satisfies WidgetRenderer;
