import type { WidgetMeta } from '../../dashboard/definitions';
import { POWER_SPENDING_OPTIONS } from '../../dashboard/powerSpendingOptions.ts';

export const definition = {
  kind: 'command-power-spending', version: 1, title: 'Command power spending', icon: 'target',
  description: 'Command power payments by date, item and cost, with what was gained.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER }, options: POWER_SPENDING_OPTIONS,
} as const satisfies WidgetMeta;
