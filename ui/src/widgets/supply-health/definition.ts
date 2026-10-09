import { breakdownOption, DRILL_OPTION } from '../../charts/drill/drillState.ts';
import type { WidgetMeta } from '../../dashboard/definitions';
import { LINE_TIME_OPTIONS } from '../../dashboard/lineChartOptions.ts';

export const definition = {
  kind: 'supply-health', version: 1, title: 'Supply health', icon: 'shield',
  description: 'The share of each country\u2019s divisions in supply, strained or without supply over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 6, h: 10 }, countries: { max: Number.MAX_SAFE_INTEGER },
  options: [breakdownOption([{ id: 'breakdown', label: 'Supply classification' }], 'Supply health'), DRILL_OPTION,
    { key: 'drillOriginHour', section: 'options', label: 'Recorded country selection hour', kind: 'number',
      default: null, min: -2147483648, max: 2147483647, step: 1, visibleWhen: () => false }, ...LINE_TIME_OPTIONS],
} as const satisfies WidgetMeta;
