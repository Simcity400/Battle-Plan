import type { WidgetMeta } from '../../dashboard/definitions';
import { MEASURE_OPTIONS } from '../country-measure/options.ts';

export const definition = {
  kind: 'political-power', version: 1, title: 'Political power', icon: 'chart-line',
  description: 'Each country’s political power balance over time, with the days it was spent.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER },
  options: MEASURE_OPTIONS,
} as const satisfies WidgetMeta;
