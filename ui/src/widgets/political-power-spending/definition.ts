import type { WidgetMeta } from '../../dashboard/definitions';
import { POLITICS_SPENDING_OPTIONS } from '../../dashboard/politicsSpendingOptions.ts';

export const definition = {
  kind: 'political-power-spending', version: 1, title: 'Political power spending', icon: 'sparkle',
  description: 'Political power payments by date, item and cost, with what was gained.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER }, options: POLITICS_SPENDING_OPTIONS,
} as const satisfies WidgetMeta;
