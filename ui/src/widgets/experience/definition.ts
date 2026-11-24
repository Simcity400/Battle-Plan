import type { WidgetMeta } from '../../dashboard/definitions';

export const definition = {
  kind: 'experience', version: 1, title: 'Experience', icon: 'chart-line',
  description: 'Army, navy and air experience banked over time.',
  timeModes: ['campaign', 'follow', 'pin', 'range'], defaultTime: 'follow', scope: 'history', data: true, history: true,
  minSize: { w: 4, h: 6 }, defaultSize: { w: 8, h: 10 },
  countries: { max: 1 },
} as const satisfies WidgetMeta;
