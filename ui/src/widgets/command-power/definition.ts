import type { WidgetMeta } from '../../dashboard/definitions';
import type { OptionField } from '../../dashboard/options';
import { MEASURE_OPTIONS } from '../country-measure/options.ts';

export const OPTIONS: readonly OptionField[] = [...MEASURE_OPTIONS,
  { key: 'ceiling', section: 'options', label: 'Maximum', kind: 'toggle', default: true }];

export const definition = {
  kind: 'command-power', version: 1, title: 'Command power', icon: 'chart-line',
  description: 'Each country’s command power balance over time, with the days it was spent.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 },
  countries: { max: Number.MAX_SAFE_INTEGER },
  options: OPTIONS,
} as const satisfies WidgetMeta;
