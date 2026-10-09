import type { WidgetMeta } from '../../dashboard/definitions';
import { POLITICS_SPENDING_OPTIONS } from '../../dashboard/politicsSpendingOptions.ts';

export const definition = {
  kind: 'command-power-spending', version: 1, title: 'Command power spending', icon: 'target',
  description: 'Recorded command power costs and daily deficits, with current reservations shown separately.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 7, h: 14 }, countries: { max: Number.MAX_SAFE_INTEGER }, options: POLITICS_SPENDING_OPTIONS,
} as const satisfies WidgetMeta;
