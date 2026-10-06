import type { WidgetMeta } from '../../dashboard/definitions';
import { MEASURE_OPTIONS } from '../country-measure/options.ts';

export const definition = {
  kind: 'command-power', version: 1, title: 'Command power', icon: 'target',
  description: 'Each country’s command power balance over time, with the days it was spent.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'campaign', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER },
  options: [...MEASURE_OPTIONS,
    { key: 'ceiling', section: 'options', label: 'Maximum', hint: 'Each country’s maximum command power, dashed', kind: 'toggle', default: true }],
} as const satisfies WidgetMeta;
